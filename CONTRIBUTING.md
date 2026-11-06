# Contributing

Follow the [Shuyra contribution standard](https://github.com/shuyra-arche/.github/blob/main/CONTRIBUTING.md)
and [engineering conventions](https://github.com/shuyra-arche/.github/tree/main/docs/engineering).

Use `main` and short-lived branches, the organization PR template, and squash
merges. Keep future work and unsettled design in Issues. README describes what
actually exists, including runnable commands and remaining verification gaps.

## Local rules

- Modules with children use `name/mod.rs`; no sibling `name.rs` and `name/`.
- Keep tests, fixtures and test support under `tests/`, grouped by behavior.
  Use `tests/<domain>/mod.rs`, explicit `[[test]]` entries and
  `autotests = false` for Rust suites. Disable library doctests when applicable.
- Do not add empty packages, placeholder backends, generated source copies or
  a `cluster.toml` to describe organizational boundaries.
- Maintain Vanllia backend files in `rust/library/` only. Keep patches limited to
  current upstream integration and dependency adaptations; use Git for history,
  not layers of patches rewriting the same backend. Do not duplicate owned code
  in patches or overwrite upstream files through the source installation step.
- Rust follows the configured rolling channel; record actual compiler and paired
  source fingerprints. Pin other cross-repository Git inputs to full revisions.
  Keep generated upstream trees/build outputs out of Git and preserve source
  attribution and licensing notices.
- Consume Kernel API/Client and service contracts. Do not introduce a reverse
  Kernel dependency or a parallel Runtime/Execution abstraction.
- Record host compilation, target compilation and guest execution separately.
  A compiled ELF alone is not evidence that it starts or exits correctly.
- Treat compiler/LLVM host requirements separately from target-library
  requirements. The Vanllia backend should not require libc or a POSIX shim.

Use the build and verification commands in README. Keep host checks, target
compilation and guest execution explicit in PR evidence. Add CI gates only after
running the corresponding real command successfully.
