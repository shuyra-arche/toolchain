//! A bounded native QEMU child; file output cannot deadlock on full pipes.
use std::{
    fs::{self, File},
    io,
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

struct Owned(Child);
impl Drop for Owned {
    fn drop(&mut self) {
        // Also covers evidence-write, wait and assertion/unwind failures.
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub fn run(command: &mut Command, evidence: &Path, timeout: Duration) -> io::Result<()> {
    fs::create_dir_all(evidence)?;
    let description = format!("{command:?}");
    command
        .stdin(Stdio::null())
        .stdout(File::create(evidence.join("stdout.log"))?)
        .stderr(File::create(evidence.join("stderr.log"))?);
    let mut child = Owned(command.spawn()?);
    fs::write(
        evidence.join("execution.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "pid": child.0.id(), "command": description, "timeout_ms": timeout.as_millis(),
        }))?,
    )?;
    let started = Instant::now();
    let result = loop {
        if let Some(status) = child.0.try_wait()? {
            break if status.success() {
                Ok(())
            } else {
                Err(io::Error::other(format!("QEMU exited: {status}")))
            };
        }
        if started.elapsed() >= timeout {
            child.0.kill()?;
            child.0.wait()?;
            break Err(io::Error::new(io::ErrorKind::TimedOut, "QEMU timed out"));
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    fs::write(
        evidence.join("execution-result.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "success": result.is_ok(), "result": format!("{result:?}"), "elapsed_ms": started.elapsed().as_millis(),
        }))?,
    )?;
    result
}
