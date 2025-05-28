use std::{collections::HashMap, path::PathBuf};

use goblin::elf::program_header;
use luwen::luwen_core::Arch;
use tensix_builder::{CacheEnable, Rewrite};

use crate::chip::{
    noc::{NocId, NocInterface, Tile},
    Chip,
};

const BRISC_SOFT_RESET: u32 = 1 << 11;
const TRISC_SOFT_RESETS: u32 = (1 << 12) | (1 << 13) | (1 << 14);
const NCRISC_SOFT_RESET: u32 = 1 << 18;

pub fn reset_to_default(device: &mut Chip) {
    device.go_idle();
    device.deassert_riscv_reset();

    // Put tensix back under soft reset
    device.noc_broadcast32(
        NocId::Noc0,
        0xFFB121B0,
        BRISC_SOFT_RESET | TRISC_SOFT_RESETS | NCRISC_SOFT_RESET,
    )
}

pub fn lower_clocks(device: &mut Chip) {
    device.go_idle();
}

pub fn raise_clocks(device: &mut Chip) {
    device.go_busy();
}

pub fn start_all(device: &mut Chip, keep_triscs_under_reset: bool, stagger_start: bool) {
    let staggered_start_enable: u32 = if stagger_start { 1 << 31 } else { 0 };

    let soft_reset_value = if keep_triscs_under_reset {
        NCRISC_SOFT_RESET | TRISC_SOFT_RESETS | staggered_start_enable
    } else {
        NCRISC_SOFT_RESET | staggered_start_enable
    };

    // Take cores out of reset
    device.noc_broadcast32(NocId::Noc0, 0xFFB121B0, soft_reset_value);
}

pub fn stop_all(device: &mut Chip) {
    lower_clocks(device);

    device.noc_broadcast32(
        NocId::Noc0,
        0xFFB121B0,
        BRISC_SOFT_RESET | TRISC_SOFT_RESETS | NCRISC_SOFT_RESET,
    );
}

pub fn start(device: &mut Chip, core: Tile, keep_triscs_under_reset: bool, stagger_start: bool) {
    let staggered_start_enable: u32 = if stagger_start { 1 << 31 } else { 0 };

    let soft_reset_value = if keep_triscs_under_reset {
        NCRISC_SOFT_RESET | TRISC_SOFT_RESETS | staggered_start_enable
    } else {
        NCRISC_SOFT_RESET | staggered_start_enable
    };

    // Take cores out of reset
    device.noc_write32(NocId::Noc0, core, 0xFFB121B0, soft_reset_value);
    let readback = device.noc_read32(NocId::Noc0, core, 0xFFB121B0);
    debug_assert_eq!(
        readback, soft_reset_value,
        "Failed to start core tried to write {soft_reset_value:x} != {readback:x} "
    );
}

pub fn easy_start(device: &mut Chip, core: Tile) {
    start(device, core, true, true);
}

pub fn easy_start_all(device: &mut Chip) {
    start_all(device, true, true);
}

pub fn stop(device: &mut Chip, core: Tile) {
    let soft_reset_value = BRISC_SOFT_RESET | TRISC_SOFT_RESETS | NCRISC_SOFT_RESET;
    device.noc_write32(NocId::Noc0, core, 0xFFB121B0, soft_reset_value);
    let readback = device.noc_read32(NocId::Noc0, core, 0xFFB121B0);
    debug_assert_eq!(
        readback,
        soft_reset_value,
        "Failed to stop core tried to write {soft_reset_value:x} to {}:{} != {readback:x}",
        core.get(NocId::Noc0).0,
        core.get(NocId::Noc0).1
    );
}

#[derive(Clone)]
#[repr(align(16))]
pub struct Alignment16(pub Box<[u8]>);

#[derive(Clone)]
pub struct KernelBytes {
    pub addr: u32,
    pub data: Alignment16,
}

impl KernelBytes {
    pub fn len(&self) -> usize {
        self.data.0.len()
    }
}

#[derive(Clone, PartialEq)]
pub struct CoreData {
    pub entry: u32,
    pub state: u32,
    pub pc: u32,
}

