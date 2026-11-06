//! Source-preparation contracts through the CLI; no target execution is mocked.
use super::{Scratch, cli};
use std::{fs, path::PathBuf, process::Output};

struct Fixture {
    scratch: Scratch,
    upstream: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        use std::os::unix::fs::PermissionsExt;
        let scratch = Scratch::new();
        let root = &scratch.0;
        let sysroot = root.join("sysroot");
        let upstream = sysroot.join("lib/rustlib/src/rust/library");
        for file in [
            "Cargo.toml",
            "Cargo.lock",
            "core/src/lib.rs",
            "compiler-builtins/Cargo.toml",
        ] {
            let path = upstream.join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "upstream\n").unwrap();
        }
        fs::create_dir_all(sysroot.join("bin")).unwrap();
        for name in ["rustc", "rustdoc"] {
            fs::write(sysroot.join("bin").join(name), "unused compiler fixture").unwrap();
        }
        fs::create_dir_all(root.join("rust/patches")).unwrap();
        fs::create_dir_all(root.join("rust/library/backend")).unwrap();
        fs::write(
            root.join("rust/library/backend/mod.rs"),
            "pub fn version() -> u8 { 1 }\n",
        )
        .unwrap();
        fs::write(root.join("rust/source.toml"), "toolchain = 'nightly'\nhost = 'x86_64-unknown-linux-gnu'\ntarget = 'riscv64gc-unknown-vanllia'\n").unwrap();
        fs::write(root.join("rust/dependencies.toml"), "[dependencies]\n").unwrap();
        fs::write(root.join("rust/dependencies.lock"), "resolver fixture\n").unwrap();
        fs::write(root.join("rust/patches/series"), "upstream.patch\n").unwrap();
        fs::write(root.join("rust/patches/upstream.patch"), "diff --git a/library/Cargo.toml b/library/Cargo.toml\n--- a/library/Cargo.toml\n+++ b/library/Cargo.toml\n@@ -1 +1 @@\n-upstream\n+integrated\n").unwrap();
        let bin = root.join("bin");
        fs::create_dir(&bin).unwrap();
        let rustup = bin.join("rustup");
        // Isolate compiler discovery/downloads only. Filesystem preparation,
        // real git apply, collisions, fingerprints and cache reuse remain real.
        fs::write(&rustup, r#"#!/bin/sh
case "$*" in
  'run nightly-x86_64-unknown-linux-gnu rustc -Vv')
    printf '%s\n' 'rustc fixture-nightly' 'commit-hash: 1111111111111111111111111111111111111111' 'host: x86_64-unknown-linux-gnu' ;;
  'run nightly-x86_64-unknown-linux-gnu rustc --print sysroot') printf '%s\n' "$TEST_SYSROOT" ;;
  'run nightly-x86_64-unknown-linux-gnu cargo vendor --locked --manifest-path '*)
    for destination do :; done
    mkdir -p "$destination"
    printf '%s\n' 'dependency fixture' > "$destination/input" ;;
  *) exit 1 ;;
esac
"#).unwrap();
        fs::set_permissions(rustup, fs::Permissions::from_mode(0o755)).unwrap();
        Self { scratch, upstream }
    }
    fn prepare(&self) -> Output {
        let root = &self.scratch.0;
        let mut paths = vec![root.join("bin")];
        paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
        cli()
            .args(["prepare-std", "--root"])
            .arg(root)
            .env("PATH", std::env::join_paths(paths).unwrap())
            .env("TEST_SYSROOT", root.join("sysroot"))
            .output()
            .unwrap()
    }
    fn prepared(&self) -> PathBuf {
        self.scratch
            .0
            .join("work/std-stage/sysroot/lib/rustlib/src/rust/library")
    }
    fn success(&self) -> serde_json::Value {
        let output = self.prepare();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
}

