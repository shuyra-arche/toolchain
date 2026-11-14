//! Test-only entry for Vanllia's platform/riscv package. Copy this file to that
//! checkout's src/main.rs; it is paired exclusively with this suite's QEMU virt
//! invocation, one Hart, 256 MiB ordinary RAM and no external devices.
#![no_std]
#![no_main]
extern crate alloc;
use kernel::entry::boot::{
    self,
    memory::topology::{Fabric, Ram, Topology},
};

#[unsafe(no_mangle)]
extern "C" fn main(hart: usize, dtp: usize) -> ! {
    let result = unsafe { platform_riscv::boot::init(hart, dtp) }.and_then(|_| {
        let tree = unsafe { fdt::Fdt::from_ptr(dtp as *const u8) }.unwrap();
        assert!(
            tree.root()
                .compatible()
                .all()
                .any(|v| matches!(v, "riscv-virtio,qemu" | "riscv-virtio"))
        );
        assert_eq!(tree.cpus().count(), 1);
        let regions: alloc::vec::Vec<_> = tree.memory().regions().collect();
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].starting_address as usize, 0x80000000);
        assert_eq!(regions[0].size, Some(256 * 1024 * 1024));
        let bytes = unsafe { core::slice::from_raw_parts(dtp as *const u8, tree.total_size()) };
        let admitted = platform_riscv::memory::parse(bytes, dtp, boot::image_region()).unwrap();
        let ram = admitted
            .identity()
            .iter()
            .map(|region| Ram {
                base: region.base as u64,
                bytes: region.size as u64,
            })
            .collect();
        // SAFETY: only the paired single-Hart QEMU virt invocation is admitted.
        // All access is through that CPU's ordinary coherent RAM; no DMA,
        // passthrough, hotplug or external instruction-fetch alias is present.
        // Kernel checks admitted RAM, rejects SMP and performs FENCE.I plus
        // local CPU drain before completing executable publication/retirement.
        let topology = unsafe {
            Topology::new(alloc::vec![Fabric {
                ram,
                routes: alloc::vec![]
            }])
            .map(|topology| topology.with_local_instructions())
        }
        .unwrap();
        boot::qualify_memory(topology).unwrap();
        boot::start()
    });
    match result {
        Ok(()) => boot::run(),
        Err(error) => boot::fail(error),
    }
}