#[derive(Clone)]
pub struct KernelBinData {
    pub start_sync: Option<u32>,
    pub brisc_state: CoreData,
    pub ncrisc_state: CoreData,
    pub trisc0_state: CoreData,
    pub trisc1_state: CoreData,
    pub trisc2_state: CoreData,

    pub data_start: Option<u32>,

    pub core_data_cache: (
        Option<u32>,
        Vec<(String, u32, u32, Option<PanicData>)>,
        Option<PanicData>,
    ),
}

#[derive(Clone)]
pub struct KernelData {
    pub sym_table: HashMap<String, u64>,
    pub writes: Vec<KernelBytes>,
}

impl KernelData {
    pub fn load(&self, chip: &mut Chip, noc_id: NocId, tile: Tile) {
        for write in &self.writes {
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
                chip.noc_write(noc_id, tile, write.addr as u64, new_data);
                unsafe { std::alloc::dealloc(datap, layout) };
            } else {
                chip.noc_write(noc_id, tile, write.addr as u64, data);
            };

            // Readback
            let mut readback_data = vec![0; write.data.0.len()];
            chip.noc_read(noc_id, tile, write.addr as u64, &mut readback_data);
            debug_assert_eq!(readback_data.as_slice(), write.data.0.as_ref());
        }
    }

    pub fn load_all(&self, chip: &mut Chip, noc_id: NocId) {
        for write in &self.writes {
            let data = write.data.0.as_ref();
            if data.as_ptr().align_offset(std::mem::align_of::<u32>()) != 0 {
                let layout = std::alloc::Layout::array::<u8>(data.len())
                    .unwrap()
                    .align_to(std::mem::align_of::<u32>())
                    .unwrap();
                let datap = unsafe { std::alloc::alloc(layout) };
                let new_data = unsafe { std::slice::from_raw_parts_mut(datap, data.len()) };
                new_data.copy_from_slice(data);
                chip.noc_broadcast(noc_id, write.addr as u64, new_data);
                unsafe { std::alloc::dealloc(datap, layout) };
            } else {
                chip.noc_broadcast(noc_id, write.addr as u64, data);
            };

            // Readback
            for tensix in 0..chip.tensix_count() {
                let mut readback_data = vec![0; write.data.0.len()];
                chip.noc_read(
                    noc_id,
                    chip.tensix(tensix),
                    write.addr as u64,
                    &mut readback_data,
                );
                debug_assert_eq!(readback_data.as_slice(), write.data.0.as_ref());
            }
        }
    }
}

pub struct Kernel {
    pub device: Chip,
    pub core: Tile,
    pub data: KernelData,
}

pub struct KernelBin {
    pub kernel: Kernel,
    pub data: KernelBinData,
}

impl KernelBinData {
    pub fn state_vec(&self) -> Vec<(String, CoreData)> {
        vec![
            ("BRISC".to_string(), self.brisc_state.clone()),
            ("NCRISC".to_string(), self.ncrisc_state.clone()),
            ("TRISC0".to_string(), self.trisc0_state.clone()),
            ("TRISC1".to_string(), self.trisc1_state.clone()),
            ("TRISC2".to_string(), self.trisc2_state.clone()),
        ]
    }

    pub fn start_sync(
        &mut self,
        chip: &mut Chip,
        noc_id: NocId,
        tile: Tile,
        data: &KernelData,
    ) -> bool {
        if self
            .value_vec(chip, noc_id, tile, data)
            .1
            .into_iter()
            .all(|v| v.1 == 0)
        {
            if let Some(sync) = self.start_sync {
                let mut sync_value = chip.noc_read32(noc_id, tile, sync as u64);
                if sync_value != 3 {
                    if sync_value == 1 {
                        chip.noc_write32(noc_id, tile, sync as u64, 2);
                        sync_value = chip.noc_read32(noc_id, tile, sync as u64);
                    }

                    return sync_value == 3;
                }
            }
        }

        // If there isn't a start sync point then we just assume we are g2g
        // If the cores already look started then we just assume we are g2g
        true
    }

