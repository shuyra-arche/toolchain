use std::{error::Error, fs, path::Path, process::Command};

pub(super) fn verify(uart: &str, elf: &str, directory: &Path) -> Result<(), Box<dyn Error>> {
    for (thread, caller) in [
        ("main", "std_backtrace_main_frame"),
        ("worker", "std_backtrace_worker_frame"),
    ] {
        let begin = format!("BEGIN std-backtrace {thread}\n");
        let end = format!("END std-backtrace {thread}");
        let (_, tail) = uart.split_once(&begin).ok_or(format!("missing {begin}"))?;
        let (trace, _) = tail.split_once(&end).ok_or(format!("missing {end}"))?;
        let mut addresses = Vec::new();
        for line in trace.lines() {
            let Some((_, frame)) = line.split_once(": ") else {
                continue;
            };
            let (address, _) = frame.trim_start().split_once(" - ").ok_or(line)?;
            let pc = usize::from_str_radix(address.strip_prefix("0x").ok_or(line)?, 16)?;
            // Look up the call instruction rather than the instruction after it.
            addresses.push(format!("{:#x}", pc.checked_sub(1).ok_or(line)?));
        }
        let output = Command::new("addr2line")
            .args(["-f", "-C", "-e", elf])
            .args(&addresses)
            .output()?;
        if !output.status.success() {
            return Err(String::from_utf8(output.stderr)?.into());
        }
        let symbols = String::from_utf8(output.stdout)?;
        fs::write(directory.join(format!("{thread}-backtrace.txt")), &symbols)?;
        let mut functions = symbols.lines().step_by(2);
        if !functions.any(|name| name == "std_backtrace_leaf") || functions.next() != Some(caller) {
            return Err(format!("unexpected {thread} call chain:\n{symbols}").into());
        }
    }
    Ok(())
}
