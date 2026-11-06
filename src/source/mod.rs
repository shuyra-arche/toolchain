use crate::Result;
use serde::{Deserialize, Serialize};
use std::{path::Path, process::Command};

pub mod patch;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub toolchain: String,
    pub commit: String,
    pub host: String,
    pub target: String,
}

impl Config {
    pub fn read(root: &Path) -> Result<Self> {
        let config: Self =
            toml::from_str(&std::fs::read_to_string(root.join("rust/source.toml"))?)?;
        if config.commit.len() != 40 || !config.commit.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("Rust commit must be a full 40-character revision".into());
        }
        let date = config.toolchain.strip_prefix("nightly-").unwrap_or("");
        if date.len() != 10
            || !date.bytes().enumerate().all(|(i, b)| {
                if i == 4 || i == 7 {
                    b == b'-'
                } else {
                    b.is_ascii_digit()
                }
            })
        {
            return Err("toolchain must be a dated nightly-YYYY-MM-DD".into());
        }
        if config.host != "x86_64-unknown-linux-gnu" || config.target != "riscv64gc-unknown-vanllia"
        {
            return Err("bootstrap currently supports Linux x86_64 -> RV64 Vanllia only".into());
        }
        Ok(config)
    }

    pub fn distribution(&self) -> String {
        format!("{}-{}", self.toolchain, self.host)
    }

    pub fn command(&self, program: &str) -> Command {
        let mut command = Command::new("rustup");
        command.args(["run", &self.distribution(), program]);
        clean_env(&mut command);
        command
    }
}

pub fn clean_env(command: &mut Command) {
    // The selected distribution and target flags must not be replaced by
    // inherited compiler wrappers, target flags or profile overrides.
    for (name, _) in std::env::vars_os() {
        let key = name.to_string_lossy();
        if matches!(
            key.as_ref(),
            "RUSTC"
                | "RUSTC_WRAPPER"
                | "RUSTC_WORKSPACE_WRAPPER"
                | "RUSTFLAGS"
                | "RUSTDOC"
                | "RUSTDOCFLAGS"
                | "RUST_TARGET_PATH"
                | "RUSTUP_TOOLCHAIN"
                | "RUSTC_BOOTSTRAP"
        ) || key.starts_with("CARGO_ENCODED_")
            || key.starts_with("CARGO_TARGET_")
            || key.starts_with("CARGO_PROFILE_")
            || key.starts_with("CARGO_BUILD_")
        {
            command.env_remove(name);
        }
    }
}

#[derive(Serialize)]
pub struct Compiler {
    pub distribution: String,
    pub commit: String,
    pub version: String,
    pub sysroot: String,
    pub source: String,
    pub source_sha256: String,
}

pub fn capture(command: &mut Command) -> Result<String> {
    let output = command.output()?;
    if !output.status.success() {
        return Err(format!(
            "{command:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

pub fn verify(config: &Config) -> Result<Compiler> {
    let version = capture(config.command("rustc").arg("-Vv"))?;
    if !version
        .lines()
        .any(|s| s == format!("commit-hash: {}", config.commit))
        || !version
            .lines()
            .any(|s| s == format!("host: {}", config.host))
    {
        return Err("installed rustc does not match rust/source.toml commit/host".into());
    }
    let sysroot = capture(config.command("rustc").args(["--print", "sysroot"]))?;
    let library = Path::new(&sysroot).join("lib/rustlib/src/rust/library");
    for path in [
        "Cargo.toml",
        "Cargo.lock",
        "core/src/lib.rs",
        "compiler-builtins/Cargo.toml",
    ] {
        if !library.join(path).is_file() {
            return Err(format!("missing paired rust-src file {path}; run prepare").into());
        }
    }
    Ok(Compiler {
        distribution: config.distribution(),
        commit: config.commit.clone(),
        version,
        sysroot,
        source: library.to_string_lossy().into_owned(),
        source_sha256: source_hash(&library)?,
    })
}

pub fn source_hash(root: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    fn visit(root: &Path, directory: &Path, digest: &mut Sha256) -> Result<()> {
        let mut paths = std::fs::read_dir(directory)?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<std::io::Result<Vec<_>>>()?;
        paths.sort();
        for path in paths {
            let metadata = std::fs::symlink_metadata(&path)?;
            if metadata.is_dir() {
                visit(root, &path, digest)?;
            } else if metadata.is_file() {
                let relative = path
                    .strip_prefix(root)?
                    .to_str()
                    .ok_or("non-UTF8 rust-src path")?;
                let bytes = std::fs::read(&path)?;
                digest.update((relative.len() as u64).to_le_bytes());
                digest.update(relative.as_bytes());
                digest.update((bytes.len() as u64).to_le_bytes());
                digest.update(bytes);
            } else {
                return Err(format!("unsupported rust-src file: {}", path.display()).into());
            }
        }
        Ok(())
    }
    let mut digest = Sha256::new();
    visit(root, root, &mut digest)?;
    Ok(format!("{:x}", digest.finalize()))
}

pub fn prepare(root: &Path, config: &Config) -> Result<()> {
    let status = Command::new("rustup")
        .args([
            "toolchain",
            "install",
            "--no-self-update",
            &config.distribution(),
            "--profile",
            "minimal",
            "--component",
            "rust-src",
        ])
        .current_dir(root)
        .status()?;
    if !status.success() {
        return Err("rustup preparation failed".into());
    }
    println!("{}", serde_json::to_string_pretty(&verify(config)?)?);
    Ok(())
}
