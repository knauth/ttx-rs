use std::{path::PathBuf, u8};

use luwen::ttkmd_if::PciDevice;
use tempfile::TempDir;
use tracing::info;
use ttx_rs::{
    chip::{
        self,
        noc::{NocId, NocInterface, Tile},
    },
    kernel::{Kernel, KernelData},
    Chip,
};

#[ctor::ctor]
fn test_init() {
    tracing_subscriber::util::SubscriberInitExt::init(
        tracing_subscriber::layer::SubscriberExt::with(
            tracing_subscriber::layer::SubscriberExt::with(
                tracing_subscriber::registry(),
                tracing_subscriber::fmt::layer(),
            ),
            tracing_subscriber::filter::EnvFilter::from_default_env(),
        ),
    );
}

fn write_cargo_toml(dir: &TempDir) {
    std::fs::write(
        dir.path().join("Cargo.toml"),
        format!(
            r#"
    [package]
    name = "kernel"
    version = "0.1.0"
    edition = "2024"

    [dependencies]
    tensix-std = {{path = "{}/../tensix-std"}}
    "#,
            env!("CARGO_MANIFEST_DIR"),
        ),
    )
    .unwrap();
}

fn write_main(src_file: &PathBuf, test: &str) {
    std::fs::write(
        src_file,
        format!(
            r#"
    #![no_std]
    #![no_main]

    #[repr(align(64))]
    struct NocAligned<T>(T);

    impl<T> core::ops::Deref for NocAligned<T> {{
        type Target = T;

        fn deref(&self) -> &<Self as core::ops::Deref>::Target {{
            &self.0
        }}
    }}

    impl<T> core::ops::DerefMut for NocAligned<T> {{
        fn deref_mut(&mut self) -> &mut <Self as core::ops::Deref>::Target {{
            &mut self.0
        }}
    }}

    #[repr(transparent)]
    struct SyncUnsafeCell<T>(core::cell::UnsafeCell<T>);
    unsafe impl<T: Sync> Sync for SyncUnsafeCell<T> {{}}

    #[repr(transparent)]
    pub struct SyncUnsafeNocCell<T>(NocAligned<SyncUnsafeCell<T>>);

    impl<T> SyncUnsafeNocCell<T> {{
        pub const fn new(value: T) -> Self {{
            SyncUnsafeNocCell(NocAligned(SyncUnsafeCell(core::cell::UnsafeCell::new(value))))
        }}

        pub fn get(&self) -> *mut T {{
            ((self.0).0).0.get()
        }}
    }}

    type SYNC<T> = SyncUnsafeNocCell<T>;

    #[unsafe(no_mangle)]
    pub static CORE_ID: SYNC<i32> = SYNC::new(-1);

    struct Tile {{
        n0: (u8, u8),
        n1: (u8, u8)
    }}

    impl From<u32> for Tile {{
        fn from(value: u32) -> Self {{
            Tile {{
                n0: (value as u8, (value >> 8) as u8),
                n1: ((value >> 16) as u8, (value >> 24) as u8)
            }}
        }}
    }}

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn memcpy(dest: *mut u8, src: *const u8, n: usize) -> *mut u8 {{
        unsafe {{
            let mut i = 0;
            while i < n {{
                dest.add(i).write(src.add(i).read());
                i += 1;
            }}
            dest
        }}
    }}

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn memset(dest: *mut u8, src: core::ffi::c_int, n: usize) -> *mut u8 {{
        unsafe {{
            let mut i = 0;
            while i < n {{
                dest.add(i).write(src as u8);
                i += 1;
            }}
            dest
        }}
    }}

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn memcmp(lhs: *const core::ffi::c_void, rhs: *const core::ffi::c_void, count: usize) -> core::ffi::c_int {{
        unsafe {{
            for i in 0..count {{
                let a = lhs.cast::<u8>().offset(i as isize).read();
                let b = rhs.cast::<u8>().offset(i as isize).read();

                let cmp = a as i32 - b as i32;
                if cmp != 0 {{
                    return cmp;
                }}
            }}
        }}

        return 0;
    }}

    {test}
    "#
        ),
    )
    .unwrap()
}