    fn print_core_panic_data(
        &mut self,
        chip: &mut Chip,
        noc_id: NocId,
        tile: Tile,
        name: &str,
        panic: &PanicData,
    ) -> bool {
        if panic.panicked {
            tracing::error!("{name} Panicked");
            let mut buf = vec![0; panic.filename_len as usize];
            tracing::error!("filename_len: {}", panic.filename_len);
            chip.noc_read(noc_id, tile, panic.filename_addr as u64, &mut buf);

            let filename = String::from_utf8(buf).unwrap();
            tracing::error!("{}; {}", filename, panic.line);

            let mut buf = vec![0; panic.message_len as usize];
            tracing::error!("message_len: {}", panic.message_len);
            chip.noc_read(noc_id, tile, panic.message_addr as u64, &mut buf);

            // TODO(drosen): For some reason the message is not completely valid utf-8
            // Also a long message doesn't seem to work...
            let message = String::from_utf8_lossy(&buf);
            tracing::error!("{}", message);

            return true;
        }
        return false;
    }

    pub fn read_panic(
        &mut self,
        name: &str,
        chip: &mut Chip,
        noc_id: NocId,
        tile: Tile,
        data: &KernelData,
    ) -> Option<PanicData> {
        let name = format!("PANIC_DATA_{name}");
        if let Some(postcode_mapping) = data.sym_table.get(&name) {
            let mut data = [0; size_of::<PanicData>()];
            chip.noc_read(noc_id, tile, *postcode_mapping, &mut data);

            Some(unsafe { std::mem::transmute_copy(&data) })
        } else {
            None
        }
    }

    fn value_vec(
        &mut self,
        chip: &mut Chip,
        noc_id: NocId,
        tile: Tile,
        data: &KernelData,
    ) -> (
        Option<u32>,
        Vec<(String, u32, u32, Option<PanicData>)>,
        Option<PanicData>,
    ) {
        let sync = self
            .start_sync
            .map(|sync| chip.noc_read32(noc_id, tile, sync as u64));
        let mut values = Vec::new();
        for state in self.state_vec() {
            let pd = self.read_panic(&state.0, chip, noc_id, tile, data);
            values.push((
                state.0,
                chip.noc_read32(noc_id, tile, (state.1).state as u64),
                chip.noc_read32(noc_id, tile, (state.1).pc as u64),
                pd,
            ));
        }

        (
            sync,
            values,
            self.read_panic("UNKNOWN", chip, noc_id, tile, data),
        )
    }

    pub fn print_state_diff(
        &mut self,
        chip: &mut Chip,
        noc_id: NocId,
        tile: Tile,
        data: &KernelData,
    ) {
        self.maybe_print_state(chip, noc_id, tile, data, false);
    }

    pub fn print_state(&mut self, chip: &mut Chip, noc_id: NocId, tile: Tile, data: &KernelData) {
        self.maybe_print_state(chip, noc_id, tile, data, true);
    }

    pub fn maybe_print_state(
        &mut self,
        chip: &mut Chip,
        noc_id: NocId,
        tile: Tile,
        data: &KernelData,
        force: bool,
    ) {
        let state = self.value_vec(chip, noc_id, tile, data);
        if !force {
            if (state.0, &state.1) == (self.core_data_cache.0, &self.core_data_cache.1) {
                return;
            }
        }

        tracing::info!(
            "State for: {}[{}]: {:?}{{{:?}}}",
            chip.arch(),
            chip.id(),
            tile,
            noc_id
        );

        if let Some(sync) = state.0 {
            tracing::info!("SYNC: {}", sync);
            if sync != 3 {
                self.core_data_cache = state;
                return;
            }
        }

        if let Some(postcode_mapping) = data.sym_table.get("NOC_DEBUG") {
            let brc = chip.noc_read32(noc_id, tile, *postcode_mapping);
            tracing::info!("noc_debug: 0x{:x}", brc);
        }

        for (name, state, pc, panic) in &state.1 {
            if let Some(panic) = panic {
                self.print_core_panic_data(chip, noc_id, tile, name, panic);
            }
            tracing::info!("{} {{ STATE: {}, POSTCODE: {:x} }}", name, state, pc);
        }
        if let Some(panic) = &state.2 {
            self.print_core_panic_data(chip, noc_id, tile, "UNKNOWN", panic);
        }

        self.core_data_cache = state;
    }

