//! Direct Kernel bootstrap probes for Toolchain's threaded std backend.
//! This deliberately does not claim to exercise System's ELF loader/lifecycle.
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf, process::Command};
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

#[test]
#[ignore = "requires Toolchain std receipt, pinned Kernel/OpenSBI and QEMU; see tests/qemu/README.md"]
fn std_main_allocator_and_termination() {
    let receipt_path = input("SHUYRA_TOOLCHAIN_RECEIPT");
    let receipt_bytes = fs::read(&receipt_path).unwrap();
    let receipt: serde_json::Value = serde_json::from_slice(&receipt_bytes).unwrap();
    assert_eq!(receipt["schema"], 1);
    assert_eq!(receipt["target_build"], "passed");
    let kernel = input("SHUYRA_KERNEL_ELF");
    let firmware = input("SHUYRA_OPENSBI");
    let firmware_hash = digest(&fs::read(&firmware).unwrap());
    assert_eq!(
        firmware_hash,
        std::env::var("SHUYRA_FIRMWARE_SHA256").expect("pin firmware SHA-256")
    );
    let kernel_revision = std::env::var("SHUYRA_KERNEL_REVISION")
        .expect("record the Kernel source revision used to build the ELF");
    assert!(kernel_revision.len() == 40 && kernel_revision.bytes().all(|b| b.is_ascii_hexdigit()));
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
    let mut results = Vec::new();
    for harts in [1, 2] {
        for (name, tid, reason, note) in [
            ("toolchain-std", 1, "0x0", "toolchain: std main returned"),
            ("std-exit", 1, "0x17", "toolchain: std process exit"),
            ("std-panic", 1, "0xffffff01", "toolchain: std abort"),
            ("std-oom", 1, "0xffffff01", "toolchain: std abort"),
            ("std-threads", 1, "0x0", "toolchain: std main returned"),
            ("std-thread-exit", 2, "0x17", "toolchain: std process exit"),
            ("std-thread-panic", 2, "0xffffff01", "toolchain: std abort"),
            ("std-tls-main", 1, "0xffffff01", "toolchain: std abort"),
            ("std-thread-fault", 1, "0xffffff01", "toolchain: std abort"),
        ] {
            let artifact = &receipt["artifacts"][name];
            let elf = fs::read(artifact["path"].as_str().expect("artifact path")).unwrap();
            let hash = digest(&elf);
            assert_eq!(
                hash,
                artifact["elf"]["sha256"].as_str().expect("artifact hash"),
                "stale Toolchain artifact: {name}"
            );
            // Inspect through the public CLI, then compare its fingerprint to the
            // exact bytes being encoded so a changed file cannot pass inspection.
            let inspection = Command::new(env!("CARGO_BIN_EXE_shuyra-toolchain"))
                .arg("inspect")
                .arg(artifact["path"].as_str().unwrap())
                .output()
                .unwrap();
            assert!(inspection.status.success(), "{inspection:?}");
            let report: serde_json::Value = serde_json::from_slice(&inspection.stdout).unwrap();
            assert_eq!(report["sha256"], hash);
            let capsule = capsule::encode(&elf);
            let mut initrd = vec![0; 4096];
            initrd[..4].copy_from_slice(&4096u32.to_le_bytes());
            initrd[4..8].copy_from_slice(&u32::try_from(capsule.len()).unwrap().to_le_bytes());
            initrd.extend_from_slice(&capsule);
            let directory = evidence.join(format!("{name}-{harts}-harts"));
            fs::create_dir_all(&directory).unwrap();
            let initrd_path = directory.join("initrd.bin");
            fs::write(&initrd_path, &initrd).unwrap();
            let result = process::run(
                Command::new(&qemu)
                    .args(["-machine", "virt", "-m", "256M", "-nographic", "-smp"])
                    .arg(harts.to_string())
                    .arg("-bios")
                    .arg(&firmware)
                    .arg("-kernel")
                    .arg(&kernel)
                    .arg("-initrd")
                    .arg(&initrd_path),
                &directory,
                std::time::Duration::from_secs(20),
            );
            let uart = fs::read_to_string(directory.join("stdout.log"))
                .unwrap_or_default()
                .replace('\r', "");
            let expected = format!("exit tid={tid} reason={reason} note: {note}");
            let passed = result.is_ok()
                && uart.lines().filter(|line| *line == expected).count() == 1
                && uart
                    .lines()
                    .any(|line| line == "task: all tasks exited, system halted")
                && !uart.contains("boot failed");
            results.push(
                serde_json::json!({"fixture": name, "harts": harts, "elf_sha256": hash,
            "initrd_sha256": digest(&initrd), "expected_exit": expected, "passed": passed}),
            );
            fs::write(evidence.join("result.json"), serde_json::to_vec_pretty(&serde_json::json!({
            "scope": "direct Kernel bootstrap; System loader not exercised",
            "toolchain_receipt_sha256": digest(&receipt_bytes), "toolchain_receipt": receipt,
            "kernel_revision": kernel_revision, "kernel_sha256": digest(&fs::read(&kernel).unwrap()),
            "firmware_sha256": firmware_hash, "qemu": String::from_utf8_lossy(&version.stdout),
            "scenarios": results,
        })).unwrap()).unwrap();
            assert!(
                passed,
                "{name}: {result:?}; expected {expected:?}; evidence: {}\n{uart}",
                directory.display()
            );
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