#[test]
fn owned_sources_invalidate_prepared_cache_on_edit_add_and_remove() {
    let fixture = Fixture::new();
    let root = &fixture.scratch.0;
    let initial = fixture.success();
    assert_eq!(
        fs::read_to_string(fixture.upstream.join("Cargo.toml")).unwrap(),
        "upstream\n"
    );
    assert_eq!(
        fs::read_to_string(fixture.prepared().join("Cargo.toml")).unwrap(),
        "integrated\n"
    );
    assert_eq!(fixture.success(), initial);
    fs::write(
        root.join("rust/library/backend/mod.rs"),
        "pub fn version() -> u8 { 2 }\n",
    )
    .unwrap();
    let edited = fixture.success();
    assert_ne!(
        initial["inputs"]["rust/library"],
        edited["inputs"]["rust/library"]
    );
    assert!(
        fs::read_to_string(fixture.prepared().join("backend/mod.rs"))
            .unwrap()
            .contains("{ 2 }")
    );
    fs::write(
        root.join("rust/library/backend/extra.rs"),
        "pub const EXTRA: u8 = 3;\n",
    )
    .unwrap();
    let added = fixture.success();
    assert_ne!(
        edited["inputs"]["rust/library"],
        added["inputs"]["rust/library"]
    );
    assert!(fixture.prepared().join("backend/extra.rs").exists());
    fs::remove_file(root.join("rust/library/backend/extra.rs")).unwrap();
    assert_eq!(fixture.success(), edited);
    assert!(!fixture.prepared().join("backend/extra.rs").exists());
    // Tampering with the generated copy cannot silently become a new input.
    fs::write(fixture.prepared().join("backend/mod.rs"), "stale\n").unwrap();
    assert_eq!(fixture.success(), edited);
    assert!(
        !fs::read_to_string(fixture.prepared().join("backend/mod.rs"))
            .unwrap()
            .contains("stale")
    );
}

#[test]
fn refuses_upstream_collisions_and_symlinked_owned_sources() {
    let fixture = Fixture::new();
    fixture.success(); // A changed upstream must invalidate even an existing cache.
    fs::create_dir(fixture.upstream.join("backend")).unwrap();
    fs::write(
        fixture.upstream.join("backend/mod.rs"),
        "upstream owns this\n",
    )
    .unwrap();
    let output = fixture.prepare();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot install owned source"));
    assert_eq!(
        fs::read_to_string(fixture.upstream.join("backend/mod.rs")).unwrap(),
        "upstream owns this\n"
    );
    assert!(!fixture.scratch.0.join("work/std-stage/state.json").exists());

    let fixture = Fixture::new();
    let owned = fixture.scratch.0.join("rust/library/backend/mod.rs");
    fs::remove_file(&owned).unwrap();
    std::os::unix::fs::symlink(fixture.upstream.join("Cargo.toml"), owned).unwrap();
    let output = fixture.prepare();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unsupported rust-src file"));
    assert!(!fixture.scratch.0.join("work/std-stage/state.json").exists());
}

#[test]
fn refuses_owned_source_edits_and_stale_upstream_patches() {
    let fixture = Fixture::new();
    fs::write(fixture.scratch.0.join("rust/patches/upstream.patch"), "diff --git a/library/backend/mod.rs b/library/backend/mod.rs\n--- a/library/backend/mod.rs\n+++ b/library/backend/mod.rs\n@@ -1 +1 @@\n-pub fn version() -> u8 { 1 }\n+pub fn version() -> u8 { 9 }\n").unwrap();
    let output = fixture.prepare();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("patch changed owned source"));
    assert!(!fixture.scratch.0.join("work/std-stage/state.json").exists());
    let fixture = Fixture::new();
    fs::write(fixture.scratch.0.join("rust/patches/upstream.patch"), "diff --git a/library/Cargo.toml b/library/Cargo.toml\n--- a/library/Cargo.toml\n+++ b/library/Cargo.toml\n@@ -1 +1 @@\n-obsolete upstream\n+integrated\n").unwrap();
    let output = fixture.prepare();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("patch does not apply"));
    assert!(!fixture.scratch.0.join("work/std-stage/state.json").exists());
}