    pub fn check_panic(
        &mut self,
        chip: &mut Chip,
        noc_id: NocId,
        tile: Tile,
        data: &KernelData,
    ) -> bool {
        let (_sync, cores, _panic) = self.value_vec(chip, noc_id, tile, data);
        cores.iter().any(|v| v.1 == 6)
    }

    pub fn wait(&mut self, chip: &mut Chip, noc_id: NocId, tile: Tile, data: &KernelData) {
        while !self.all_complete(chip, noc_id, tile) {
            std::thread::sleep(std::time::Duration::from_millis(10));
            self.print_state_diff(chip, noc_id, tile, data);
        }

        stop(chip, tile);

        self.print_state_diff(chip, noc_id, tile, data)
    }

    /// Marked by all cores either having completed... or not started
    pub fn all_complete(&mut self, chip: &mut Chip, noc_id: NocId, tile: Tile) -> bool {
        let states = self.state_vec();

        let state_value = states
            .iter()
            .map(|v| chip.noc_read32(noc_id, tile, (v.1).state as u64))
            .collect::<Vec<_>>();

        let complete_count = state_value.iter().filter(|v| **v >= 3).count();
        let not_started_count = state_value.iter().filter(|v| **v == 0).count();

        complete_count > 0 && complete_count + not_started_count == states.len()
    }
}

impl Kernel {
    pub fn new(device: Chip, core: Tile, data: KernelData) -> Self {
        Self { device, core, data }
    }
}

// TODO(drosen): This should be a shared definition
#[repr(C)]
#[derive(PartialEq, Debug, Clone)]
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

impl KernelBin {
    pub fn read32(&mut self, addr: u64) -> u32 {
        self.kernel.read32(addr)
    }

    pub fn write32(&mut self, addr: u64, value: u32) {
        self.kernel.write32(addr, value)
    }

    pub fn read(&mut self, addr: u64, data: &mut [u8]) {
        self.kernel.read(addr, data)
    }

    pub fn write(&mut self, addr: u64, data: &[u8]) {
        self.kernel.write(addr, data)
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
            tracing::error!("{name} Panicked");
            let mut buf = vec![0; panic.filename_len as usize];
            tracing::error!("filename_len: {}", panic.filename_len);
            self.read(panic.filename_addr as u64, &mut buf);

            let filename = String::from_utf8(buf).unwrap();
            tracing::error!("{}; {}", filename, panic.line);

            let mut buf = vec![0; panic.message_len as usize];
            tracing::error!("message_len: {}", panic.message_len);
            self.read(panic.message_addr as u64, &mut buf);

            // TODO(drosen): For some reason the message is not completely valid utf-8
            // Also a long message doesn't seem to work...
            let message = String::from_utf8_lossy(&buf);
            tracing::error!("{}", message);

            return true;
        }
        return false;
    }

    pub fn read_panic(&mut self, name: &str) -> Option<PanicData> {
        let name = format!("PANIC_DATA_{name}");
        if let Some(postcode_mapping) = self.kernel.data.sym_table.get(&name) {
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
        self.data.maybe_print_state(
            &mut self.kernel.device,
            NocId::Noc0,
            self.kernel.core,
            &self.kernel.data,
            force,
        );
    }

    pub fn check_panic(&mut self) -> bool {
        let (cores, _panic) = self.value_vec();
        cores.iter().any(|v| v.1 == 6)
    }

    pub fn wait(&mut self) {
        while !self.all_complete() {
            std::thread::sleep(std::time::Duration::from_millis(10));
            self.print_state_diff();
        }

        stop(&mut self.kernel.device, self.kernel.core);

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
        self.kernel.set_entry(
            self.data.ncrisc_state.entry,
            self.data.trisc0_state.entry,
            self.data.trisc1_state.entry,
            self.data.trisc2_state.entry,
        );
    }
}