fn build_test(chip: &mut Chip, noc_id: NocId, tile: Tile, wait: bool, file: &str) -> Kernel {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("src")).unwrap();

    let src_file = dir.path().join("src").join("main.rs");

    write_cargo_toml(&dir);
    write_main(&src_file, file);

    let kernel_data = chip::loader::build_kernel(
        &"test".to_string(),
        chip.arch(),
        chip::loader::LoadOptions::new(dir.path()).hide_output(),
        None,
    );

    chip.load_kernel(kernel_data, noc_id, tile, wait)
}

#[allow(unused)]
fn build_tests(chip: &mut Chip, tiles: Option<Vec<Tile>>, file: &str, wait: bool) -> KernelData {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("src")).unwrap();

    let src_file = dir.path().join("src").join("main.rs");

    write_cargo_toml(&dir);
    write_main(&src_file, file);

    let mut kernel_data = chip::loader::build_kernel(
        &"test".to_string(),
        chip.arch(),
        chip::loader::LoadOptions::new(dir.path()).hide_output(),
        None,
    );

    chip.load_kernels(&mut kernel_data, tiles, wait);

    kernel_data
}

macro_rules! rust_test {
    ($chip:ident, $noc_id:expr, $tile:expr, {$($t:tt)*}) => {{
        let __tile = $tile;
        build_test(
            &mut $chip,
            $noc_id,
            __tile,
            core::stringify!($($t)*),
            true
        )
    }};

    (nowait, $chip:ident, $noc_id:expr, $tile:expr, {$($t:tt)*}) => {{
        let __tile = $tile;
        build_test(
            &mut $chip,
            $noc_id,
            __tile,
            core::stringify!($($t)*),
            false
        )
    }};
}

