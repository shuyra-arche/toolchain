# Toolchain

Rust toolchain support for Vanllia, maintained by Shuyra.

The CLI prepares the selected Rust compiler/library source pair and builds static
RV64 programs. It supports the original `no_std` startup probe and an experimental
`std` backend with ordinary Rust `main`, normal return, explicit
exit, panic-abort, threads, key-based TLS and a Kernel-backed allocator. The custom
`riscv64gc-unknown-vanllia` target remains experimental; there is no installable
sysroot or native Vanllia compiler yet. Further work lives in
[Issue #1](https://github.com/shuyra-arche/toolchain/issues/1).

## Build

The initial host is Linux x86_64. Install rustup and Git, with access to the
Rust distribution server, crates.io and GitHub. Run from this checkout:

```sh
rustup toolchain install nightly --profile minimal --component rust-src,rustfmt,clippy
cargo +nightly run --locked -- prepare
cargo +nightly run --locked -- build
cargo +nightly run --locked -- inspect build/cargo/riscv64gc-unknown-vanllia/release/toolchain-startup
```

`prepare` uses the official rustup distribution's `rust-src`, paired with the
compiler. `rust/source.toml` selects the rolling `nightly` channel, host and target;
no Rust date or commit is fixed. `prepare` updates the selected distribution;
`build` uses its installed compiler and checks the host and required source files.
Receipts record the actual compiler commit/version and source hash. Run `prepare`
again to update nightly. Patch or target incompatibilities fail visibly and require
adapting this repository; the build does not silently fall back to an older Rust.
The bootstrap command uses upstream Cargo `-Zbuild-std=core,compiler_builtins`;
it does not rebuild rustc. No `rust-toolchain.toml` or default
toolchain change is needed. Host and guest Cargo lockfiles are committed.

| Input | Revision |
| --- | --- |
| Rust compiler and library source | Rolling `nightly`; actual version/commit/source hash in each build receipt |
| Kernel API/Client | [`17059c2922c1084d3b816f54d02226488ddb8298`](https://github.com/shuyra-arche/kernel/tree/17059c2922c1084d3b816f54d02226488ddb8298) |

Successful builds write `build/bootstrap.json` with the compiler identity, a
fingerprint of the installed Rust library source tree, hashes of fixture inputs,
and the ELF hash. The source fingerprint records actual installed contents; it
is not an independent verification against the upstream Git tree. Rustup owns
distribution checksum verification. Use an unmodified rustup installation and no
additional Cargo config overrides for the documented build. Inherited compiler
wrappers, flags and profile overrides are cleared for the target build.

A failed build removes the previous success receipt. Existing ELF files alone
are not success evidence. `guest_execution` remains `not_run`: the build command
does not execute a guest. `prepare` and `build` accept `--root DIRECTORY` when
invoked elsewhere. Use one build invocation per checkout at a time.

## Minimal std

After `prepare`, run:

```sh
cargo +nightly run --locked -- prepare-std
cargo +nightly run --locked -- build-std
```

`prepare-std` copies matching Rust library sources into ignored `work/std-stage/`,
vendors Kernel/Talc inputs from `rust/dependencies.toml` and its committed
`dependencies.lock`, then applies `rust/patches/series` with `git apply --check`. `build-std` also prepares
this tree if needed. The installed Rust source/compiler remains unchanged. The
private sysroot reuses immutable host libraries and copies rustc/rustc_driver so
Cargo selects the patched sources. It is a build workspace, not a distribution.

The backend lives in upstream `std` patches, with no separate Runtime package.
Kernel API/Client and their no_std dependencies receive only private Cargo
manifest adaptations to use the same `core` and `alloc` as std. All vendored package versions
come from `rust/dependencies.lock`, including host proc-macro dependencies.
Cargo resolves these inputs through an ignored, generated manifest that is never
built; it does not add a runtime package or make std depend on a test fixture. Original
source and licensing notices are retained in the generated tree.

| Boundary | Implemented behavior |
| --- | --- |
| Entry | `_start` establishes `gp`, checks stack alignment and invokes rustc's generated `main` entry; Kernel argument words are not miscast as C strings, so `std::env::args()` is empty |
| Normal return | Upstream `std::rt` initializes the main thread, calls user `main` and `Termination::report`, then runs std cleanup; the backend runs TLS destructors, stops other std-managed tasks and calls Kernel Client `Reap` |
| Explicit exit / abort | `process::exit` preserves its code; panic and allocation-error handling abort with the Kernel panic reason, without unwinding |
| Allocation | A process-shared `TalcLock` heap supplies objects; `GlobalAllocSource<Pages>` obtains backing mappings with SpaceMembers control and immediately releases temporary task-local source handles |
| Failure / realloc | Transient mapping `Busy` yields and retries; other arithmetic/acquire/attach failure returns null; failed attachment releases its source; failed realloc preserves the old allocation; cleanup failure aborts without allocating |

Talc 5.1.1 reuses and coalesces small allocations in shared heaps. Backing blocks
start at 16 KiB and grow to fit requests; completely free heaps are returned to
Kernel, while the first heap can remain for Talc metadata. There is no fixed heap
or per-object mapping. Alignment, zeroing and realloc preservation are supported;
impossible sizes are rejected before Talc's source-size arithmetic.

The heap lock uses atomics and direct Kernel yield, without allocation, TLS or
std locks. Backing allocation calls Kernel directly, never `System` recursively.
Cross-thread frees and creator-thread exit retain the existing SpaceMembers
ownership semantics. This is one shared heap, not a per-thread or scalable
sharded allocator. Physical page materialization stays under Kernel policy.

`std::thread::spawn`, `Builder`, `join`, scoped threads and detached workers use
Kernel tasks in the same Space. Kernel owns stacks and their reclamation. A
completion flag is published after the closure and TLS cleanup; `join` acquires
it before returning. If Kernel terminates a joined worker before cleanup, the
process aborts instead of waiting forever. Spawn failure releases the closure.
Normal main return, explicit exit and Rust panic-abort stop the other registered
std tasks. Raw Kernel-created tasks are outside this registry and must not be
mixed with these process-lifecycle assumptions. They also lack this TLS bootstrap
and cannot directly enter this std backend.

`thread_local!` uses upstream key-based storage with a per-task table addressed
by RISC-V `tp`; it does not require ELF TLS segments. Tables are installed before
std initialization on the initial task and every spawned task. Values are
isolated, destructors run before current-thread cleanup, and `join` waits for
this cleanup. Destructors may initialize other keys; resurrection is limited to
four passes, after which remaining values may leak, as with bounded key-based
TLS destruction. Main-thread TLS is also cleaned on normal runtime exit.

Mutex, Condvar, RwLock, Once and park/unpark use upstream atomic algorithms with
a cooperative polling backend. Waiters recheck state after yielding; this is
correctness-first and does not provide efficient blocked futex waits or fairness
guarantees. `Instant`, sleep and timed waits use Kernel's monotonic clock.
`available_parallelism` reports unavailable rather than inventing a CPU count.
The task registry is bounded at 1024 entries, including the initial task.

Service-backed console/fs/net/process facilities, wall-clock SystemTime,
unwinding and service argument translation are not implemented. Console reads
and writes return Unsupported, and unavailable random generation fails explicitly.
Debug output is not substituted for stdout. Static objects are not automatically
dropped at exit. Panic-abort and termination of other tasks do not run their TLS
destructors; automatic process-wide handling of a detached task's Kernel fault
is not implemented.

`build/std.json` records the prepared source/patch/dependency fingerprints, guest
ELF hashes and the target library inventory; the command rejects a target `libc`
dependency. It always records `guest_execution: not_run`, because building does
not execute the programs. Outputs are under
`build/std/riscv64gc-unknown-vanllia/release/`:

- `toolchain-std`: main-thread initialization, local Drop/Termination ordering,
  Vec/Box/String, over-alignment, zeroing, realloc preservation and mapping reuse.
- `std-exit`: explicit exit code 23.
- `std-panic`: panic-abort.
- `std-oom`: allocation-error-handler abort.
- `std-threads`: concurrent TLS isolation/destructors, cross-thread ownership,
  nested creators, scoped/detached threads, lock/once contention, timed waits,
  spawn failure, and main return with a live detached worker.
- `std-thread-exit`, `std-thread-panic`: process termination from a worker.
- `std-tls-main`: main TLS destructor execution, observed through intentional abort.
- `std-thread-fault`: a Kernel illegal-instruction fault must abort join rather
  than hang or return normally.

Toolchain owns the opt-in [direct Kernel QEMU suite](tests/qemu/README.md) under
`tests/qemu/`. It checks these ELF hashes, validates their format through the
public inspect CLI, and encodes the Kernel boot capsule using Kernel API's public
format and parser. The native QEMU child has a bounded timeout and captured logs.
The test requires matching Kernel exit records and all-tasks-exited shutdown on
one and two harts. Evidence is stored under `target/qemu/<run>/`, separately from
the build receipt.

This directly boots the fixture as Kernel's initial supervisor task; it does not
run System's Loader or claim user-mode deployment coverage. Vanllia owns tests
that exercise its actual composition, artifact handoff and System Loader path.
The Toolchain suite does not depend on Vanllia or its build library.

## Executable contract

The initial contract was checked against Kernel at the revision above and
[System Loader at `4177004`](https://github.com/shuyra-arche/system/blob/4177004b7f39c8e6458c158a5c4e0a4a71f48097/server/src/system/loader/image/elf.rs).

- Static ELF64, little-endian RV64GC/LP64D, entry at `0x10000`.
- Separate readable RX, R and RW load segments with nonoverlapping 4 KiB pages.
  No dynamic interpreter, dynamic segment or TLS segment.
- Kernel supplies a 16-byte-aligned stack, argument word pointer in `a0`, and word
  count in `a1`. The fixture initializes `gp`, preserves these arguments and
  never returns through an undefined caller address.
- The fixture checks entry registers, argument readability, zero-filled BSS and
  initialized writable data. It then calls Kernel Client `room::reap` with reason
  zero and note `toolchain: startup passed`; failures use nonzero reasons.
  Kernel calls use the Client's `ebreak` transport, not an assumed Unix syscall.

`inspect` validates this bootstrap subset and reports its SHA-256. It is a
host-side check, not System's Loader or proof that an ELF has run. The exit call
terminates this task; it does not implement Rust process-wide panic/exit policy.
The fixture does not implement the legacy Env/Execution startup handshake.

## Verification

```sh
cargo +nightly fmt --all -- --check
cargo +nightly fmt --manifest-path tests/target/startup/Cargo.toml -- --check
cargo +nightly fmt --manifest-path tests/target/std/Cargo.toml -- --check
cargo +nightly test --locked --test bootstrap --test qemu
cargo +nightly clippy --locked --all-targets -- -D warnings
cargo +nightly run --locked -- build
```

The host suite exercises the CLI's rejection of malformed/unsafe ELF files,
unsupported host/target settings, compiler identity recording, and stale success
receipts. The independent guest package is under `tests/target/startup/`; host tests do not build it automatically.

Host checks and the target build have passed. Vanllia's planner can select the
minimum `system + toolchain.startup` composition via the fixture's `unit.toml`.
Assembled execution still needs a compatible Toolchain artifact handoff in
Vanllia, with observed task termination and bounded failure/timeout handling.
The std fixtures are verified through direct-Kernel QEMU execution on one and
two harts. System-mediated
execution and hardware remain unverified. Toolchain CI checks host behavior and
target compilation and the native-child timeout regression. Direct Kernel QEMU
execution is opt-in in this repository; Vanllia owns assembled-system scenarios.

The initial assembly attempt stopped during System's Kernel macro source-overlay
resolution, before target compilation. The reproduction and execution acceptance
are tracked in [Vanllia #19](https://github.com/shuyra-arche/vanllia/issues/19).

## Ownership and layout

| Path | Responsibility |
| --- | --- |
| `src/source/mod.rs` | Prepare and verify the compiler/source pair |
| `src/build/mod.rs` | Build the startup target and publish its receipt |
| `src/build/standard/mod.rs` | Build/inspect std consumers and record their dependency inventory |
| `src/source/patch/mod.rs` | Prepare and fingerprint the isolated patched source tree |
| `src/elf/mod.rs` | Inspect the bootstrap executable contract |
| `rust/` | Rust channel, locked std dependency inputs, upstream patches and custom target settings |
| `tests/bootstrap/mod.rs` | Host CLI contract tests |
| `tests/qemu/mod.rs` | Direct Kernel runtime acceptance and test-only boot/process helpers |
| `tests/target/startup/` | Bootstrap fixture, linker script and Unit declaration |
| `tests/target/std/` | Main, allocation, thread/TLS and termination consumers |

Generated `build/`, `target/`, `work/` and `dist/` paths are ignored. Upstream
source edits are maintained only as patches; installation remains separate work. Modules with children
use `name/mod.rs`; no sibling `name.rs` and `name/`. All test code and fixtures
remain under `tests/`.

Kernel owns API/ABI; Toolchain consumes those public contracts. The target backend
must not introduce libc/POSIX calls, a reverse Kernel dependency, or a parallel
Runtime/Execution framework. Vanllia owns assembled-system execution. Compiler
host dependencies and eventual native compiler requirements are separate work.

Follow [CONTRIBUTING.md](CONTRIBUTING.md). Preserve upstream notices when sources
are imported or redistributed; this bootstrap does not settle the project's
redistribution license or publish toolchain binaries.
