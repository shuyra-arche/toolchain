use crate::{Result, elf, source};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Component, Path, PathBuf},
    process::Command,
};

#[derive(Serialize, Deserialize)]
pub struct Stage {
    pub sysroot: PathBuf,
    pub inputs: BTreeMap<String, String>,
    pub library_sha256: String,
    pub dependencies_sha256: String,
}

pub fn series(root: &Path) -> Result<Vec<PathBuf>> {
    let directory = root.join("rust/patches");
    let text = fs::read_to_string(directory.join("series"))?;
    let mut paths = Vec::new();
    for name in text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
    {
        let path = Path::new(name);
        if path.components().count() != 1
            || !matches!(path.components().next(), Some(Component::Normal(_)))
            || path
                .extension()
                .is_none_or(|extension| extension != "patch")
        {
            return Err("patch series entries must be plain .patch filenames".into());
        }
        let path = directory.join(path);
        if paths.contains(&path) || fs::symlink_metadata(&path)?.file_type().is_symlink() {
            return Err("duplicate or symlinked patch in series".into());
        }
        paths.push(path);
    }
    if paths.is_empty() {
        return Err("empty patch series".into());
    }
    Ok(paths)
}

fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &to.join(entry.file_name()))?;
        } else if entry.file_type()?.is_file() {
            fs::copy(entry.path(), to.join(entry.file_name()))?;
        } else {
            return Err(format!("unsupported source file: {}", entry.path().display()).into());
        }
    }
    Ok(())
}

pub fn prepare(root: &Path, config: &source::Config, compiler: &source::Compiler) -> Result<Stage> {
    let patches = series(root)?;
    let mut inputs = BTreeMap::from([("rust-src".into(), compiler.source_sha256.clone())]);
    for path in [
        root.join("rust/source.toml"),
        root.join("rust/patches/series"),
        root.join("tests/target/startup/Cargo.toml"),
        root.join("tests/target/startup/Cargo.lock"),
    ]
    .into_iter()
    .chain(patches.iter().cloned())
    {
        inputs.insert(
            path.strip_prefix(root)?.to_string_lossy().into_owned(),
            elf::hash(&fs::read(path)?),
        );
    }
    let directory = root.join("work/std-stage");
    let sysroot = directory.join("sysroot");
    let sources = sysroot.join("lib/rustlib/src/rust");
    let state = directory.join("state.json");
    if let Ok(bytes) = fs::read(&state)
        && let Ok(stage) = serde_json::from_slice::<Stage>(&bytes)
        && stage.inputs == inputs
        && stage.sysroot == sysroot
        && Some(&stage.library_sha256)
            == source::source_hash(&sources.join("library")).ok().as_ref()
        && Some(&stage.dependencies_sha256)
            == source::source_hash(&sources.join("dependencies"))
                .ok()
                .as_ref()
    {
        return Ok(stage);
    }

    // Only this generated directory is replaced; installed rustup files are
    // immutable inputs. A partially prepared tree never has a state receipt.
    if directory.exists() {
        fs::remove_dir_all(&directory)?;
    }
    fs::create_dir_all(sysroot.join("bin"))?;
    fs::create_dir_all(&sources)?;
    let original = Path::new(&compiler.sysroot);
    for name in ["rustc", "rustdoc"] {
        fs::copy(
            original.join("bin").join(name),
            sysroot.join("bin").join(name),
        )?;
    }
    // rustc locates its sysroot through librustc_driver, so that library must be
    // copied as well. Other immutable compiler/host libraries can be linked.
    for entry in fs::read_dir(original.join("lib"))? {
        let entry = entry?;
        let name = entry.file_name();
        if name == "rustlib" {
            continue;
        }
        let target = sysroot.join("lib").join(&name);
        if name.to_string_lossy().starts_with("librustc_driver") {
            fs::copy(entry.path(), target)?;
        } else {
            std::os::unix::fs::symlink(entry.path(), target)?;
        }
    }
    for entry in fs::read_dir(original.join("lib/rustlib"))? {
        let entry = entry?;
        if entry.file_name() != "src" {
            std::os::unix::fs::symlink(
                entry.path(),
                sysroot.join("lib/rustlib").join(entry.file_name()),
            )?;
        }
    }
    copy_tree(Path::new(&compiler.source), &sources.join("library"))?;
    if source::source_hash(&sources.join("library"))? != compiler.source_sha256 {
        return Err("rust-src changed while preparing its private copy".into());
    }
    source::capture(
        config
            .command("cargo")
            .current_dir(root)
            .args(["vendor", "--locked", "--manifest-path"])
            .arg(root.join("tests/target/startup/Cargo.toml"))
            .arg(sources.join("dependencies")),
    )?;
    source::capture(
        Command::new("git")
            .arg("-C")
            .arg(&sources)
            .args(["init", "-q"]),
    )?;
    for patch in patches {
        source::capture(
            Command::new("git")
                .arg("-C")
                .arg(&sources)
                .args(["apply", "--check", "--whitespace=error-all"])
                .arg(&patch),
        )?;
        source::capture(
            Command::new("git")
                .arg("-C")
                .arg(&sources)
                .args(["apply", "--whitespace=error-all"])
                .arg(&patch),
        )?;
    }
    let stage = Stage {
        sysroot,
        inputs,
        library_sha256: source::source_hash(&sources.join("library"))?,
        dependencies_sha256: source::source_hash(&sources.join("dependencies"))?,
    };
    fs::write(state, serde_json::to_vec_pretty(&stage)?)?;
    Ok(stage)
}

impl Stage {
    pub fn cargo(&self, compiler: &source::Compiler) -> Command {
        // rustup run prepends the original dynamic-library path, causing the
        // copied rustc to discover the wrong source tree. Invoke Cargo directly.
        let mut command = Command::new(Path::new(&compiler.sysroot).join("bin/cargo"));
        source::clean_env(&mut command);
        command
            .env("RUSTC", self.sysroot.join("bin/rustc"))
            .env("RUSTDOC", self.sysroot.join("bin/rustdoc"))
            .env("LD_LIBRARY_PATH", self.sysroot.join("lib"));
        command
    }
}
