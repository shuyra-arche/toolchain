use crate::{Result, elf, source};
use std::{collections::BTreeMap, path::Path};

pub mod standard;

pub fn invalidate(root: &Path) -> Result<()> {
    // A failed rebuild must not leave an earlier success receipt behind.
    match std::fs::remove_file(root.join("build/bootstrap.json")) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

pub fn build(root: &Path, config: &source::Config) -> Result<()> {
    let directory = root.join("build");
    std::fs::create_dir_all(&directory)?;
    let receipt_path = directory.join("bootstrap.json");
    let compiler = source::verify(config)?;
    let target = root
        .join("rust/targets")
        .join(format!("{}.json", config.target));
    let fixture = root.join("tests/target/startup");
    let mut inputs = BTreeMap::new();
    for path in [
        "rust/source.toml",
        "rust/targets/riscv64gc-unknown-vanllia.json",
        "tests/target/startup/Cargo.toml",
        "tests/target/startup/Cargo.lock",
        "tests/target/startup/src/main.rs",
        "tests/target/startup/link.ld",
    ] {
        inputs.insert(path, elf::hash(&std::fs::read(root.join(path))?));
    }
    let status = config
        .command("cargo")
        .current_dir(root)
        .args([
            "rustc",
            "--locked",
            "--release",
            "-Zjson-target-spec",
            "-Zbuild-std=core,compiler_builtins",
            "--manifest-path",
        ])
        .arg(fixture.join("Cargo.toml"))
        .arg("--target")
        .arg(&target)
        .arg("--target-dir")
        .arg(directory.join("cargo"))
        .args(["--bin", "toolchain-startup", "--", "-C"])
        .arg(format!("link-arg=-T{}", fixture.join("link.ld").display()))
        .status()?;
    if !status.success() {
        return Err("startup target build failed; no success receipt published".into());
    }
    let artifact = directory
        .join("cargo")
        .join(&config.target)
        .join("release/toolchain-startup");
    let report = elf::inspect(&std::fs::read(&artifact)?)?;
    for (path, before) in &inputs {
        if &elf::hash(&std::fs::read(root.join(path))?) != before {
            return Err(format!("input changed during build: {path}").into());
        }
    }
    if serde_json::to_vec(&source::verify(config)?)? != serde_json::to_vec(&compiler)? {
        return Err("compiler or rust-src changed during build".into());
    }
    let receipt = serde_json::json!({
        "schema": 1,
        "compiler": compiler,
        "inputs_sha256": inputs,
        "artifact": artifact,
        "elf": report,
        "target_build": "passed",
        "guest_execution": "not_run",
    });
    let temporary = directory.join("bootstrap.json.tmp");
    std::fs::write(&temporary, serde_json::to_vec_pretty(&receipt)?)?;
    std::fs::rename(temporary, &receipt_path)?;
    println!(
        "target build and ELF inspection passed: {}",
        artifact.display()
    );
    println!(
        "receipt: {} (guest execution: not_run)",
        receipt_path.display()
    );
    Ok(())
}