impl Kernel {
    pub fn read(&mut self, addr: u64, data: &mut [u8]) {
        self.device.noc_read(NocId::Noc0, self.core, addr, data);
    }

    pub fn write(&mut self, addr: u64, data: &[u8]) {
        self.device.noc_write(NocId::Noc0, self.core, addr, data);
    }

    pub fn read32(&mut self, addr: u64) -> u32 {
        self.device.noc_read32(NocId::Noc0, self.core, addr)
    }

    pub fn write32(&mut self, addr: u64, value: u32) {
        self.device.noc_write32(NocId::Noc0, self.core, addr, value);
    }

    pub fn set_entry(&mut self, ncrisc: u32, trisc0: u32, trisc1: u32, trisc2: u32) {
        pub const TENSIX_CFG_BASE: u32 = 4293853184;

        pub const TRISC0_RESET_PC_ADDR: u32 = 158;
        pub const TRISC1_RESET_PC_ADDR: u32 = 159;
        pub const TRISC2_RESET_PC_ADDR: u32 = 160;
        pub const TRISC_RESET_PC_OVERRIDE_EN: u32 = 161;
        pub const NCRISC_RESET_PC_ADDR: u32 = 162;
        pub const NCRISC_RESET_PC_OVERRIDE_EN: u32 = 163;

        self.write32((TENSIX_CFG_BASE + TRISC0_RESET_PC_ADDR) as u64, trisc0);
        self.write32((TENSIX_CFG_BASE + TRISC1_RESET_PC_ADDR) as u64, trisc1);
        self.write32((TENSIX_CFG_BASE + TRISC2_RESET_PC_ADDR) as u64, trisc2);
        self.write32((TENSIX_CFG_BASE + TRISC_RESET_PC_OVERRIDE_EN) as u64, 0b111);

        self.write32((TENSIX_CFG_BASE + NCRISC_RESET_PC_ADDR) as u64, ncrisc);
        self.write32((TENSIX_CFG_BASE + NCRISC_RESET_PC_OVERRIDE_EN) as u64, 1);
    }
}

fn preload(elf: &[u8]) -> KernelData {
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
        sym_table: sym_table
            .into_iter()
            .map(|v| (v.0.to_string(), v.1))
            .collect(),
        writes,
    }
}

fn preload_lib(elf: &[u8]) -> KernelData {
    preload(elf)
}

fn preload_bin(elf: &[u8]) -> (KernelData, KernelBinData) {
    let data = preload(elf);
    let bin_data = KernelBinData {
        start_sync: data.sym_table.get("START_SYNC").map(|v| *v as u32),

        brisc_state: CoreData {
            entry: data.sym_table["__brisc_start"] as u32,
            state: data.sym_table["STATE_BRISC"] as u32,
            pc: data.sym_table["POSTCODE_BRISC"] as u32,
        },

        ncrisc_state: CoreData {
            entry: data.sym_table["__ncrisc_start"] as u32,
            state: data.sym_table["STATE_NCRISC"] as u32,
            pc: data.sym_table["POSTCODE_NCRISC"] as u32,
        },

        trisc0_state: CoreData {
            entry: data.sym_table["__trisc0_start"] as u32,
            state: data.sym_table["STATE_TRISC0"] as u32,
            pc: data.sym_table["POSTCODE_TRISC0"] as u32,
        },

        trisc1_state: CoreData {
            entry: data.sym_table["__trisc1_start"] as u32,
            state: data.sym_table["STATE_TRISC1"] as u32,
            pc: data.sym_table["POSTCODE_TRISC1"] as u32,
        },

        trisc2_state: CoreData {
            entry: data.sym_table["__trisc2_start"] as u32,
            state: data.sym_table["STATE_TRISC2"] as u32,
            pc: data.sym_table["POSTCODE_TRISC2"] as u32,
        },

        data_start: data.sym_table.get("__firmware_end").map(|v| *v as u32),
        core_data_cache: (None, Vec::new(), None),
    };

    (data, bin_data)
}

