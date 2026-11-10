//! Direct Kernel startup and std probes. System's Loader is tested separately.
use capsule::Format;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{ffi::OsString, fs, path::PathBuf, process::Command};
mod capsule;
mod process;

fn input(name: &str) -> PathBuf {
    PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("set {name}")))
        .canonicalize()
        .unwrap()
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

struct Probe<'a> {
    name: &'a str,
    elf: Vec<u8>,
    task: u32,
    reason: u32,
    note: &'a str,
}

impl<'a> Probe<'a> {
    fn new(
        name: &'a str,
        path: &Value,
        hash: &Value,
        task: u32,
        reason: u32,
        note: &'a str,
    ) -> Self {
        let path = path.as_str().expect("artifact path");
        let elf = fs::read(path).unwrap();
        let expected = hash.as_str().expect("artifact hash");
        assert_eq!(digest(&elf), expected, "stale Toolchain artifact: {name}");
        // Compare the CLI report to the exact bytes that will be booted.
        let inspection = Command::new(env!("CARGO_BIN_EXE_shuyra-toolchain"))
            .args(["inspect", path])
            .output()
            .unwrap();
        assert!(inspection.status.success(), "{inspection:?}");
        let report: Value = serde_json::from_slice(&inspection.stdout).unwrap();
        assert_eq!(report["sha256"], expected);
        Self {
            name,
            elf,
            task,
            reason,
            note,
        }
    }
}

struct Run {
    format: Format,
    kernel: PathBuf,
    firmware: PathBuf,
    qemu: OsString,
    evidence: PathBuf,
    report: Value,
    results: Vec<Value>,
}

impl Run {
    fn new(format: Format, receipt: Value, receipt_bytes: &[u8], kernel_revision: &str) -> Self {
        assert_eq!(receipt["schema"], 1);
        assert_eq!(receipt["target_build"], "passed");
        let kernel = input("SHUYRA_KERNEL_ELF");
        let firmware = input("SHUYRA_OPENSBI");
        let firmware_hash = digest(&fs::read(&firmware).unwrap());
        assert_eq!(
            firmware_hash,
            std::env::var("SHUYRA_FIRMWARE_SHA256").expect("pin firmware SHA-256")
        );
        let qemu = std::env::var_os("SHUYRA_QEMU").unwrap_or_else(|| "qemu-system-riscv64".into());
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let evidence = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/qemu")
            .join(format!("{stamp}-{}", std::process::id()));
        fs::create_dir_all(&evidence).unwrap();
        eprintln!("Toolchain bootstrap evidence: {}", evidence.display());
        let version = Command::new(&qemu).arg("--version").output().unwrap();
        assert!(version.status.success());
        let report = json!({
            "scope": "direct Kernel bootstrap; System loader not exercised",
            "toolchain_receipt_sha256": digest(receipt_bytes), "toolchain_receipt": receipt,
            "kernel_revision": kernel_revision, "kernel_sha256": digest(&fs::read(&kernel).unwrap()),
            "firmware_sha256": firmware_hash, "qemu": String::from_utf8_lossy(&version.stdout),
        });
        Self {
            format,
            kernel,
            firmware,
            qemu,
            evidence,
            report,
            results: Vec::new(),
        }
    }

    fn boot(&mut self, probe: &Probe<'_>, harts: u32) {
        let Probe {
            name,
            elf,
            task,
            reason,
            note,
        } = probe;
        let initrd = capsule::initrd(elf, self.format);
        let directory = self.evidence.join(format!("{name}-{harts}-harts"));
        fs::create_dir_all(&directory).unwrap();
        let initrd_path = directory.join("initrd.bin");
        fs::write(&initrd_path, &initrd).unwrap();
        let result = process::run(
            Command::new(&self.qemu)
                .args(["-machine", "virt", "-m", "256M", "-nographic", "-smp"])
                .arg(harts.to_string())
                .arg("-bios")
                .arg(&self.firmware)
                .arg("-kernel")
                .arg(&self.kernel)
                .arg("-initrd")
                .arg(&initrd_path),
            &directory,
            std::time::Duration::from_secs(20),
        );
        let uart = fs::read_to_string(directory.join("stdout.log"))
            .unwrap_or_default()
            .replace('\r', "");
        let expected = format!("exit tid={task} reason={reason:#x} note: {note}");
        let expected_cpus = format!("cpu: admitted={harts} boot=");
        let harts_admitted = matches!(self.format, Format::Legacy)
            || uart.lines().any(|line| line.starts_with(&expected_cpus));
        let passed = result.is_ok()
            && harts_admitted
            && uart.lines().filter(|line| *line == expected).count() == 1
            && uart
                .lines()
                .any(|line| line == "task: all tasks exited, system halted")
            && !uart.contains("boot failed");
        self.results.push(json!({
            "fixture": name, "harts": harts, "elf_sha256": digest(elf),
            "initrd_sha256": digest(&initrd), "expected_exit": expected,
            "passed": passed,
        }));
        self.report["scenarios"] = json!(self.results);
        fs::write(
            self.evidence.join("result.json"),
            serde_json::to_vec_pretty(&self.report).unwrap(),
        )
        .unwrap();
        assert!(
            passed,
            "{name}: {result:?}; harts admitted: {harts_admitted}; expected {expected:?}; evidence: {}\n{uart}",
            directory.display()
        );
    }
}