#[test]
fn pci_to_tensix_32() {
    for id in PciDevice::scan() {
        let mut chip = if let Ok(chip) = chip::open(id) {
            chip
        } else {
            continue;
        };

        let base_addr = chip.tensix_l1() / 3;
        let value = 0xfacadaca;

        // Check baseline
        info!("Checking baseline");
        let tile = chip.tensix(0);
        chip.noc_write32(NocId::Noc0, tile, tile.align_write_ptr(base_addr), value);
        let readback = chip.noc_read32(NocId::Noc1, tile, tile.align_read_ptr(base_addr));
        assert_eq!(value, readback);
        info!("Checked baseline");

        // Test read
        info!("Checking read");
        let tile = chip.tensix(1);
        let mut alignment = 1;
        let mut read_value = value;
        // let mut ever_checked_eq = false;
        // let mut ever_checked_ne = false;
        while alignment < ((tile.align_read as u64) << 2) {
            let addr = (base_addr + (alignment - 1) & !(alignment - 1)) + alignment;
            assert!(addr < chip.tensix_l1());

            read_value = read_value.wrapping_add(0x12345678);

            let write_addr = base_addr & !((tile.align_write as u64) - 1);

            let mut value = vec![];
            for i in write_addr..(addr + 25) {
                value.push((i as u8).wrapping_add((read_value >> (read_value % 4)) as u8));
            }

            chip.noc_write(NocId::Noc1, tile, write_addr, &value);
            let readback = chip.noc_read32(NocId::Noc0, tile, addr);

            let offset = addr - write_addr;
            let offset = offset as usize;
            let expected = u32::from_le_bytes([
                value[offset..][0],
                value[offset..][1],
                value[offset..][2],
                value[offset..][3],
            ]);

            assert_eq!(
                expected, readback,
                "Did not expect to fail to correctly readback with an alignment of {alignment} from 0x{addr:x} with an offset of {offset}"
            );

            // if alignment >= tile.align_read as u64 {
            //     ever_checked_eq = true;
            //     assert_eq!(
            //         expected, readback,
            //         "Did not expect to fail to correctly readback with an alignment of {alignment} from 0x{addr:x} with an offset of {offset}"
            //     );
            // } else {
            //     ever_checked_ne = true;
            //     if expected == readback {
            //         error!("Did not expect to correctly readback with an alignment of {alignment} from 0x{addr:x} with an offset of {offset}");
            //     }
            //     // assert_ne!(
            //     //     expected, readback,
            //     //     "Did not expect to correctly readback with an alignment of {alignment} from 0x{addr:x} with an offset of {offset}"
            //     // );
            // }

            alignment <<= 1;
        }

        // assert!(ever_checked_eq && ever_checked_ne);

        info!("Checked read");

        // Test write
        info!("Checking write");
        let tile = chip.tensix(2);
        let mut alignment = 1;
        let mut write_value = value;
        // let mut ever_checked_eq = false;
        // let mut ever_checked_ne = false;
        while alignment < ((tile.align_read as u64) << 2) {
            let addr = (base_addr + (alignment - 1) & !(alignment - 1)) + alignment;
            assert!(addr < chip.tensix_l1());

            write_value = write_value.wrapping_add(0x12345678);

            let read_addr = base_addr & !((tile.align_read as u64) - 1);

            chip.noc_write32(NocId::Noc1, tile, addr, write_value);

            let mut read_value = vec![0u8; (addr - read_addr + 25) as usize];
            chip.noc_read(NocId::Noc0, tile, read_addr, &mut read_value);

            let offset = addr - read_addr;
            let offset = offset as usize;
            let expected = u32::from_le_bytes([
                read_value[offset..][0],
                read_value[offset..][1],
                read_value[offset..][2],
                read_value[offset..][3],
            ]);

            assert_eq!(
                expected, write_value,
                "Did not expect to fail to correctly write with an alignment of {alignment} from 0x{addr:x} with an offset of {offset}"
            );

            // if alignment >= tile.align_write as u64 {
            //     ever_checked_eq = true;
            //     assert_eq!(
            //         expected, write_value,
            //         "Did not expect to fail to correctly write with an alignment of {alignment} from 0x{addr:x} with an offset of {offset}"
            //     );
            // } else {
            //     ever_checked_ne = true;
            //     if expected == write_value {
            //         error!("Did not expect to correctly write with an alignment of {alignment} from 0x{addr:x} with an offset of {offset}");
            //     }
            //     // assert_ne!(
            //     //     expected, readback,
            //     //     "Did not expect to correctly readback with an alignment of {alignment} from 0x{addr:x} with an offset of {offset}"
            //     // );
            // }

            alignment <<= 1;
        }
        // assert!(ever_checked_eq && ever_checked_ne);
        info!("Checked write");
    }
}

