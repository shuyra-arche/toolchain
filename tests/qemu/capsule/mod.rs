//! Encode only the ELF subset already accepted by the Toolchain inspect CLI.
//! Kernel's public parser validates the resulting wire format before boot.
use kernel_api_v7::boot::capsule::{HEADER, MAGIC, PAGE, RECORD};

#[derive(Clone, Copy)]
pub enum Format {
    Legacy,
    V7,
}

// These capsule generations use the same region and payload layout.
const _: () = {
    use kernel_api::boot::capsule as legacy;
    assert!(HEADER == legacy::HEADER && PAGE == legacy::PAGE && RECORD == legacy::RECORD);
};

fn number(bytes: &[u8], at: usize, width: usize) -> usize {
    let mut raw = [0; 8];
    raw[..width].copy_from_slice(&bytes[at..at.checked_add(width).unwrap()]);
    usize::try_from(u64::from_le_bytes(raw)).unwrap()
}

pub fn initrd(elf: &[u8], format: Format) -> Vec<u8> {
    let table = number(elf, 32, 8);
    let segments: Vec<_> = (0..number(elf, 56, 2))
        .map(|i| table.checked_add(i.checked_mul(56).unwrap()).unwrap())
        .filter(|&at| number(elf, at, 4) == 1 && number(elf, at + 40, 8) != 0)
        .collect();
    let mut out = vec![0; (HEADER + segments.len() * RECORD).next_multiple_of(PAGE)];
    out[..8].copy_from_slice(&MAGIC);
    let (version, abi) = match format {
        Format::Legacy => (1u16, 0),
        Format::V7 => (
            kernel_api_v7::boot::capsule::VERSION,
            u8::try_from(kernel_api_v7::abi::VERSION).unwrap(),
        ),
    };
    out[8..10].copy_from_slice(&version.to_le_bytes());
    out[10] = PAGE.trailing_zeros() as u8;
    out[11] = abi;
    out[12..16].copy_from_slice(&u32::try_from(segments.len()).unwrap().to_le_bytes());
    out[16..24].copy_from_slice(&elf[24..32]);
    for (index, at) in segments.into_iter().enumerate() {
        let flags = number(elf, at + 4, 4);
        let offset = number(elf, at + 8, 8);
        let address = number(elf, at + 16, 8);
        let file_size = number(elf, at + 32, 8);
        let memory_size = number(elf, at + 40, 8);
        let va = address / PAGE * PAGE;
        let prefix = address - va;
        let size = prefix
            .checked_add(memory_size)
            .unwrap()
            .checked_next_multiple_of(PAGE)
            .unwrap();
        let data_size = if flags & 1 != 0 {
            size // Executable pages must be completely backed by capsule bytes.
        } else if file_size == 0 {
            0
        } else {
            prefix
                .checked_add(file_size)
                .unwrap()
                .checked_next_multiple_of(PAGE)
                .unwrap()
        };
        let payload = out.len();
        let end = payload.checked_add(data_size).unwrap();
        // This harness boots small fixtures in a 256 MiB machine, not arbitrary images.
        assert!(
            end <= 256 * 1024 * 1024,
            "fixture capsule exceeds guest memory"
        );
        out.resize(end, 0);
        if file_size != 0 {
            out[payload + prefix..payload + prefix + file_size]
                .copy_from_slice(&elf[offset..offset + file_size]);
        }
        let record = HEADER + index * RECORD;
        for (field, value) in [
            va,
            size / PAGE,
            data_size / PAGE,
            if data_size == 0 { 0 } else { payload },
        ]
        .into_iter()
        .enumerate()
        {
            out[record + field * 8..record + field * 8 + 8]
                .copy_from_slice(&(value as u64).to_le_bytes());
        }
        let permissions = ((flags & 4) >> 2 | (flags & 2) | (flags & 1) << 2) as u32;
        out[record + 32..record + 36].copy_from_slice(&permissions.to_le_bytes());
    }
    let length = out.len() as u64;
    out[24..32].copy_from_slice(&length.to_le_bytes());
    let mut initrd = vec![0; PAGE];
    initrd[..4].copy_from_slice(&u32::try_from(PAGE).unwrap().to_le_bytes());
    initrd[4..8].copy_from_slice(&u32::try_from(out.len()).unwrap().to_le_bytes());
    initrd.extend_from_slice(&out);
    let valid = match format {
        Format::Legacy => kernel_api::boot::capsule::Capsule::parse(&initrd[PAGE..]).is_some(),
        Format::V7 => kernel_api_v7::boot::initrd::entry_image(&initrd)
            .and_then(kernel_api_v7::boot::capsule::Capsule::parse)
            .is_some(),
    };
    assert!(valid, "Kernel rejected encoded capsule");
    initrd
}
