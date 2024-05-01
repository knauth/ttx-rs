use std::{collections::HashMap, path::PathBuf};

use goblin::elf::program_header;

use crate::chip::Chip;

pub mod blackhole;
pub mod grayskull;
pub mod wormhole;

#[derive(Debug)]
pub struct NocGrid {
    pub tensix: Vec<(u32, u32)>,
    pub dram: Vec<(u32, u32)>,
    pub pci: Vec<(u32, u32)>,
    pub arc: Vec<(u32, u32)>,
    pub eth: Vec<(u32, u32)>,
}

const BRISC_SOFT_RESET: u32 = 1 << 11;
const TRISC_SOFT_RESETS: u32 = (1 << 12) | (1 << 13) | (1 << 14);
const NCRISC_SOFT_RESET: u32 = 1 << 18;

pub fn reset_to_default(device: &Chip) {
    device.go_idle();
    device.deassert_riscv_reset();

    // Put tensix back under soft reset
    device
        .noc_broadcast32(
            0,
            0xFFB121B0,
            BRISC_SOFT_RESET | TRISC_SOFT_RESETS | NCRISC_SOFT_RESET,
        )
        .unwrap();
}

pub fn lower_clocks(device: &Chip) {
    device.go_idle();
}

pub fn raise_clocks(device: &Chip) {
    device.go_busy();
}

pub fn start_all(device: &Chip, keep_triscs_under_reset: bool, stagger_start: bool) {
    let staggered_start_enable: u32 = if stagger_start { 1 << 31 } else { 0 };

    let soft_reset_value = if keep_triscs_under_reset {
        NCRISC_SOFT_RESET | TRISC_SOFT_RESETS | staggered_start_enable
    } else {
        NCRISC_SOFT_RESET | staggered_start_enable
    };

    // Take cores out of reset
    device
        .noc_broadcast32(0, 0xFFB121B0, soft_reset_value)
        .unwrap();
}

pub fn stop_all(device: &Chip) {
    lower_clocks(device);

    device
        .noc_broadcast32(
            0,
            0xFFB121B0,
            BRISC_SOFT_RESET | TRISC_SOFT_RESETS | NCRISC_SOFT_RESET,
        )
        .unwrap();
}

pub fn start(device: &Chip, x: u8, y: u8, keep_triscs_under_reset: bool, stagger_start: bool) {
    let staggered_start_enable: u32 = if stagger_start { 1 << 31 } else { 0 };

    let soft_reset_value = if keep_triscs_under_reset {
        NCRISC_SOFT_RESET | TRISC_SOFT_RESETS | staggered_start_enable
    } else {
        NCRISC_SOFT_RESET | staggered_start_enable
    };

    // Take cores out of reset
    device
        .noc_write32(0, x, y, 0xFFB121B0, soft_reset_value)
        .unwrap();
    let readback = device.noc_read32(0, x, y, 0xFFB121B0).unwrap();
    debug_assert_eq!(
        readback, soft_reset_value,
        "Failed to start core tried to write {soft_reset_value:x} != {readback:x} "
    );
}

pub fn easy_start(device: &Chip, x: u8, y: u8) {
    start(device, x, y, true, true);
}

pub fn easy_start_all(device: &Chip) {
    start_all(device, true, true);
}

pub fn stop(device: &Chip, x: u8, y: u8) {
    let soft_reset_value = BRISC_SOFT_RESET | TRISC_SOFT_RESETS | NCRISC_SOFT_RESET;
    device
        .noc_write32(0, x, y, 0xFFB121B0, soft_reset_value)
        .unwrap();
    let readback = device.noc_read32(0, x, y, 0xFFB121B0).unwrap();
    debug_assert_eq!(
        readback, soft_reset_value,
        "Failed to stop core tried to write {soft_reset_value:x} to {x}:{y} != {readback:x}"
    );
}

#[repr(align(16))]
pub struct Alignment16(Box<[u8]>);

pub struct KernelBytes {
    pub addr: u32,
    pub data: Alignment16,
}

#[derive(Clone, PartialEq)]
pub struct CoreData {
    pub entry: u32,
    pub state: u32,
    pub pc: u32,
}

pub struct KernelData {
    pub start_sync: Option<u32>,
    pub brisc_state: CoreData,
    pub ncrisc_state: CoreData,
    pub trisc0_state: CoreData,
    pub trisc1_state: CoreData,
    pub trisc2_state: CoreData,

    pub data_start: Option<u32>,
    pub sym_table: HashMap<String, u64>,

    pub writes: Vec<KernelBytes>,
}

