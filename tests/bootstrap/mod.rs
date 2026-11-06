use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "toolchain-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn inspect(&self, bytes: &[u8]) -> Output {
        let path = self.0.join("input.elf");
        fs::write(&path, bytes).unwrap();
        cli().arg("inspect").arg(path).output().unwrap()
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_shuyra-toolchain"))
}
fn put(bytes: &mut [u8], at: usize, width: usize, value: u64) {
    bytes[at..at + width].copy_from_slice(&value.to_le_bytes()[..width]);
}

// A minimal independent ELF producer: RX code and RW zero-fill on separate
// pages. Tests exercise the public inspector through its CLI, not private code.
fn executable() -> Vec<u8> {
    let mut bytes = vec![0; 0x1002];
    bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
    for (offset, width, value) in [
        (16, 2, 2),
        (18, 2, 243),
        (20, 4, 1),
        (24, 8, 0x10000),
        (32, 8, 64),
        (48, 4, 5),
        (52, 2, 64),
        (54, 2, 56),
        (56, 2, 2),
        (64, 4, 1),
        (68, 4, 5),
        (72, 8, 0x1000),
        (80, 8, 0x10000),
        (96, 8, 2),
        (104, 8, 2),
        (112, 8, 4096),
        (120, 4, 1),
        (124, 4, 6),
        (136, 8, 0x11000),
        (160, 8, 4096),
        (168, 8, 4096),
    ] {
        put(&mut bytes, offset, width, value);
    }
    bytes[0x1000..].copy_from_slice(&[1, 0]); // c.nop
    bytes
}

#[test]
fn accepts_static_executable_and_zero_fill() {
    let result = Scratch::new().inspect(&executable());
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["entry"], 0x10000);
    assert_eq!(report["load_segments"], 2);
    assert_eq!(report["sha256"].as_str().unwrap().len(), 64);
}

#[test]
fn rejects_unsafe_or_unsupported_images() {
    let scratch = Scratch::new();
    for (offset, width, value, message) in [
        (16, 2, 3, "static ELF64"),
        (18, 2, 62, "RISC-V"),
        (48, 4, 1, "LP64D"),
        (56, 2, 65535, "header table"),
        (64, 4, 2, "dynamic linking"),
        (64, 4, 3, "dynamic linking"),
        (120, 4, 7, "TLS"),
        (68, 4, 7, "writable+executable"),
        (68, 4, 1, "readable"),
        (96, 8, 3, "file range"),
        (72, 8, u64::MAX, "overflow"),
        (112, 8, 3, "alignment"),
        (136, 8, 0x10000, "overlap"),
        (136, 8, 0, "null page"),
        (136, 8, u64::MAX - 4095, "overflow"),
        (24, 8, 0x10001, "entry"),
        (24, 8, 0x11000, "entry"),
    ] {
        let mut bytes = executable();
        put(&mut bytes, offset, width, value);
        let result = scratch.inspect(&bytes);
        assert!(!result.status.success(), "accepted mutation at {offset}");
        assert!(
            String::from_utf8_lossy(&result.stderr).contains(message),
            "{result:?}"
        );
    }
    for len in [0, 6, 20, 63, 119, 175, 4097] {
        assert!(!scratch.inspect(&executable()[..len]).status.success());
    }
}

#[test]
fn rejects_mutable_source_pins_before_preparing() {
    let scratch = Scratch::new();
    fs::create_dir(scratch.0.join("rust")).unwrap();
    fs::write(scratch.0.join("rust/source.toml"),
        "toolchain = 'nightly'\ncommit = 'main'\nhost = 'x86_64-unknown-linux-gnu'\ntarget = 'riscv64gc-unknown-vanllia'\n").unwrap();
    let result = cli()
        .args(["prepare", "--root"])
        .arg(&scratch.0)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("full 40-character revision"));
    assert!(!scratch.0.join("build").exists());
}

#[test]
fn help_and_unknown_command_do_not_require_a_repository() {
    let scratch = Scratch::new();
    assert!(
        cli()
            .arg("--help")
            .current_dir(&scratch.0)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        !cli()
            .arg("install")
            .current_dir(&scratch.0)
            .status()
            .unwrap()
            .success()
    );
}

#[test]
fn failed_build_invalidates_previous_success_receipt() {
    let scratch = Scratch::new();
    fs::create_dir(scratch.0.join("build")).unwrap();
    let receipt = scratch.0.join("build/bootstrap.json");
    fs::write(&receipt, b"previous success").unwrap();
    // Missing source configuration must fail before invoking a compiler.
    let result = cli()
        .args(["build", "--root"])
        .arg(&scratch.0)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(!receipt.exists());
}
