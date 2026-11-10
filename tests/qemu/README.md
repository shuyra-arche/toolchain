# Direct Kernel QEMU probes

This focused scenario boots Toolchain's actual ELF as Kernel's initial task,
using test-only capsule and native-child helpers in this repository. No System,
console, filesystem or other service is needed. It verifies the direct Kernel
startup/memory/termination boundary in supervisor mode; it does **not** verify
System's ELF loader, user-mode deployment or its lifecycle handshake.

Both probes are opt-in. Supply an explicit OpenSBI firmware and its expected
SHA-256. The suite never downloads or substitutes these inputs.

## ABI v6 startup

Run Toolchain's `build` command. Build the normal `shuyra` ELF from Vanllia's
`platform/riscv` at `eb37b0b0db299513274864cec23cee509a3cd3cc` (the integration
revision used by this Kernel's CI), with both `kernel` and `kernel-api` resolved
to the startup fixture's Kernel revision. Refresh the platform lockfile before
building the image. Use the resulting ELF with `build/bootstrap.json`:

```sh
SHUYRA_TOOLCHAIN_RECEIPT=/absolute/toolchain/build/bootstrap.json \
SHUYRA_KERNEL_ELF=/absolute/vanllia/platform/riscv/target/riscv64gc-unknown-none-elf/release/shuyra \
SHUYRA_KERNEL_REVISION=9610a3e3e6a4c6d490fa83a408436aa637e2a577 \
SHUYRA_OPENSBI=/absolute/fw_jump.bin \
SHUYRA_FIRMWARE_SHA256=<sha256-of-the-selected-firmware> \
cargo +nightly test --locked --test qemu startup_abi_v6 -- --ignored --nocapture
```

This probe validates the Capsule v2/ABI v6 initrd with Kernel API. The Kernel
supplies the initial View and Ledger argument words, which the startup ELF
reads through its pointer/count entry contract. The probe checks the number
of admitted harts, exactly one Kernel exit record for task 1 with reason zero
and note `toolchain: startup passed`, and successful all-tasks-exited shutdown.

## Historical std backend

Run Toolchain's `build-std` command and build the historical Kernel revision
it consumes. This probe uses `build/std.json` and Capsule v1:

```sh
SHUYRA_TOOLCHAIN_RECEIPT=/absolute/toolchain/build/std.json \
SHUYRA_KERNEL_ELF=/absolute/kernel/target/riscv64gc-unknown-none-elf/release/shuyra \
SHUYRA_KERNEL_REVISION=65e89e1fd52e7fdf14e8ddcb65e75219a5684144 \
SHUYRA_OPENSBI=/absolute/fw_jump.bin \
SHUYRA_FIRMWARE_SHA256=<sha256-of-the-selected-firmware> \
cargo +nightly test --locked --test qemu std_main_allocator_and_termination -- --ignored --nocapture
```

`SHUYRA_QEMU` can select a `qemu-system-riscv64` executable. Each scenario runs
on both one and two harts, with a 20-second timeout per run and must exit QEMU successfully, emit exactly one matching
Kernel exit record, and reach Kernel's all-tasks-exited shutdown. A printed
application marker alone does not pass. The expected records are:

| Fixture | Reason | Kernel note |
| --- | --- | --- |
| `toolchain-std` | `0` | `toolchain: std main returned` |
| `std-exit` | `23` | `toolchain: std process exit` |
| `std-panic` | `0xffffff01` | `toolchain: std abort` |
| `std-oom` | `0xffffff01` | `toolchain: std abort` |
| `std-threads` | `0` | `toolchain: std main returned` |
| `std-waits` | `0` | `toolchain: std main returned` |
| `std-thread-exit` | `23` | `toolchain: std process exit` (task 2) |
| `std-thread-panic` | `0xffffff01` | `toolchain: std abort` (task 2) |
| `std-tls-main` | `0xffffff01` | `toolchain: std abort` |
| `std-thread-fault` | `0xffffff01` | `toolchain: std abort` (joining task 1) |

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
and the initial task on worker exit/panic. An illegal-instruction worker checks
that join aborts when Kernel ends a task before Rust cleanup. The wait fixture checks Kernel queue counts, comparison mismatch, poll and timed
expiry, invalid addresses, event-key separation, wake-one/broadcast, finite-timeout
overflow, 256 producer/consumer races, std Condvar broadcast, park/unpark and
termination of a blocked detached worker. These are 20 QEMU
runs in one opt-in test, not host emulation of the Kernel boundary.

Run the native QEMU executable directly with `SHUYRA_QEMU`; shell wrappers and
external helper processes are outside this small harness's scope. Each owned
child is killed and reaped on timeout or an evidence/write/wait failure. Ordinary
`cargo +nightly test --locked --test qemu` runs the host timeout/nonzero-exit
regression and leaves guest execution ignored until explicitly requested.
