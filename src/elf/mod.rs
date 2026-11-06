use crate::Result;
use serde::Serialize;

#[derive(Serialize)]
pub struct Report {
    entry: u64,
    load_segments: usize,
    pub sha256: String,
}

pub fn hash(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}

fn number(bytes: &[u8], offset: usize, width: usize) -> Result<u64> {
    let part = bytes
        .get(offset..offset.checked_add(width).ok_or("ELF offset overflow")?)
        .ok_or("truncated ELF")?;
    let mut value = [0; 8];
    value[..width].copy_from_slice(part);
    Ok(u64::from_le_bytes(value))
}

fn end(start: u64, size: u64) -> Result<u64> {
    start
        .checked_add(size)
        .ok_or_else(|| "ELF range overflow".into())
}

pub fn inspect(bytes: &[u8]) -> Result<Report> {
    if bytes.get(..7) != Some(b"\x7fELF\x02\x01\x01")
        || number(bytes, 16, 2)? != 2
        || number(bytes, 18, 2)? != 243
        || number(bytes, 20, 4)? != 1
        || number(bytes, 52, 2)? != 64
    {
        return Err("expected static ELF64 little-endian RISC-V executable".into());
    }
    // The bootstrap compiler uses LP64D and optional compressed instructions.
    let flags = number(bytes, 48, 4)?;
    if flags & !1 != 4 {
        return Err("expected LP64D ELF flags".into());
    }
    let entry = number(bytes, 24, 8)?;
    let table = number(bytes, 32, 8)?;
    let count = number(bytes, 56, 2)?;
    if number(bytes, 54, 2)? != 56 || count == 0 || end(table, count * 56)? > bytes.len() as u64 {
        return Err("invalid ELF program header table".into());
    }
    let mut pages = Vec::new();
    let mut executable_entry = false;
    for index in 0..count {
        let at = usize::try_from(table + index * 56)?;
        match number(bytes, at, 4)? {
            2 | 3 | 7 => return Err("dynamic linking and TLS are unsupported by bootstrap".into()),
            1 => {}
            _ => continue,
        }
        let flags = number(bytes, at + 4, 4)?;
        let offset = number(bytes, at + 8, 8)?;
        let address = number(bytes, at + 16, 8)?;
        let file_size = number(bytes, at + 32, 8)?;
        let memory_size = number(bytes, at + 40, 8)?;
        let align = number(bytes, at + 48, 8)?;
        if flags & !7 != 0 || flags & 4 == 0 || flags & 3 == 3 {
            return Err("LOAD segment must be readable and never writable+executable".into());
        }
        if file_size > memory_size || end(offset, file_size)? > bytes.len() as u64 {
            return Err("invalid LOAD file range".into());
        }
        if align > 1 && (!align.is_power_of_two() || offset % align != address % align) {
            return Err("invalid LOAD alignment".into());
        }
        if memory_size == 0 {
            continue;
        }
        let start_page = address & !4095;
        let end_page = end(end(address, memory_size)?, 4095)? & !4095;
        if start_page == 0 || pages.iter().any(|&(a, b)| start_page < b && a < end_page) {
            return Err("LOAD pages overlap or include the null page".into());
        }
        pages.push((start_page, end_page));
        if flags & 1 != 0 && entry >= address && end(entry, 2)? <= end(address, file_size)? {
            executable_entry = true;
        }
    }
    if entry & 1 != 0 || !executable_entry {
        return Err("entry must be aligned and file-backed in an executable LOAD".into());
    }
    Ok(Report {
        entry,
        load_segments: pages.len(),
        sha256: hash(bytes),
    })
}
