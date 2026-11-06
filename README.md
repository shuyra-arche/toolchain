# Toolchain

Rust toolchain support for Vanllia, maintained by Shuyra.

The CLI prepares a pinned Rust compiler/library source pair and builds static
RV64 programs. It supports the original `no_std` startup probe and an experimental
single-thread `std` backend with ordinary Rust `main`, normal return, explicit
exit, panic-abort and a Kernel-backed allocator. The custom
`riscv64gc-unknown-vanllia` target remains experimental; there is no installable
sysroot or native Vanllia compiler yet. Further work lives in
[Issue #1](https://github.com/shuyra-arche/toolchain/issues/1).

## Build

The initial host is Linux x86_64. Install rustup and Git, with access to the
Rust distribution server, crates.io and GitHub. Run from this checkout:

```sh
rustup toolchain install nightly-2026-09-04 --profile minimal --component rust-src,rustfmt,clippy
cargo +nightly-2026-09-04 run --locked -- prepare
cargo +nightly-2026-09-04 run --locked -- build
cargo +nightly-2026-09-04 run --locked -- inspect build/cargo/riscv64gc-unknown-vanllia/release/toolchain-startup
```

`prepare` uses the official rustup distribution's `rust-src`, paired with the
compiler. `rust/source.toml` pins the date, full upstream commit, host and target;
`build` verifies the installed compiler's commit/host and required source files.
The bootstrap command uses upstream Cargo `-Zbuild-std=core,compiler_builtins`;
it does not rebuild rustc. No `rust-toolchain.toml` or default
toolchain change is needed. Both Cargo lockfiles are committed.

| Input | Revision |
| --- | --- |
| Rust compiler and library source | [`a69a63265cfd9e006d43137f98301b8d274ad4c9`](https://github.com/rust-lang/rust/tree/a69a63265cfd9e006d43137f98301b8d274ad4c9), nightly-2026-09-04 |
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
cargo +nightly-2026-09-04 run --locked -- prepare-std
cargo +nightly-2026-09-04 run --locked -- build-std
```

`prepare-std` copies matching Rust library sources into ignored `work/std-stage/`,
vendors the Kernel dependency graph from the committed startup lockfile, then
applies `rust/patches/series` with `git apply --check`. `build-std` also prepares
this tree if needed. The installed Rust source/compiler remains unchanged. The
private sysroot reuses immutable host libraries and copies rustc/rustc_driver so
Cargo selects the patched sources. It is a build workspace, not a distribution.

The backend lives in upstream `std` patches, with no separate Runtime package.
Kernel API/Client and their no_std dependencies receive only private Cargo
manifest adaptations to use the same `core` as std. All vendored package versions
come from the startup lockfile, including host proc-macro dependencies. Original
source and licensing notices are retained in the generated tree.

| Boundary | Implemented behavior |
| --- | --- |
| Entry | `_start` establishes `gp`, checks stack alignment and invokes rustc's generated `main` entry; Kernel argument words are not miscast as C strings, so `std::env::args()` is empty |
| Normal return | Upstream `std::rt` initializes the main thread, calls user `main` and `Termination::report`, then runs std cleanup; the backend cleans current-thread state and calls Kernel Client `Reap` |
| Explicit exit / abort | `process::exit` preserves its code; panic and allocation-error handling abort with the Kernel panic reason, without unwinding |
| Allocation | Each allocation acquires a source and attaches an aligned RW mapping; metadata retains reference/view ownership; deallocation detaches then releases |
| Failure / realloc | Arithmetic/acquire/attach failure returns null; failed attachment releases its source; failed realloc preserves the old allocation; cleanup failure aborts without allocating |

This allocator favors a small ownership implementation over efficiency: even
small allocations use page-backed mappings and Kernel calls. It supports aligned
and zeroed allocation, growth and shrinking. Physical page materialization stays
under Kernel policy.

The target explicitly disables threads. Thread creation is unsupported; direct
Kernel task creation must not be mixed into a program using this single-thread
std. User TLS destructors, service-backed console/fs/net/process facilities,
unwinding and service argument translation are not implemented. Console reads
and writes return Unsupported, and unavailable random generation fails explicitly.
Debug output is not substituted for stdout. Static objects are not automatically
dropped at exit; normal Rust local drops are provided by Rust itself.

`build/std.json` records the prepared source/patch/dependency fingerprints, four
ELF hashes and the target library inventory; the command rejects a target `libc`
dependency. It always records `guest_execution: not_run`, because building does
not execute the programs. Outputs are under
`build/std/riscv64gc-unknown-vanllia/release/`:

- `toolchain-std`: main-thread initialization, local Drop/Termination ordering,
  Vec/Box/String, over-alignment, zeroing, realloc preservation and mapping reuse.
- `std-exit`: explicit exit code 23.
- `std-panic`: panic-abort.
- `std-oom`: allocation-error-handler abort.

Vanllia owns the opt-in `tests/toolchain/` QEMU scenario. It consumes these actual
ELFs and their hashes, reuses its capsule encoder and timeout runner, and checks
Kernel exit records plus clean all-tasks-exited shutdown. This directly boots the
fixture as Kernel's initial supervisor task; it does not run System's Loader or
claim user-mode deployment coverage. Results and input hashes are recorded in
Vanllia's `target/toolchain/<run>/`, separately from the build receipt.

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
cargo +nightly-2026-09-04 fmt --all -- --check
cargo +nightly-2026-09-04 fmt --manifest-path tests/target/startup/Cargo.toml -- --check
cargo +nightly-2026-09-04 fmt --manifest-path tests/target/std/Cargo.toml -- --check
cargo +nightly-2026-09-04 test --locked --test bootstrap
cargo +nightly-2026-09-04 clippy --locked --all-targets -- -D warnings
cargo +nightly-2026-09-04 run --locked -- build
```

The host suite exercises the CLI's rejection of malformed/unsafe ELF files,
mutable source pins, and stale success receipts. The independent guest package
is under `tests/target/startup/`; host tests do not build it automatically.

Host checks and the target build have passed. Vanllia's planner can select the
minimum `system + toolchain.startup` composition via the fixture's `unit.toml`.
Assembled execution still needs a compatible Toolchain artifact handoff in
Vanllia, with observed task termination and bounded failure/timeout handling.
The four std fixtures have passed direct-Kernel QEMU execution. System-mediated
execution and hardware remain unverified. Toolchain CI checks host behavior and
target compilation; Vanllia owns the explicit QEMU scenario.

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
| `rust/` | Pinned Rust inputs and custom target settings |
| `tests/bootstrap/mod.rs` | Host CLI contract tests |
| `tests/target/startup/` | Bootstrap fixture, linker script and Unit declaration |
| `tests/target/std/` | Ordinary main, allocator and termination consumers |

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
