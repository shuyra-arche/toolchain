use crate::{Result, elf, source};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

pub fn invalidate(root: &Path) -> Result<()> {
    match fs::remove_file(root.join("build/std.json")) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

pub fn build(root: &Path, config: &source::Config) -> Result<()> {
    let compiler = source::verify(config)?;
    let stage = source::patch::prepare(root, config, &compiler)?;
    let target_cache = root.join("build/std");
    let cache_stamp = target_cache.join("prepared-dependencies");
    // The relocated dependency paths are stable across preparations. Cargo
    // can reuse a proc macro if a newly vendored source has an older mtime.
    let stamp = format!("{}\n{}\n", compiler.commit, stage.dependencies_sha256);
    if fs::read_to_string(&cache_stamp).ok().as_deref() != Some(&stamp) {
        match fs::remove_dir_all(&target_cache) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        fs::create_dir_all(&target_cache)?;
        fs::write(cache_stamp, stamp)?;
    }
    let fixture = root.join("tests/target/std");
    let target = root
        .join("rust/targets")
        .join(format!("{}.json", config.target));
    let fixture_hash = source::source_hash(&fixture)?;
    let target_hash = elf::hash(&fs::read(&target)?);
    let mut artifacts = BTreeMap::new();
    let mut libraries = BTreeSet::new();
    for name in [
        "toolchain-std",
        "std-panic",
        "std-exit",
        "std-oom",
        "std-threads",
        "std-thread-exit",
        "std-thread-panic",
        "std-tls-main",
        "std-thread-fault",
        "std-waits",
        "std-parallelism",
    ] {
        let mut command = stage.cargo(&compiler);
        command
            .current_dir(root)
            .args([
                "rustc",
                "--locked",
                "--release",
                "-Zjson-target-spec",
                "-Zbuild-std=std,panic_abort",
                "-Zbuild-std-features=compiler-builtins-mem",
                "--message-format=json-render-diagnostics",
                "--manifest-path",
            ])
            .arg(fixture.join("Cargo.toml"))
            .arg("--target")
            .arg(&target)
            .arg("--target-dir")
            .arg(&target_cache)
            .args(["--bin", name, "--", "-C"])
            .arg(format!("link-arg=-T{}", fixture.join("link.ld").display()));
        println!("building std fixture: {name}");
        let messages = source::capture(&mut command)?;
        let target_directory = target_cache.join(&config.target);
        for line in messages.lines() {
            let message: serde_json::Value = serde_json::from_str(line)?;
            if message["reason"] == "compiler-artifact"
                && message["target"]["kind"]
                    .as_array()
                    .is_some_and(|kinds| kinds.iter().any(|kind| kind == "lib" || kind == "rlib"))
                && message["filenames"].as_array().is_some_and(|files| {
                    files
                        .iter()
                        .filter_map(|file| file.as_str())
                        .any(|file| Path::new(file).starts_with(&target_directory))
                })
            {
                libraries.insert(
                    message["target"]["name"]
                        .as_str()
                        .ok_or("missing library name")?
                        .to_owned(),
                );
            }
        }
        let path = root
            .join("build/std")
            .join(&config.target)
            .join("release")
            .join(name);
        let report = elf::inspect(&fs::read(&path)?)?;
        artifacts.insert(name, serde_json::json!({ "path": path, "elf": report }));
    }
    if libraries.contains("libc") {
        return Err("unexpected target libc dependency".into());
    }
    if ![
        "core",
        "alloc",
        "std",
        "kernel_api",
        "kernel_client",
        "talc",
    ]
    .iter()
    .all(|name| libraries.contains(*name))
    {
        return Err("missing expected target library build evidence".into());
    }
    if fixture_hash != source::source_hash(&fixture)?
        || target_hash != elf::hash(&fs::read(&target)?)
    {
        return Err("std build inputs changed during compilation".into());
    }
    let sources = stage.sysroot.join("lib/rustlib/src/rust");
    if stage.library_sha256 != source::source_hash(&sources.join("library"))?
        || stage.dependencies_sha256 != source::source_hash(&sources.join("dependencies"))?
    {
        return Err("prepared sources changed during compilation".into());
    }
    for (path, hash) in &stage.inputs {
        if path != "rust-src"
            && path != "compiler"
            && source::input_hash(&root.join(path))? != *hash
        {
            return Err(format!("prepared source input changed during compilation: {path}").into());
        }
    }
    let after = source::verify(config)?;
    if serde_json::to_vec(&after)? != serde_json::to_vec(&compiler)? {
        return Err("compiler or rust-src changed during std build".into());
    }
    let receipt = serde_json::json!({"schema": 1, "compiler": compiler, "stage": stage,
        "fixture_sha256": fixture_hash, "target_sha256": target_hash, "artifacts": artifacts, "target_libraries": libraries,
        "target_build": "passed", "guest_execution": "not_run"});
    let temporary = root.join("build/std.json.tmp");
    fs::write(&temporary, serde_json::to_vec_pretty(&receipt)?)?;
    fs::rename(temporary, root.join("build/std.json"))?;
    println!(
        "std fixtures built and inspected; receipt: build/std.json (guest execution: not_run)"
    );
    Ok(())
}