pub struct Kernel<'a> {
    pub device: &'a Chip,
    pub x: u8,
    pub y: u8,
    pub data: KernelData,

    pub start_sync: Option<u32>,
    pub core_data: (
        Vec<(String, u32, u32, Option<PanicData>)>,
        Option<PanicData>,
    ),
}

impl KernelData {
    pub fn state_vec(&self) -> Vec<(String, CoreData)> {
        vec![
            ("BRISC".to_string(), self.brisc_state.clone()),
            ("NCRISC".to_string(), self.ncrisc_state.clone()),
            ("TRISC0".to_string(), self.trisc0_state.clone()),
            ("TRISC1".to_string(), self.trisc1_state.clone()),
            ("TRISC2".to_string(), self.trisc2_state.clone()),
        ]
    }
}

impl<'a> Kernel<'a> {
    pub fn new(device: &'a Chip, x: u8, y: u8, data: KernelData) -> Self {
        Self {
            device,
            x,
            y,
            data,
            start_sync: None,
            core_data: (Vec::new(), None),
        }
    }
}

// TODO(drosen): This should be a shared definition
#[repr(C)]
#[derive(PartialEq, Debug)]
pub struct PanicData {
    pub filename_addr: u32,
    pub filename_len: u32,
    pub line: u32,

    pub message_addr: u32,
    pub message_len: u32,

    pub stack_pointer: u32,
    pub program_counter: u32,

    pub panicked: bool,
}

impl Kernel<'_> {
    pub fn read(&mut self, addr: u64, data: &mut [u8]) {
        self.device.noc_read(0, self.x, self.y, addr, data).unwrap();
    }

    pub fn write(&mut self, addr: u64, data: &[u8]) {
        self.device
            .noc_write(0, self.x, self.y, addr, data)
            .unwrap();
    }

    pub fn read32(&mut self, addr: u64) -> u32 {
        self.device.noc_read32(0, self.x, self.y, addr).unwrap()
    }

    pub fn write32(&mut self, addr: u64, value: u32) {
        self.device
            .noc_write32(0, self.x, self.y, addr, value)
            .unwrap();
    }

    pub fn start_sync(&mut self) -> bool {
        if self.value_vec().0.into_iter().all(|v| v.1 == 0) {
            if let Some(sync) = self.data.start_sync {
                let mut sync_value = self.read32(sync as u64);
                if sync_value != 3 {
                    if sync_value == 1 {
                        self.write32(sync as u64, 2);
                        sync_value = self.read32(sync as u64);
                    }

                    return sync_value == 3;
                }
            }
        }

        // If there isn't a start sync point then we just assume we are g2g
        // If the cores already look started then we just assume we are g2g
        true
    }

    fn print_core_panic_data(&mut self, name: &str, panic: &PanicData) -> bool {
        if panic.panicked {
            println!("{name} Panicked");
            let mut buf = vec![0; panic.filename_len as usize];
            println!("filename_len: {}", panic.filename_len);
            self.read(panic.filename_addr as u64, &mut buf);

            let filename = String::from_utf8(buf).unwrap();
            println!("{}; {}", filename, panic.line);

            let mut buf = vec![0; panic.message_len as usize];
            println!("message_len: {}", panic.message_len);
            self.read(panic.message_addr as u64, &mut buf);

            // TODO(drosen): For some reason the message is not completely valid utf-8
            // Also a long message doesn't seem to work...
            let message = String::from_utf8_lossy(&buf);
            println!("{}", message);

            return true;
        }
        return false;
    }

    pub fn read_panic(&mut self, name: &str) -> Option<PanicData> {
        let name = format!("PANIC_DATA_{name}");
        if let Some(postcode_mapping) = self.data.sym_table.get(&name) {
            let mut data = [0; size_of::<PanicData>()];
            self.read(*postcode_mapping, &mut data);

            Some(unsafe { std::mem::transmute_copy(&data) })
        } else {
            None
        }
    }

    fn value_vec(
        &mut self,
    ) -> (
        Vec<(String, u32, u32, Option<PanicData>)>,
        Option<PanicData>,
    ) {
        let mut values = Vec::new();
        for state in self.data.state_vec() {
            let pd = self.read_panic(&state.0);
            values.push((
                state.0,
                self.read32((state.1).state as u64),
                self.read32((state.1).pc as u64),
                pd,
            ));
        }

        (values, self.read_panic("UNKNOWN"))
    }

    pub fn print_state_diff(&mut self) {
        self.maybe_print_state(false);
    }

    pub fn print_state(&mut self) {
        self.maybe_print_state(true);
    }

    pub fn maybe_print_state(&mut self, force: bool) {
        let sync = self.data.start_sync.map(|sync| self.read32(sync as u64));
        let state = (sync, self.value_vec());
        if !force {
            if (&state.0, &state.1) == (&self.start_sync, &self.core_data) {
                return;
            }
        }

        if let Some(sync) = state.0 {
            println!("SYNC: {}", sync);
            if sync != 3 {
                return;
            }
        }

        if let Some(postcode_mapping) = self.data.sym_table.get("NOC_DEBUG") {
            let brc = self
                .device
                .noc_read32(0, self.x, self.y, *postcode_mapping)
                .unwrap();
            println!("noc_debug: 0x{:x}", brc);
        }

        for (name, state, pc, panic) in &(state.1).0 {
            if let Some(panic) = panic {
                self.print_core_panic_data(name, panic);
            }
            println!("{} {{ STATE: {}, POSTCODE: {:x} }}", name, state, pc);
        }
        if let Some(panic) = &(state.1).1 {
            self.print_core_panic_data("UNKNOWN", panic);
        }

        self.start_sync = state.0;
        self.core_data = state.1;
    }

    pub fn check_panic(&mut self) -> bool {
        let (cores, panic) = self.value_vec();
        cores.iter().any(|v| v.1 == 6)
    }

    pub fn wait(&mut self) {
        while !self.all_complete() {
            std::thread::sleep(std::time::Duration::from_millis(10));
            self.print_state_diff();
        }

        crate::loader::stop(&mut self.device, self.x, self.y);

        self.print_state_diff()
    }

    /// Marked by all cores either having completed... or not started
    pub fn all_complete(&mut self) -> bool {
        let states = self.data.state_vec();

        let state_value = states
            .iter()
            .map(|v| self.read32((v.1).state as u64))
            .collect::<Vec<_>>();

        let complete_count = state_value.iter().filter(|v| **v >= 3).count();
        let not_started_count = state_value.iter().filter(|v| **v == 0).count();

        complete_count > 0 && complete_count + not_started_count == states.len()
    }

    pub fn set_entry(&mut self) {
        pub const TENSIX_CFG_BASE: u32 = 4293853184;

        pub const TRISC0_RESET_PC_ADDR: u32 = 158;
        pub const TRISC1_RESET_PC_ADDR: u32 = 159;
        pub const TRISC2_RESET_PC_ADDR: u32 = 160;
        pub const TRISC_RESET_PC_OVERRIDE_EN: u32 = 161;
        pub const NCRISC_RESET_PC_ADDR: u32 = 162;
        pub const NCRISC_RESET_PC_OVERRIDE_EN: u32 = 163;

        self.write32(
            (TENSIX_CFG_BASE + TRISC0_RESET_PC_ADDR) as u64,
            self.data.trisc0_state.entry,
        );
        self.write32(
            (TENSIX_CFG_BASE + TRISC1_RESET_PC_ADDR) as u64,
            self.data.trisc1_state.entry,
        );
        self.write32(
            (TENSIX_CFG_BASE + TRISC2_RESET_PC_ADDR) as u64,
            self.data.trisc2_state.entry,
        );

        self.write32((TENSIX_CFG_BASE + TRISC_RESET_PC_OVERRIDE_EN) as u64, 0b111);

        self.write32(
            (TENSIX_CFG_BASE + NCRISC_RESET_PC_ADDR) as u64,
            self.data.ncrisc_state.entry,
        );

        self.write32((TENSIX_CFG_BASE + NCRISC_RESET_PC_OVERRIDE_EN) as u64, 1);
    }
}

