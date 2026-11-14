# Direct Kernel QEMU probes

The startup fixture boots as Kernel's initial Task. The std fixtures run in
opted-in User Teams started by a test-only launcher. These are direct Kernel
probes. The assembled System Loader path is tracked in Vanllia.

Both probes are opt-in. Supply an explicit OpenSBI firmware and its expected
SHA-256. The suite never downloads or substitutes these inputs.

## ABI v11 startup

Run Toolchain's `build` command. Build the normal `shuyra` ELF from Vanllia's
`platform/riscv` at `dd32c35dbb67e2e78edb00ad35f968d16423249e` (the
platform source used by Gamma's native acceptance), with both `kernel` and
`kernel-api` resolved
to the startup fixture's Kernel tag `26.11.14-preview-Gamma`. In a dedicated
integration checkout, replace the platform's historical `rev` selection and
remove `features = ["memory-v11"]` from both dependencies. Refresh its lockfile
before building the image:

```sh
cargo +nightly update --manifest-path platform/riscv/Cargo.toml -p kernel
cargo +nightly build --locked --release --target riscv64gc-unknown-none-elf \
  --manifest-path platform/riscv/Cargo.toml
```

Use the resulting normal platform ELF with `build/bootstrap.json`:

```sh
SHUYRA_TOOLCHAIN_RECEIPT=/absolute/toolchain/build/bootstrap.json \
SHUYRA_KERNEL_ELF=/absolute/vanllia/platform/riscv/target/riscv64gc-unknown-none-elf/release/shuyra \
SHUYRA_KERNEL_REVISION=9a35dd5420474ebe75a6b5f3e4589a398745e32c \
SHUYRA_OPENSBI=/absolute/fw_jump.bin \
SHUYRA_FIRMWARE_SHA256=<sha256-of-the-selected-firmware> \
cargo +nightly test --locked --test qemu startup_abi_v11 -- --ignored --nocapture
```

This probe validates the Capsule v2/ABI v11 initrd with Kernel API. The Kernel
supplies the initial View and Ledger argument words, which the startup ELF
reads and validates through its pointer/count entry contract before calling
`Unit.Exit` with `ExitScope::Task`. The probe checks the number
of admitted harts, exactly one Kernel exit record for task 1 with reason zero
and note `toolchain: startup passed`, and successful all-tasks-exited shutdown.

## ABI v11 std backend

Run Toolchain's `build-std` command. Gamma's native memory backend requires
trusted RAM qualification and rejects qualification with more than one Hart.
The normal startup image does not install those facts. Use the test-only
[`platform.rs`](platform.rs) entry with the same dedicated Vanllia checkout and
Gamma tag selection described above:

```sh
cp /absolute/toolchain/tests/qemu/platform.rs platform/riscv/src/main.rs
cargo +nightly build --locked --release --target riscv64gc-unknown-none-elf \
  --manifest-path platform/riscv/Cargo.toml
```

Supply this rebuilt ELF for std only. This entry admits only QEMU `virt`, one
Hart, 256 MiB ordinary RAM and no external devices. It qualifies local CPU
visibility, instruction synchronization and retirement; it is not a hardware
qualification or a general platform default. Do not use it for the two-Hart
startup probe. Toolchain has no source dependency on Vanllia or its build library.

The QEMU test builds the no-std launcher with the
`riscv64gc-unknown-none-elf` target. Install it with
`rustup target add --toolchain nightly riscv64gc-unknown-none-elf`.
The backtrace probe requires OpenSBI with the debug console extension (DBCN)
and a host `addr2line` that reads RV64 ELF files.

```sh
SHUYRA_TOOLCHAIN_RECEIPT=/absolute/toolchain/build/std.json \
SHUYRA_KERNEL_ELF=/absolute/vanllia/platform/riscv/target/riscv64gc-unknown-none-elf/release/shuyra \
SHUYRA_KERNEL_REVISION=9a35dd5420474ebe75a6b5f3e4589a398745e32c \
SHUYRA_OPENSBI=/absolute/fw_jump.bin \
SHUYRA_FIRMWARE_SHA256=<sha256-of-the-selected-firmware> \
cargo +nightly test --locked --test qemu std_main_allocator_and_termination -- --ignored --nocapture
```

`SHUYRA_QEMU` selects the native QEMU binary. Each std scenario runs on one
qualified Hart with a 20-second timeout. Startup runs on one and two Harts.
The launcher checks the retained Team and initiating Task results through `Unit.Inspect` and waits for Team retirement.
The host verifies the admitted hart count, the Kernel exit records and
all-tasks-exited shutdown. The pinned ABI v11 Kernel reserves Task 2 for its
memory observer, so the launcher expects broker 3, Rust main 4 and first worker 5.

| Fixture | Exit value | Reason | Kernel note |
| --- | --- | --- | --- |
| `toolchain-std`, `std-threads`, `std-waits`, `std-backtrace` | `0` | `0` | `toolchain: std main returned` |
| `std-parallelism` | Hart count | `0` | `toolchain: std main returned` |
| `std-exit`, `std-thread-exit` | `23` | `0` | `toolchain: std process exit` |
| `std-panic`, `std-oom`, `std-thread-panic`, `std-tls-main`, `std-thread-fault` | `0` | `0xffffff01` | `toolchain: std abort` |

Toolchain owns the programs, assertions and execution. The test checks artifact
hashes against the build receipt and public inspect CLI, encodes the capsule
and validates it with Kernel API’s public parser before observing actual Kernel
termination. There is no Vanllia/build-library dependency. UART/process logs, input hashes,
QEMU version and results are saved under `target/qemu/<run>/`. The declared Kernel revision records the build
input supplied by the caller; its binary hash records what was actually run.
The Toolchain build receipt is left unchanged; execution evidence is separate.

This does not close [Vanllia #19](https://github.com/shuyra-arche/vanllia/issues/19)'s
assembled-System artifact handoff or deployment acceptance.

The thread fixture covers TLS isolation and destructor ordering, cross-task heap
ownership, concurrent allocation/free, join/scoped/detached workers,
synchronization and timeouts. Process-exit
probes require all tasks to halt, including a live detached worker on main return
and the Rust main Task on worker exit or panic. An illegal-instruction worker checks
that join aborts when Kernel ends a task before Rust cleanup. The wait fixture checks Kernel queue counts, comparison mismatch, poll and timed
expiry, invalid addresses, wake-one/broadcast, finite-timeout
overflow, 256 producer/consumer races, std Condvar broadcast, park/unpark and
termination of a blocked detached worker. The backtrace probe resolves the
main and worker call chains from the exact fixture ELF and records both
symbolized traces. The std test covers 12 QEMU runs; startup adds two.
Two-Hart std execution is unsupported by Gamma's native memory provider and is not claimed as verified.

Run the native QEMU executable directly with `SHUYRA_QEMU`; shell wrappers and
external helper processes are outside this small harness's scope. Each owned
child is killed and reaped on timeout or an evidence/write/wait failure. Ordinary
`cargo +nightly test --locked --test qemu` runs the host timeout/nonzero-exit
regression and leaves guest execution ignored until explicitly requested.
