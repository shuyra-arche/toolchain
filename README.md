# Toolchain

Rust toolchain support for Vanllia, maintained by Shuyra.

The bootstrap CLI prepares a pinned Rust compiler/library source pair, rebuilds
`core` and `compiler_builtins`, and produces a minimal static RV64 program. The
custom `riscv64gc-unknown-vanllia` target is experimental. There is no `alloc`/`std`
backend, installable sysroot or native Vanllia compiler yet. Further work lives in
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
This uses upstream Cargo `-Zbuild-std=core,compiler_builtins`; it does not rebuild
rustc or copy Rust source into this repository. No `rust-toolchain.toml` or default
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
QEMU and hardware execution have not been verified. CI checks host behavior and
target compilation only.

The initial assembly attempt stopped during System's Kernel macro source-overlay
resolution, before target compilation. The reproduction and execution acceptance
are tracked in [Vanllia #19](https://github.com/shuyra-arche/vanllia/issues/19).

## Ownership and layout

| Path | Responsibility |
| --- | --- |
| `src/source/mod.rs` | Prepare and verify the compiler/source pair |
| `src/build/mod.rs` | Build the startup target and publish its receipt |
| `src/elf/mod.rs` | Inspect the bootstrap executable contract |
| `rust/` | Pinned Rust inputs and custom target settings |
| `tests/bootstrap/mod.rs` | Host CLI contract tests |
| `tests/target/startup/` | Guest fixture, linker script and Unit declaration |

Generated `build/`, `target/`, `work/` and `dist/` paths are ignored. Add source
patches and installation modules only when implemented. Modules with children
use `name/mod.rs`; no sibling `name.rs` and `name/`. All test code and fixtures
remain under `tests/`.

Kernel owns API/ABI; Toolchain consumes those public contracts. The target backend
must not introduce libc/POSIX calls, a reverse Kernel dependency, or a parallel
Runtime/Execution framework. Vanllia owns assembled-system execution. Compiler
host dependencies and eventual native compiler requirements are separate work.

Follow [CONTRIBUTING.md](CONTRIBUTING.md). Preserve upstream notices when sources
are imported or redistributed; this bootstrap does not settle the project's
redistribution license or publish toolchain binaries.