fn load_bin_all(device: &mut Chip, elf: &[u8]) -> (KernelData, KernelBinData) {
    let (data, bin_data) = preload_bin(elf);

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
            device.noc_broadcast(NocId::Noc0, write.addr as u64, new_data);
            unsafe { std::alloc::dealloc(datap, layout) };
        } else {
            device.noc_broadcast(NocId::Noc0, write.addr as u64, data);
        };
    }

    (data, bin_data)
}

pub fn load_file_all(device: &mut Chip, kernel: PathBuf) -> (KernelData, KernelBinData) {
    let kernel = std::fs::read(kernel).unwrap();
    load_bin_all(device, &kernel)
}

fn load_bins(device: &mut Chip, cores: &[Tile], elf: &[u8]) -> (KernelData, KernelBinData) {
    let data = preload_bin(elf);

    for core in cores.iter().copied() {
        data.0.load(device, NocId::Noc0, core);
    }

    data
}

pub fn load_files(
    device: &mut Chip,
    cores: &[Tile],
    kernel: PathBuf,
) -> (KernelData, KernelBinData) {
    let kernel = std::fs::read(kernel).unwrap();
    load_bins(device, cores, &kernel)
}

fn load_bin(mut device: Chip, core: Tile, elf: &[u8]) -> KernelBin {
    let kernel_data = load_bins(&mut device, &[core], elf);
    let kernel = Kernel::new(device, core, kernel_data.0);
    KernelBin {
        kernel,
        data: kernel_data.1,
    }
}

pub fn load_bin_file(device: Chip, core: Tile, kernel: PathBuf) -> KernelBin {
    let kernel = std::fs::read(kernel).unwrap();
    load_bin(device, core, &kernel)
}

pub fn load_lib_file(device: Chip, core: Tile, kernel: PathBuf) -> KernelBin {
    let kernel = std::fs::read(kernel).unwrap();
    load_bin(device, core, &kernel)
}

pub struct LoadOptions {
    pub no_wait: bool,
    pub build_std: bool,
    pub verbose: bool,
    pub lto: bool,
    pub use_cache: tensix_builder::CacheEnable,
    pub base_path: PathBuf,
    pub path: String,
    pub profile: String,
    pub default_features: bool,
    pub stack_probes: bool,
    pub hide_output: bool,
}

impl LoadOptions {
    pub fn new(base_path: &std::path::Path) -> Self {
        Self {
            no_wait: false,
            build_std: false,
            verbose: false,
            use_cache: CacheEnable::Disabled,
            lto: false,
            base_path: base_path.to_path_buf(),
            path: String::new(),
            profile: "release".to_string(),
            default_features: true,
            stack_probes: false,
            hide_output: false,
        }
    }
}

impl LoadOptions {
    pub fn no_wait(mut self, no_wait: bool) -> Self {
        self.no_wait = no_wait;
        self
    }

    pub fn verbose(mut self, verbose: bool) -> Self {
        self.verbose = verbose;
        self
    }

    pub fn build_std(mut self, build_std: bool) -> Self {
        self.build_std = build_std;
        self
    }

    pub fn use_cache(mut self, cache: CacheEnable) -> Self {
        self.use_cache = cache;
        self
    }

    pub fn hide_output(mut self) -> Self {
        self.hide_output = true;
        self
    }

    pub fn lto(mut self, lto: bool) -> Self {
        self.lto = lto;
        self
    }

    pub fn path(mut self, path: &str) -> Self {
        let path = std::path::Path::new(path);
        self.path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.base_path.join(path).to_path_buf()
        }
        .to_string_lossy()
        .to_string();

        self
    }

    pub fn profile(mut self, profile: &str) -> Self {
        self.profile = profile.to_string();
        self
    }

    pub fn default_features(mut self, df: bool) -> Self {
        self.default_features = df;
        self
    }

    pub fn stack_probes(mut self, probe: bool) -> Self {
        self.stack_probes = probe;
        self
    }
}

pub enum BinOrLib {
    Lib(KernelData),
    Bin {
        data: KernelData,
        bin_data: KernelBinData,
    },
}

