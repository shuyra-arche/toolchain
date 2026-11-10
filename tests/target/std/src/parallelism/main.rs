use std::{process::ExitCode, thread};

fn main() -> ExitCode {
    let count = thread::available_parallelism().map_or(0, |count| count.get());
    ExitCode::from(u8::try_from(count).unwrap())
}