fn preload_bin(elf: &[u8]) -> KernelData {
    let bin = goblin::elf::Elf::parse(elf).unwrap();

    assert_eq!(bin.entry, 0, "Don't yet support non-zero entrypoint");

    let mut writes = vec![];

    for header in bin.program_headers {
        if header.p_type == program_header::PT_LOAD {
            let write = header.vm_range();
            let data = &elf[header.file_range()];

            writes.push(KernelBytes {
                addr: write.start as u32,
                data: Alignment16(data.to_vec().into_boxed_slice()),
            });
        }
    }

    let mut sym_table = HashMap::with_capacity(bin.syms.len());
    for sym in bin.syms.iter() {
        if let Some(name) = bin.strtab.get_at(sym.st_name) {
            sym_table.insert(name, sym.st_value);
        }
    }

    KernelData {
        start_sync: sym_table.get("START_SYNC").map(|v| *v as u32),

        brisc_state: CoreData {
            entry: sym_table["__brisc_start"] as u32,
            state: sym_table["STATE_BRISC"] as u32,
            pc: sym_table["POSTCODE_BRISC"] as u32,
        },

        ncrisc_state: CoreData {
            entry: sym_table["__ncrisc_start"] as u32,
            state: sym_table["STATE_NCRISC"] as u32,
            pc: sym_table["POSTCODE_NCRISC"] as u32,
        },

        trisc0_state: CoreData {
            entry: sym_table["__trisc0_start"] as u32,
            state: sym_table["STATE_TRISC0"] as u32,
            pc: sym_table["POSTCODE_TRISC0"] as u32,
        },

        trisc1_state: CoreData {
            entry: sym_table["__trisc1_start"] as u32,
            state: sym_table["STATE_TRISC1"] as u32,
            pc: sym_table["POSTCODE_TRISC1"] as u32,
        },

        trisc2_state: CoreData {
            entry: sym_table["__trisc2_start"] as u32,
            state: sym_table["STATE_TRISC2"] as u32,
            pc: sym_table["POSTCODE_TRISC2"] as u32,
        },

        data_start: sym_table.get("__firmware_end").map(|v| *v as u32),
        sym_table: sym_table
            .into_iter()
            .map(|v| (v.0.to_string(), v.1))
            .collect(),

        writes,
    }
}