fn kernel_revision() -> String {
    let revision = std::env::var("SHUYRA_KERNEL_REVISION")
        .expect("record the Kernel source revision used to build the ELF");
    assert!(revision.len() == 40 && revision.bytes().all(|b| b.is_ascii_hexdigit()));
    revision
}

#[test]
#[ignore = "requires ABI v7 startup receipt, pinned Kernel/OpenSBI and QEMU; see tests/qemu/README.md"]
fn startup_abi_v7() {
    let receipt_bytes = fs::read(input("SHUYRA_TOOLCHAIN_RECEIPT")).unwrap();
    let receipt: Value = serde_json::from_slice(&receipt_bytes).unwrap();
    let kernel_revision = kernel_revision();
    let manifest = include_str!("../target/startup/Cargo.toml");
    let manifest_config: toml::Value = toml::from_str(manifest).unwrap();
    assert_eq!(
        manifest_config["dependencies"]["kernel-api"]["rev"].as_str(),
        Some(kernel_revision.as_str()),
        "boot with the Kernel revision pinned by the startup fixture"
    );
    for (path, contents) in [
        ("tests/target/startup/Cargo.toml", manifest.as_bytes()),
        (
            "tests/target/startup/Cargo.lock",
            include_bytes!("../target/startup/Cargo.lock").as_slice(),
        ),
        (
            "tests/target/startup/src/main.rs",
            include_bytes!("../target/startup/src/main.rs").as_slice(),
        ),
        (
            "tests/target/startup/link.ld",
            include_bytes!("../target/startup/link.ld").as_slice(),
        ),
    ] {
        assert_eq!(
            receipt["inputs_sha256"][path],
            digest(contents),
            "stale startup input: {path}"
        );
    }
    let probe = Probe::new(
        "toolchain-startup",
        &receipt["artifact"],
        &receipt["elf"]["sha256"],
        1,
        0,
        "toolchain: startup passed",
    );
    let mut run = Run::new(Format::V7, receipt, &receipt_bytes, &kernel_revision);
    for harts in [1, 2] {
        run.boot(&probe, harts);
    }
}

#[test]
#[ignore = "requires Toolchain std receipt, pinned Kernel/OpenSBI and QEMU; see tests/qemu/README.md"]
fn std_main_allocator_and_termination() {
    let receipt_bytes = fs::read(input("SHUYRA_TOOLCHAIN_RECEIPT")).unwrap();
    let receipt: Value = serde_json::from_slice(&receipt_bytes).unwrap();
    let mut run = Run::new(Format::Legacy, receipt, &receipt_bytes, &kernel_revision());
    for harts in [1, 2] {
        for (name, task, reason, note) in [
            ("toolchain-std", 1, 0, "toolchain: std main returned"),
            ("std-exit", 1, 23, "toolchain: std process exit"),
            ("std-panic", 1, 0xffffff01, "toolchain: std abort"),
            ("std-oom", 1, 0xffffff01, "toolchain: std abort"),
            ("std-threads", 1, 0, "toolchain: std main returned"),
            ("std-waits", 1, 0, "toolchain: std main returned"),
            ("std-thread-exit", 2, 23, "toolchain: std process exit"),
            ("std-thread-panic", 2, 0xffffff01, "toolchain: std abort"),
            ("std-tls-main", 1, 0xffffff01, "toolchain: std abort"),
            ("std-thread-fault", 1, 0xffffff01, "toolchain: std abort"),
        ] {
            let artifact = &run.report["toolchain_receipt"]["artifacts"][name];
            let probe = Probe::new(
                name,
                &artifact["path"],
                &artifact["elf"]["sha256"],
                task,
                reason,
                note,
            );
            run.boot(&probe, harts);
        }
    }
}

#[test]
fn runner_rejects_failure_and_reaps_timeout() {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};
    let directory = std::env::temp_dir().join(format!(
        "toolchain-qemu-runner-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    assert!(
        process::run(
            &mut Command::new("false"),
            &directory,
            Duration::from_secs(1)
        )
        .is_err()
    );
    let error = process::run(
        Command::new("sleep").arg("30"),
        &directory,
        Duration::from_millis(30),
    )
    .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
    let record: serde_json::Value =
        serde_json::from_slice(&fs::read(directory.join("execution.json")).unwrap()).unwrap();
    let pid = record["pid"].as_u64().unwrap();
    assert!(
        !std::path::Path::new(&format!("/proc/{pid}")).exists(),
        "timed-out child is still present"
    );
    fs::remove_dir_all(directory).unwrap();
}