#[test]
fn pci_to_tensix_block() {
    for id in PciDevice::scan() {
        let mut chip = if let Ok(chip) = chip::open(id) {
            chip
        } else {
            continue;
        };

        let base_addr = chip.tensix_l1() / 3;
        let mut value = vec![0u8; 1473];
        for i in 0..value.len() {
            value[i] = (i % u8::MAX as usize) as u8;
        }

        // Check baseline
        info!("Checking baseline");
        let tile = chip.tensix(0);
        chip.noc_write(NocId::Noc0, tile, tile.align_write_ptr(base_addr), &value);

        let mut readback = vec![0u8; value.len()];
        chip.noc_read(
            NocId::Noc1,
            tile,
            tile.align_read_ptr(base_addr),
            &mut readback,
        );
        assert_eq!(value, readback);
        info!("Checked baseline");

        // Test read
        info!("Checking read");
        let tile = chip.tensix(1);
        let mut alignment = 1;
        // let mut ever_checked_eq = false;
        // let mut ever_checked_ne = false;
        while alignment < ((tile.align_read as u64) << 2) {
            let addr = (base_addr + (alignment - 1) & !(alignment - 1)) + alignment;
            assert!(addr < chip.tensix_l1());

            for i in 0..value.len() {
                value[i] = value[i].wrapping_add((0x12345678 >> (i % 4)) as u8);
            }

            let write_addr = base_addr & !((tile.align_write as u64) - 1);
            chip.noc_write(NocId::Noc1, tile, write_addr, &value);

            let offset = addr - write_addr;
            let offset = offset as usize;
            let mut readback = vec![0u8; value.len() - offset];
            chip.noc_read(NocId::Noc0, tile, addr, &mut readback);

            let expected = &value[offset..];
            assert_eq!(
                expected, readback,
                "Did not expect to fail to correctly readback with an alignment of {alignment} from 0x{addr:x} with an offset of {offset}"
            );

            // if alignment >= tile.align_read as u64 {
            //     ever_checked_eq = true;
            //     assert_eq!(
            //         expected, readback,
            //         "Did not expect to fail to correctly readback with an alignment of {alignment} from 0x{addr:x} with an offset of {offset}"
            //     );
            // } else {
            //     ever_checked_ne = true;
            //     if expected == readback {
            //         error!("Did not expect to correctly readback with an alignment of {alignment} from 0x{addr:x} with an offset of {offset}");
            //     }
            //     // assert_ne!(
            //     //     expected, readback,
            //     //     "Did not expect to correctly readback with an alignment of {alignment} from 0x{addr:x} with an offset of {offset}"
            //     // );
            // }

            alignment <<= 1;
        }

        // assert!(ever_checked_eq && ever_checked_ne);

        info!("Checked read");

        // Test write
        info!("Checking write");
        let tile = chip.tensix(2);
        let mut alignment = 1;
        // let mut ever_checked_eq = false;
        // let mut ever_checked_ne = false;
        while alignment < ((tile.align_read as u64) << 2) {
            let addr = (base_addr + (alignment - 1) & !(alignment - 1)) + alignment;
            assert!(addr < chip.tensix_l1());

            for i in 0..value.len() {
                value[i] = value[i].wrapping_add((0x12345678 >> (i % 4)) as u8);
            }

            let read_addr = base_addr & !((tile.align_read as u64) - 1);

            chip.noc_write(NocId::Noc1, tile, addr, &value);

            let offset = addr - read_addr;
            let offset = offset as usize;
            let mut readback = vec![0u8; value.len() + offset];
            chip.noc_read(NocId::Noc0, tile, read_addr, &mut readback);

            let expected = &value;
            let readback = &readback[offset..];

            assert_eq!(
                expected, readback,
                "Did not expect to fail to correctly write with an alignment of {alignment} from 0x{addr:x} with an offset of {offset}"
            );

            // if alignment >= tile.align_write as u64 {
            //     ever_checked_eq = true;
            //     assert_eq!(
            //         expected, readback,
            //         "Did not expect to fail to correctly write with an alignment of {alignment} from 0x{addr:x} with an offset of {offset}"
            //     );
            // } else {
            //     ever_checked_ne = true;
            //     if expected == readback {
            //         error!("Did not expect to correctly write with an alignment of {alignment} from 0x{addr:x} with an offset of {offset}");
            //     }
            //     // assert_ne!(
            //     //     expected, readback,
            //     //     "Did not expect to correctly readback with an alignment of {alignment} from 0x{addr:x} with an offset of {offset}"
            //     // );
            // }

            alignment <<= 1;
        }
        // assert!(ever_checked_eq && ever_checked_ne);
        info!("Checked write");
    }
}