fn load_bin_all(device: &Chip, elf: &[u8]) -> KernelData {
    let data = preload_bin(elf);

    for write in &data.writes {
        let data = write.data.0.as_ref();
        if data.as_ptr().align_offset(std::mem::align_of::<u32>()) != 0 {
            let layout = std::alloc::Layout::array::<u8>(data.len())
                .unwrap()
                .align_to(std::mem::align_of::<u32>())
                .unwrap();
            let datap = unsafe { std::alloc::alloc(layout) };
            let new_data = unsafe { std::slice::from_raw_parts_mut(datap, data.len()) };
            new_data.copy_from_slice(data);
            device
                .noc_broadcast(0, write.addr as u64, new_data)
                .unwrap();
            unsafe { std::alloc::dealloc(datap, layout) };
        } else {
            device.noc_broadcast(0, write.addr as u64, data).unwrap();
        };
    }

    data
}

pub fn load_file_all(device: &Chip, kernel: PathBuf) -> KernelData {
    let kernel = std::fs::read(kernel).unwrap();
    load_bin_all(device, &kernel)
}

fn load_bins(device: &Chip, cores: &[(u8, u8)], elf: &[u8]) -> KernelData {
    let data = preload_bin(elf);

    for core in cores {
        let x = core.0;
        let y = core.1;

        for write in &data.writes {
            let data = write.data.0.as_ref();
            // for i in 0..data.len() / 4 {
            // let data = u32::from_le_bytes([
            // data[i * 4],
            // data[i * 4 + 1],
            // data[i * 4 + 2],
            // data[i * 4 + 3],
            // ]);
            // device
            // .noc_write32(0, x, y, write.addr as u64 + i as u64 * 4, data)
            // .unwrap();
            // }

            if data.as_ptr().align_offset(std::mem::align_of::<u32>()) != 0 {
                let layout = std::alloc::Layout::array::<u8>(data.len())
                    .unwrap()
                    .align_to(std::mem::align_of::<u32>())
                    .unwrap();
                let datap = unsafe { std::alloc::alloc(layout) };
                let new_data = unsafe { std::slice::from_raw_parts_mut(datap, data.len()) };
                new_data.copy_from_slice(data);
                device
                    .noc_write(0, x, y, write.addr as u64, new_data)
                    .unwrap();
                unsafe { std::alloc::dealloc(datap, layout) };
            } else {
                device.noc_write(0, x, y, write.addr as u64, data).unwrap();
            };

            // Readback
            let mut readback_data = vec![0; write.data.0.len()];
            device
                .noc_read(0, x, y, write.addr as u64, &mut readback_data)
                .unwrap();
            // for i in 0..data.len() / 4 {
            //     let data = device
            //         .noc_read32(0, x, y, write.addr as u64 + i as u64 * 4)
            //         .unwrap();
            //     readback_data[i * 4..i * 4 + 4].copy_from_slice(&data.to_le_bytes());
            // }
            debug_assert_eq!(readback_data.as_slice(), write.data.0.as_ref());
        }
    }

    data
}

pub fn load_files(device: &Chip, cores: &[(u8, u8)], kernel: PathBuf) -> KernelData {
    let kernel = std::fs::read(kernel).unwrap();
    load_bins(device, cores, &kernel)
}

fn load_bin<'a>(device: &'a Chip, x: u8, y: u8, elf: &[u8]) -> Kernel<'a> {
    let kernel = load_bins(device, &[(x, y)], elf);
    Kernel {
        device,
        x,
        y,
        data: kernel,
        start_sync: None,
        core_data: (Vec::new(), None),
    }
}

pub fn load_file(device: &Chip, x: u8, y: u8, kernel: PathBuf) -> Kernel {
    let kernel = std::fs::read(kernel).unwrap();
    load_bin(device, x, y, &kernel)
}