pub fn build_kernel(
    name: &str,
    arch: Arch,
    options: LoadOptions,
    custom_link: Option<(String, Vec<Rewrite>)>,
) -> BinOrLib {
    let arch = match arch {
        luwen::luwen_core::Arch::Grayskull => tensix_builder::StandardTarget::Grayskull,
        luwen::luwen_core::Arch::Wormhole => tensix_builder::StandardTarget::Wormhole,
        luwen::luwen_core::Arch::Blackhole => tensix_builder::StandardTarget::Blackhole,
        luwen::luwen_core::Arch::Unknown(_) => todo!(),
    };

    let arch = if let Some((link, rewrites)) = custom_link {
        tensix_builder::TensixTarget::Custom {
            name: format!("{arch}-custom"),
            target_def: tensix_builder::StandardTargetOrCustom::Standard((arch, rewrites)),
            linker_script: link,
        }
    } else {
        tensix_builder::TensixTarget::Standard(arch)
    };

    let profile = match options.profile.as_str() {
        "debug" => tensix_builder::CargoProfile::Debug,
        "release" => tensix_builder::CargoProfile::Release,
        other => tensix_builder::CargoProfile::Other(other.to_string()),
    };

    let kernel = tensix_builder::build_kernel(
        if options.path.is_empty() {
            options.base_path.to_string_lossy().to_string()
        } else {
            options.path
        },
        tensix_builder::CargoOptions {
            target: arch.clone(),
            profile,
            lto: options.lto,
            use_cache: options.use_cache,
            verbose: options.verbose,
            build_std: options.build_std,
            default_features: options.default_features,
            stack_probes: options.stack_probes,
            kernel_name: name.to_string(),
            hide_output: options.hide_output,
        },
    );

    let elf = std::fs::read(kernel.path).unwrap();
    if kernel.bin {
        let (data, bin_data) = preload_bin(&elf);
        BinOrLib::Bin { data, bin_data }
    } else {
        BinOrLib::Lib(preload_lib(&elf))
    }
}

pub fn quick_load(name: &str, mut device: Chip, core: Tile, options: LoadOptions) -> KernelBin {
    device.start();

    let arch = match device.arch() {
        luwen::luwen_core::Arch::Grayskull => tensix_builder::StandardTarget::Grayskull,
        luwen::luwen_core::Arch::Wormhole => tensix_builder::StandardTarget::Wormhole,
        luwen::luwen_core::Arch::Blackhole => tensix_builder::StandardTarget::Blackhole,
        luwen::luwen_core::Arch::Unknown(_) => todo!(),
    };

    let profile = match options.profile.as_str() {
        "debug" => tensix_builder::CargoProfile::Debug,
        "release" => tensix_builder::CargoProfile::Release,
        other => tensix_builder::CargoProfile::Other(other.to_string()),
    };

    let kernel = tensix_builder::build_kernel(
        if options.path.is_empty() {
            options.base_path.to_string_lossy().to_string()
        } else {
            options.path
        },
        tensix_builder::CargoOptions {
            target: tensix_builder::TensixTarget::Standard(arch.clone()),
            profile,
            lto: options.lto,
            use_cache: options.use_cache,
            verbose: options.verbose,
            build_std: options.build_std,
            default_features: options.default_features,
            stack_probes: options.stack_probes,
            kernel_name: name.to_string(),
            hide_output: options.hide_output,
        },
    );

    println!("Loading binary to {arch}");

    stop(&mut device, core);

    assert!(kernel.bin, "Can only quick load binary");
    let mut kernel = load_bin_file(device.dupe().unwrap(), core, kernel.path);

    easy_start(&mut device, core);

    if kernel.data.start_sync.is_some() {
        println!("Waiting for kernel start");

        kernel.print_state();

        while !kernel.start_sync() {
            if !kernel.all_complete() {
                kernel.print_state_diff();
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        if !options.no_wait {
            println!("Waiting for kernel complete");
        }
    } else {
        if !options.no_wait {
            println!("Waiting for kernel complete");
        }
        kernel.print_state();
    };

    if options.no_wait {
        return kernel;
    }

    kernel.wait();

    kernel
}
