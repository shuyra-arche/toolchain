# Toolchain

Rust toolchain support for Vanllia, maintained by Shuyra.

This repository is being established. It does not yet contain a Vanllia Rust
target, a standard-library backend, or an installable toolchain. There are no
build or installation commands to run yet. Implementation and acceptance work
is tracked in [Issue #1](https://github.com/shuyra-arche/toolchain/issues/1),
rather than duplicated in a roadmap under `docs/`.

## Scope

| Area | Responsibility |
| --- | --- |
| Target | Rust/LLVM target settings, linking and executable ABI adaptation |
| Standard library | Adapt upstream Rust libraries to Kernel contracts and Vanllia services |
| Build and distribution | Reproducible source selection, patches, sysroot construction and toolchain installation |
| Host tools | Eventually run rustc and Cargo on Vanllia itself |

The first goal is a toolchain running on an existing host that builds programs
for Vanllia. Running the compiler and Cargo on Vanllia is a later, separately
validated goal. Building a target standard library does not by itself establish
native compiler support.

Kernel owns its API and ABI. This repository consumes those public contracts;
Kernel must not depend on Toolchain. Files, networking, process creation and
console I/O depend on the relevant Vanllia services, not an assumed Unix API.
Platform remains the hardware-porting domain.

The intended Rust backend uses Kernel Client and service contracts directly,
without a libc/POSIX compatibility layer or a separate Runtime/Execution
framework. Host-side compiler build requirements and eventual native compiler
dependencies need their own audit; this repository makes no claim that those
requirements have already been eliminated.

## Sources and layout

Use upstream Rust implementations and keep local adaptation focused. Record full
upstream and cross-repository revisions when executable build inputs are added.
No upstream revision, custom target triple or Kernel dependency is pinned yet.
Do not add a `rust-toolchain.toml` override as part of repository scaffolding.

Add directories only when they contain an implementation, configuration or test
with a defined owner. There is no empty `std`, `cargo`, `runtime` or `cluster`
scaffold. Modules with children use `name/mod.rs`; never put `name.rs` next to a
`name/` directory. This rule applies to tests as well.

All test code, fixtures and support belong under the owning package's `tests/`.
Rust suites use `tests/<domain>/mod.rs` with explicit Cargo `[[test]]` entries.
Production `no_std` libraries stay `no_std` during host testing. Tests will
protect observable contracts and invariants rather than mirror source files.

## Current verification and contribution

This initial repository contains documentation and ignore rules only. There are
no compilation, guest execution or standard-library results to report. Add CI
when the first real build or test command has succeeded; do not create a dummy
passing check. Vanllia owns assembled-system execution.

Follow [CONTRIBUTING.md](CONTRIBUTING.md) and the organization's engineering
conventions. New implementation lands through focused PRs. This bootstrap does
not choose a license for the project or import upstream source; retain upstream
notices when source is introduced and resolve redistribution terms before
publishing toolchain artifacts.
