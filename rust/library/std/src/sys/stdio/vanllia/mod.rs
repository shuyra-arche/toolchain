//! Console services have not been bound. Never pretend to consume output.
use crate::io::{self, Read, Write};
pub struct Stdin;
pub struct Stdout;
pub type Stderr = Stdout;
impl Stdin {
    pub const fn new() -> Self {
        Self
    }
}
impl Stdout {
    pub const fn new() -> Self {
        Self
    }
}
impl Read for Stdin {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::UNSUPPORTED_PLATFORM)
    }
}
impl Write for Stdout {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        Err(io::Error::UNSUPPORTED_PLATFORM)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
pub const STDIN_BUF_SIZE: usize = 0;
pub fn is_ebadf(_: &io::Error) -> bool {
    false
}
pub fn panic_output() -> Option<Stderr> {
    None
}