#[test]
fn pci_to_dram_32() {
    for id in PciDevice::scan() {
        let mut chip = if let Ok(chip) = chip::open(id) {
            chip
        } else {
            continue;
        };

        let base_addr = chip.dram_size() / 3;
        let value = 0xfacadaca;

        // Check baseline
        info!("Checking baseline");
        let dram_tiles = chip.dram(0).len();
        let tile = chip.dram(0)[2 % dram_tiles];
        chip.noc_write32(NocId::Noc0, tile, tile.align_rw_ptr(base_addr), value);
        let readback = chip.noc_read32(NocId::Noc1, tile, tile.align_rw_ptr(base_addr));
        assert_eq!(value, readback);
        info!("Checked baseline");

        // Test read
        info!("Checking read");
        let tile = chip.dram(1)[1 % dram_tiles];
        let mut alignment = 1;
        let mut read_value = value;
        // let mut ever_checked_eq = false;
        // let mut ever_checked_ne = false;
        while alignment < ((tile.align_read as u64) << 2) {
            let addr = (base_addr + (alignment - 1) & !(alignment - 1)) + alignment;
            assert!(addr < chip.dram_size());

            read_value = read_value.wrapping_add(0x12345678);

            let write_addr = base_addr & !((tile.align_write as u64) - 1);

            let mut value = vec![];
            for i in write_addr..(addr + 25) {
                value.push((i as u8).wrapping_add((read_value >> (read_value % 4)) as u8));
            }

            chip.noc_write(NocId::Noc1, tile, write_addr, &value);
            let readback = chip.noc_read32(NocId::Noc0, tile, addr);

            let offset = addr - write_addr;
            let offset = offset as usize;
            let expected = u32::from_le_bytes([
                value[offset..][0],
                value[offset..][1],
                value[offset..][2],
                value[offset..][3],
            ]);

            assert_eq!(
                expected, readback,
                "Did not expect to fail to correctly readback with an alignment of {alignment} from 0x{addr:x} with an offset of {offset}"
            );

            // if alignment >= tile.align_read as u64 {
            //     ever_checked_eq = true;
            //     assert_eq!(
            //         expected, readback,
            //         "Did not expect to fail to correctly readback with an alignment of {alignment} from 0x{addr:x} with an offset of {offset}"
            //     );
            // } else {
            //     ever_checked_ne = true;
            //     if expected == readback {
            //         error!("Did not expect to correctly readback with an alignment of {alignment} from 0x{addr:x} with an offset of {offset}");
            //     }
            //     // assert_ne!(
            //     //     expected, readback,
            //     //     "Did not expect to correctly readback with an alignment of {alignment} from 0x{addr:x} with an offset of {offset}"
            //     // );
            // }

            alignment <<= 1;
        }

        // assert!(ever_checked_eq && ever_checked_ne);

        info!("Checked read");

        // Test write
        info!("Checking write");
        let tile = chip.dram(2)[0];
        let mut alignment = 1;
        let mut write_value = value;
        // let mut ever_checked_eq = false;
        // let mut ever_checked_ne = false;
        while alignment < ((tile.align_read as u64) << 2) {
            let addr = (base_addr + (alignment - 1) & !(alignment - 1)) + alignment;
            assert!(addr < chip.dram_size());

            write_value = write_value.wrapping_add(0x12345678);

            let read_addr = base_addr & !((tile.align_read as u64) - 1);

            chip.noc_write32(NocId::Noc1, tile, addr, write_value);

            let mut read_value = vec![0u8; (addr - read_addr + 25) as usize];
            chip.noc_read(NocId::Noc0, tile, read_addr, &mut read_value);

            let offset = addr - read_addr;
            let offset = offset as usize;
            let expected = u32::from_le_bytes([
                read_value[offset..][0],
                read_value[offset..][1],
                read_value[offset..][2],
                read_value[offset..][3],
            ]);

            assert_eq!(
                expected, write_value,
                "Did not expect to fail to correctly write with an alignment of {alignment} from 0x{addr:x} with an offset of {offset}"
            );

            // if alignment >= tile.align_write as u64 {
            //     ever_checked_eq = true;
            //     assert_eq!(
            //         expected, write_value,
            //         "Did not expect to fail to correctly write with an alignment of {alignment} from 0x{addr:x} with an offset of {offset}"
            //     );
            // } else {
            //     ever_checked_ne = true;
            //     if expected == write_value {
            //         error!("Did not expect to correctly write with an alignment of {alignment} from 0x{addr:x} with an offset of {offset}");
            //     }
            //     // assert_ne!(
            //     //     expected, readback,
            //     //     "Did not expect to correctly readback with an alignment of {alignment} from 0x{addr:x} with an offset of {offset}"
            //     // );
            // }

            alignment <<= 1;
        }
        // assert!(ever_checked_eq && ever_checked_ne);
        info!("Checked write");
    }
}

