# Direct Kernel QEMU probes

The startup fixture boots as Kernel's initial Task. The std fixtures run in
opted-in User Teams started by a test-only launcher. These are direct Kernel
probes. The assembled System Loader path is tracked in Vanllia.

Both probes are opt-in. Supply an explicit OpenSBI firmware and its expected
SHA-256. The suite never downloads or substitutes these inputs.

## ABI v8 startup

Run Toolchain's `build` command. Build the normal `shuyra` ELF from Vanllia's
`platform/riscv` at `eb37b0b0db299513274864cec23cee509a3cd3cc` (the integration
revision used by this Kernel's CI), with both `kernel` and `kernel-api` resolved
to the startup fixture's Kernel revision. Refresh the platform lockfile before
building the image. Use the resulting ELF with `build/bootstrap.json`:

```sh
SHUYRA_TOOLCHAIN_RECEIPT=/absolute/toolchain/build/bootstrap.json \
SHUYRA_KERNEL_ELF=/absolute/vanllia/platform/riscv/target/riscv64gc-unknown-none-elf/release/shuyra \
SHUYRA_KERNEL_REVISION=f8fc37207c909044e55c190a8af1b3e2435a377d \
SHUYRA_OPENSBI=/absolute/fw_jump.bin \
SHUYRA_FIRMWARE_SHA256=<sha256-of-the-selected-firmware> \
cargo +nightly test --locked --test qemu startup_abi_v8 -- --ignored --nocapture
```

This probe validates the Capsule v2/ABI v8 initrd with Kernel API. The Kernel
supplies the initial View and Ledger argument words, which the startup ELF
reads through its pointer/count entry contract before calling `Unit.Exit` with
`ExitScope::Task`. The probe checks the number
of admitted harts, exactly one Kernel exit record for task 1 with reason zero
and note `toolchain: startup passed`, and successful all-tasks-exited shutdown.

## ABI v8 std backend

Run Toolchain's `build-std` command and use the same Kernel image as the startup
probe. The QEMU test builds the no-std launcher with the
`riscv64gc-unknown-none-elf` target. Install it with
`rustup target add --toolchain nightly riscv64gc-unknown-none-elf`.
The backtrace probe requires OpenSBI with the debug console extension (DBCN)
and a host `addr2line` that reads RV64 ELF files.

```sh
SHUYRA_TOOLCHAIN_RECEIPT=/absolute/toolchain/build/std.json \
SHUYRA_KERNEL_ELF=/absolute/vanllia/platform/riscv/target/riscv64gc-unknown-none-elf/release/shuyra \
SHUYRA_KERNEL_REVISION=f8fc37207c909044e55c190a8af1b3e2435a377d \
SHUYRA_OPENSBI=/absolute/fw_jump.bin \
SHUYRA_FIRMWARE_SHA256=<sha256-of-the-selected-firmware> \
cargo +nightly test --locked --test qemu std_main_allocator_and_termination -- --ignored --nocapture
```

`SHUYRA_QEMU` selects the native QEMU binary. Each scenario runs on one and
two harts with a 20-second timeout. The launcher checks the retained Team and
initiating Task results through `Unit.Inspect` and waits for Team retirement.
The host verifies the admitted hart count, the Kernel exit records and
all-tasks-exited shutdown.

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
symbolized traces. The opt-in test covers 24 QEMU runs.

Run the native QEMU executable directly with `SHUYRA_QEMU`; shell wrappers and
external helper processes are outside this small harness's scope. Each owned
child is killed and reaped on timeout or an evidence/write/wait failure. Ordinary
`cargo +nightly test --locked --test qemu` runs the host timeout/nonzero-exit
regression and leaves guest execution ignored until explicitly requested.
