mod build;
mod elf;
mod source;

use std::{path::PathBuf, process::ExitCode};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("toolchain: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let command = args.next().unwrap_or_default();
    if command == "--help" || command == "help" || command.is_empty() {
        println!(
            "usage: shuyra-toolchain prepare|build|prepare-std|build-std [--root DIRECTORY]\n       shuyra-toolchain inspect ELF\n\nprepare installs and verifies the selected rustc/rust-src pair\nbuild rebuilds core/compiler_builtins and validates the startup ELF\nprepare-std applies the patch series in an isolated source tree\nbuild-std builds ordinary main/allocator, panic, exit and OOM fixtures\ninspect checks the ELF contract (does not execute it)"
        );
        return Ok(());
    }
    if command == "inspect" {
        let path = args.next().ok_or("inspect requires an ELF path")?;
        if args.next().is_some() {
            return Err("unexpected argument".into());
        }
        let report = elf::inspect(&std::fs::read(path)?)?;
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    if command != "prepare"
        && command != "build"
        && command != "prepare-std"
        && command != "build-std"
    {
        return Err("unknown command; use --help".into());
    }
    let root = match args.next() {
        None => std::env::current_dir()?,
        Some(flag) if flag == "--root" => {
            PathBuf::from(args.next().ok_or("--root requires a directory")?)
        }
        Some(_) => return Err("unexpected argument; use --help".into()),
    };
    if args.next().is_some() {
        return Err("unexpected argument".into());
    }
    let root = root.canonicalize()?;
    if command == "build" {
        build::invalidate(&root)?;
    }
    if command == "build-std" {
        build::standard::invalidate(&root)?;
    }
    if command == "build-std" || command == "prepare-std" {
        source::patch::series(&root)?;
    }
    let config = source::Config::read(&root)?;
    if command == "prepare" {
        source::prepare(&root, &config)?;
    } else if command == "prepare-std" {
        let compiler = source::verify(&config)?;
        let stage = source::patch::prepare(&root, &config, &compiler)?;
        println!("{}", serde_json::to_string_pretty(&stage)?);
    } else if command == "build-std" {
        build::standard::build(&root, &config)?;
    } else {
        build::build(&root, &config)?;
    }
    Ok(())
}