#[test]
fn pci_to_dram_block() {
    for id in PciDevice::scan() {
        let mut chip = if let Ok(chip) = chip::open(id) {
            chip
        } else {
            continue;
        };

        let base_addr = chip.dram_size() / 3;
        let mut value = vec![0u8; 1473];
        for i in 0..value.len() {
            value[i] = (i % u8::MAX as usize) as u8;
        }

        // Check baseline
        info!("Checking baseline");
        let dram_tiles = chip.dram(0).len();
        let tile = chip.dram(0)[2 % dram_tiles];
        chip.noc_write(NocId::Noc0, tile, tile.align_rw_ptr(base_addr), &value);

        let mut readback = vec![0u8; value.len()];
        chip.noc_read(
            NocId::Noc1,
            tile,
            tile.align_rw_ptr(base_addr),
            &mut readback,
        );
        assert_eq!(value, readback);
        info!("Checked baseline");

        // Test read
        info!("Checking read");
        let tile = chip.dram(1)[1 % dram_tiles];
        let mut alignment = 1;
        // let mut ever_checked_eq = false;
        // let mut ever_checked_ne = false;
        while alignment < ((tile.align_read as u64) << 2) {
            let addr = (base_addr + (alignment - 1) & !(alignment - 1)) + alignment;
            assert!(addr < chip.dram_size());

            for i in 0..value.len() {
                value[i] = value[i].wrapping_add((0x12345678 >> (i % 4)) as u8);
            }

            let write_addr = base_addr & !((tile.align_write as u64) - 1);
            chip.noc_write(NocId::Noc1, tile, write_addr, &value);

            let offset = addr - write_addr;
            let offset = offset as usize;
            let mut readback = vec![0u8; value.len() - offset];
            chip.noc_read(NocId::Noc0, tile, addr, &mut readback);

            let expected = &value[offset..];
            assert_eq!(
                expected, readback,
                "Did not expect to fail to correctly readback with an alignment of {alignment} from 0x{addr:x} with an offset of {offset}"
            );

            // if alignment >= tile.align_read as u64 {
            //     ever_checked_eq = true;
            //     assert_eq!(
            //         expected, readback,
            //         "Did not expect to fail to correctly readback with an alignment of {alignment} from 0x{addr:x} with an offset of {offset}"
            //     );
            // } else {
            //     ever_checked_ne = true;
            //     if expected == readback {
            //         error!("Did not expect to correctly readback with an alignment of {alignment} from 0x{addr:x} with an offset of {offset}");
            //     }
            //     // assert_ne!(
            //     //     expected, readback,
            //     //     "Did not expect to correctly readback with an alignment of {alignment} from 0x{addr:x} with an offset of {offset}"
            //     // );
            // }

            alignment <<= 1;
        }

        // assert!(ever_checked_eq && ever_checked_ne);

        info!("Checked read");

        // Test write
        info!("Checking write");
        let tile = chip.dram(2)[0];
        let mut alignment = 1;
        // let mut ever_checked_eq = false;
        // let mut ever_checked_ne = false;
        while alignment < ((tile.align_read as u64) << 2) {
            let addr = (base_addr + (alignment - 1) & !(alignment - 1)) + alignment;
            assert!(addr < chip.dram_size());

            for i in 0..value.len() {
                value[i] = value[i].wrapping_add((0x12345678 >> (i % 4)) as u8);
            }

            let read_addr = base_addr & !((tile.align_read as u64) - 1);

            chip.noc_write(NocId::Noc1, tile, addr, &value);

            let offset = addr - read_addr;
            let offset = offset as usize;
            let mut readback = vec![0u8; value.len() + offset];
            chip.noc_read(NocId::Noc0, tile, read_addr, &mut readback);

            let expected = &value;
            let readback = &readback[offset..];

            assert_eq!(
                expected, readback,
                "Did not expect to fail to correctly write with an alignment of {alignment} from 0x{addr:x} with an offset of {offset}"
            );

            // if alignment >= tile.align_write as u64 {
            //     ever_checked_eq = true;
            //     assert_eq!(
            //         expected, readback,
            //         "Did not expect to fail to correctly write with an alignment of {alignment} from 0x{addr:x} with an offset of {offset}"
            //     );
            // } else {
            //     ever_checked_ne = true;
            //     if expected == readback {
            //         error!("Did not expect to correctly write with an alignment of {alignment} from 0x{addr:x} with an offset of {offset}");
            //     }
            //     // assert_ne!(
            //     //     expected, readback,
            //     //     "Did not expect to correctly readback with an alignment of {alignment} from 0x{addr:x} with an offset of {offset}"
            //     // );
            // }

            alignment <<= 1;
        }
        // assert!(ever_checked_eq && ever_checked_ne);
        info!("Checked write");
    }
}

