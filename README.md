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
| Kernel API/Client (startup, std and QEMU) | [`602ee97b8a4a5bc924c369c079c19ed00954e4c0`](https://github.com/shuyra-arche/kernel/tree/602ee97b8a4a5bc924c369c079c19ed00954e4c0) — ABI v7 |

Use the matching Kernel and Capsule v2 for both targets.

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
installs our backend from `rust/library/`, vendors the locked dependency inputs,
and applies `rust/patches/series` with `git apply --check`. `build-std` also
prepares this tree if needed. Installed Rust sources/compiler remain unchanged.
The private sysroot reuses immutable host libraries and copies rustc/rustc_driver
so Cargo selects the prepared sources. It is a build workspace, not a distribution.

| Maintained input | Responsibility |
| --- | --- |
| `rust/library/std/src/sys/` | Canonical Vanllia allocator, startup/TLS, threads, waits, time, backtrace and unsupported console backend as ordinary Rust files |
| `rust/patches/upstream.patch` | Current upstream platform selection, dependency declarations and other integration edits |
| `rust/patches/dependencies.patch` | Private dependency manifest adaptations for sysroot core/alloc and relocated Kernel packages |
| `rust/dependencies.toml` / `.lock` | External input selection and locked Cargo resolution |

Backend files are maintained only in `rust/library/`, with paths relative to
upstream `library/`; they are not duplicated inside patches. Patches describe
the current integration, not a history of feature additions. Update a backend
file directly, or update the relevant integration patch against the unmodified
selected inputs. Then run `prepare-std` and `build-std`; build/boot evidence is
still required for behavior changes. There is no separate Runtime package.

Owned files may only add paths absent from upstream. Preparation refuses a path
collision, symlinked source, or patch that changes an owned backend file. File
content and path fingerprints invalidate the prepared tree on additions, edits
and removals. The std build resets its Cargo cache when the prepared dependency
fingerprint or compiler changes, including changes to vendored ABI macros.
Build receipts also detect changes during compilation. Updating
nightly may still require adapting upstream interfaces or integration hunks;
failed patch application is reported rather than silently skipped.

Dependency versions come from `rust/dependencies.lock`, including host
proc-macro inputs. Cargo resolves them through an ignored, generated manifest
that is never built; std does not depend on a test fixture. Package source and
licensing notices are retained in the generated tree.

| Boundary | Implemented behavior |
| --- | --- |
| Entry | `_start` initializes the process broker. The broker creates the Rust main Task, which installs TLS and invokes rustc's generated `main`. `std::env::args()` is empty. |
| Normal return | Upstream `std::rt` calls user `main` and `Termination::report`. After normal cleanup and TLS destructors, the main Task commits the Team exit value through `Unit.Exit(Team)`. |
| Explicit exit / abort | `process::exit` preserves the 32-bit code in the Team result. Panic and allocation-error handling use the Kernel panic reason and abort without unwinding. |
| Allocation | A process-shared `TalcLock` obtains backing pages through the broker. The broker holds the private Cell authority and publishes or retracts its Internal Memos. |
| Failure / realloc | The broker retries transient mapping contention, rolls back failed publication and returns null for failed allocation. Failed realloc preserves the original allocation. |

Talc 5.1.1 reuses and coalesces small allocations in shared heaps. Backing blocks
start at 16 KiB and grow to fit requests; completely free heaps are returned to
Kernel, while the first heap can remain for Talc metadata. There is no fixed heap
or per-object mapping. Alignment, zeroing and realloc preservation are supported;
impossible sizes are rejected before Talc's source-size arithmetic.

The process broker runs in the initial Task of a Team configured with
`member_exit`. The launcher grants that Task a `None` Pend with `FLAT | SIGN`
before resuming it. The broker uses the grant to create backing Slots and a
Sign endpoint. Worker Tasks receive a narrow Trigger branch for the endpoint.
The shared mailbox uses address-space-private `Room.WaitValue` for callers and
a retained Sign notification for the broker. Backing allocation and cleanup
run without entering Rust's allocator on the broker. Mapping control survives
the lifetime of individual allocating and freeing threads.

The broker creates and resumes each Rust Task and owns its Kernel exit receipt.
It observes `SignSource::Join`, reads `Unit.Inspect` and wakes the Rust
`JoinHandle` after the worker's exit. The worker separately publishes Rust
completion after its closure and TLS destructors. A joined Task that ends
before Rust completion aborts the Team. Spawn failure releases the closure.
A `JoinHandle` can move to another thread or outlive its creator.

The broker releases its Task receipts after Kernel reclamation. A Team creator
that receives its own receipts must also release completed records to sustain
sequential thread creation. Normal main return, explicit exit and panic-abort
close the whole Team, including detached Tasks.

`thread_local!` uses upstream key-based storage with a per-task table addressed
by RISC-V `tp`; it does not require ELF TLS segments. Tables are installed before
std initialization on the Rust main Task and every worker. Values are
isolated, destructors run before current-thread cleanup, and `join` waits for
this cleanup. Destructors may initialize other keys; resurrection is limited to
four passes, after which remaining values may leak, as with bounded key-based
TLS destruction. Main-thread TLS is also cleaned on normal runtime exit.

Mutex, Condvar, RwLock, Once and park/unpark use upstream atomic algorithms with
Kernel conditional waits: an atomic comparison and queue insertion share the
wake commit boundary, so a notification cannot be lost between checking and
blocking. Wake-one returns the actual number removed; broadcast drains current
waiters without leaving notification tokens. Queues are private to each Space.
Only transient mapping `Busy` retries yield; ordinary waiters leave the runnable
queue. Timeouts retain the original budget across those retries. This does not
add fairness guarantees. `Instant`, sleep and timed waits use Kernel's monotonic clock.
`available_parallelism` reports unavailable rather than inventing a CPU count.
The broker reserves completion slots for up to 1022 workers or unreleased
JoinHandles. The Kernel counts the broker and main against the Team member limit.

`std::backtrace::Backtrace::force_capture()` captures RV64 frame pointers on the
Rust main thread and workers. The target keeps frame pointers enabled. The
runtime obtains stack bounds from the Task entry argument block and the stack
size requested by the broker. The walk stops at an invalid link or after 128
frames. Display prints return addresses. To resolve a frame on the host, run
`addr2line -f -C -e <matching-elf> <return-address-minus-one>` against the exact
unstripped executable.

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
- `std-backtrace`: main and worker captures, with the worker's trace formatted
  after it exits and both call chains resolved by the host probe.

Toolchain owns the opt-in [direct Kernel QEMU suite](tests/qemu/README.md) under
`tests/qemu/`. It checks these ELF hashes, validates their format through the
public inspect CLI, and encodes the Kernel boot capsule using Kernel API's public
format and parser. The native QEMU child has a bounded timeout and captured logs.
The test requires matching Kernel exit records and all-tasks-exited shutdown on
one and two harts. Evidence is stored under `target/qemu/<run>/`, separately from
the build receipt.

The startup fixture boots as Kernel's initial supervisor Task. A test launcher
starts the std fixtures in opted-in User Teams and observes their Task and Team
exit values before retirement. Vanllia owns the assembled System Loader and
artifact-handoff tests.

## Executable contract

The ABI v7 startup fixture has a [direct Kernel QEMU probe](tests/qemu/README.md#abi-v7-startup)
that boots Capsule v2 and checks Kernel termination on one and two harts.

- Static ELF64, little-endian RV64GC/LP64D, entry at `0x10000`.
- Separate readable RX, R and RW load segments with nonoverlapping 4 KiB pages.
  No dynamic interpreter, dynamic segment or TLS segment.
- Kernel supplies a 16-byte-aligned stack, argument word pointer in `a0`, and word
  count in `a1`. The fixture initializes `gp`, preserves these arguments and
  never returns through an undefined caller address.
- The fixture checks entry registers, argument readability, zero-filled BSS and
  initialized writable data. It then calls Kernel Client `unit::exit` with
  `ExitScope::Task`, result zero, reason zero and note `toolchain: startup passed`.
  Failures report nonzero reasons.
  Kernel calls use the Client's `ebreak` transport, not an assumed Unix syscall.

`inspect` validates this bootstrap subset and reports its SHA-256. It is a
host-side check, not System's Loader or proof that an ELF has run. The exit call
terminates this task; it does not implement Rust process-wide panic/exit policy.
The fixture does not implement the legacy Env/Execution startup handshake.

## Verification

```sh
cargo +nightly fmt --all -- --check
rg --files --null rust/library -g '*.rs' | xargs -0 rustfmt +nightly --edition 2024 --check
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
two harts. System Loader and physical hardware validation are tracked separately. Toolchain CI checks host behavior and
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
| `rust/` | Rust channel, maintained backend sources, locked dependencies, integration patches and target settings |
| `tests/bootstrap/mod.rs` | Host CLI contract tests |
| `tests/qemu/mod.rs` | Direct Kernel runtime acceptance and test-only boot/process helpers |
| `tests/target/startup/` | Bootstrap fixture, test-only std launcher, linker scripts and Unit declaration |
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