#[test]
fn tensix_to_dram_block() {
    for id in PciDevice::scan() {
        let mut chip = if let Ok(chip) = chip::open(id) {
            chip
        } else {
            continue;
        };

        let mut kernel = build_test(
            &mut chip.dupe().unwrap(),
            NocId::Noc0,
            chip.tensix(0),
            true,
            &format!(
                r#"
            use tensix_std::entry;

            #[unsafe(no_mangle)]
            pub static VALUE_BUFFER: SyncUnsafeNocCell<[u8; 1473]> = SyncUnsafeNocCell::new([0; 1473]);

            #[unsafe(no_mangle)]
            pub static READBACK_BUFFER: SyncUnsafeNocCell<[u8; 1473]> = SyncUnsafeNocCell::new([0; 1473]);

            #[entry(brisc)]
            unsafe fn brisc_main() {{
                unsafe {{
                    unsafe fn set_pc(pc: u16) {{
                        unsafe {{
                            tensix_std::set_postcode_brisc(0xc0de0000 | pc as u32);
                        }}
                    }}

                    let base_addr = {base_addr};
                    for i in 0..(*VALUE_BUFFER.get()).len() {{
                        (*VALUE_BUFFER.get())[i] = (i % u8::MAX as usize) as u8;
                    }}

                    set_pc(0x1);

                    let read_align = tensix_std::target::noc_map::ALIGNMENT_DRAM_READ as u64;
                    let write_align = tensix_std::target::noc_map::ALIGNMENT_DRAM_WRITE as u64;
                    let max_align = read_align.max(write_align) as u64;

                    set_pc(0x2);

                    // Check baseline
                    tensix_std::target::noc::noc_write(
                        tensix_std::target::noc::NocCommandSel::default(),
                        tensix_std::target::noc::NocAddr {{
                            offset: (base_addr + (max_align - 1)) & !(max_align - 1),
                            x_end: {x_end_0},
                            y_end: {y_end_0},
                            ..Default::default()
                        }},
                        (*VALUE_BUFFER.get()).as_slice(),
                        true,
                    );

                    set_pc(0x3);

                    tensix_std::target::noc::noc_read(
                        tensix_std::target::noc::NocCommandSel::default(),
                        tensix_std::target::noc::NocAddr {{
                            offset: (base_addr + (max_align - 1)) & !(max_align - 1),
                            x_end: {x_end_0},
                            y_end: {y_end_0},
                            ..Default::default()
                        }},
                        (*READBACK_BUFFER.get()).as_mut_slice(),
                        true,
                    );

                    set_pc(0x4);

                    assert_eq!(&*VALUE_BUFFER.get(), &*READBACK_BUFFER.get());

                    set_pc(0x5);
                }}
            }}
        "#,
                base_addr = chip.dram_size() / 3,
                x_end_0 = chip.dram(0)[0].addr.n0.0,
                y_end_0 = chip.dram(0)[0].addr.n0.1,
            ),
        );

        assert!(!kernel.check_panic());
    }
}
