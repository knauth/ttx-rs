use std::{
    collections::{BTreeSet, HashMap},
    hash::Hash,
    sync::{
        atomic::{AtomicBool, AtomicUsize},
        Mutex,
    },
};

use luwen::{
    luwen_core::Arch,
    ttkmd_if::{self, tlb::MemoryType, DmaBuffer, PciDevice, PciError, Tlb},
};

use crate::loader::NocGrid;

pub mod blackhole;
pub mod field;
pub mod grayskull;
pub mod wormhole;

pub static ARC_LOCK: Mutex<Vec<Mutex<()>>> = Mutex::new(Vec::new());
pub static IDLE: Mutex<Vec<AtomicBool>> = Mutex::new(Vec::new());

pub struct Chip {
    pub device: PciDevice,
    pub noc: NocGrid,
    pub tensix_l1_size: u32,
}

pub fn get_mask(chip: &mut Chip) -> u32 {
    match chip.device.arch {
        Arch::Grayskull => grayskull::arc_msg(
            chip,
            &grayskull::ArcMsg::GetHarvesting,
            true,
            std::time::Duration::from_secs(1),
            5,
            3,
            &grayskull::ArcMsgAddr {
                scratch_base: 0x1ff30060,
                arc_misc_cntl: 0x1ff30100,
            },
        )
        .unwrap()
        .arg(),
        Arch::Wormhole => wormhole::arc_msg(
            chip,
            &wormhole::ArcMsg::GetHarvesting,
            true,
            std::time::Duration::from_secs(1),
            5,
            3,
            &wormhole::ArcMsgAddr {
                scratch_base: 0x1ff30060,
                arc_misc_cntl: 0x1ff30100,
            },
        )
        .unwrap()
        .arg(),
        Arch::Blackhole => 0,
    }
}

pub fn open(index: usize) -> Result<Chip, ttkmd_if::PciOpenError> {
    let device = PciDevice::open(index)?;

    device.detect_ffffffff_read(None)?;

    let mut chip = Chip {
        noc: match device.arch {
            Arch::Grayskull => crate::loader::grayskull::get_grid(0),
            Arch::Wormhole => crate::loader::wormhole::get_grid(0),
            Arch::Blackhole => crate::loader::blackhole::get_grid(0),
        },
        tensix_l1_size: match device.arch {
            Arch::Grayskull => crate::loader::grayskull::get_tensix_l1_size(),
            Arch::Wormhole => crate::loader::wormhole::get_tensix_l1_size(),
            Arch::Blackhole => crate::loader::blackhole::get_tensix_l1_size(),
        },
        device,
    };

    let harvest = get_mask(&mut chip);
    chip.noc = match chip.device.arch {
        Arch::Grayskull => crate::loader::grayskull::get_grid(harvest),
        Arch::Wormhole => crate::loader::wormhole::get_grid(harvest),
        Arch::Blackhole => crate::loader::blackhole::get_grid(harvest),
    };

    Ok(chip)
}

pub struct FixedTlb(Tlb);

impl Hash for FixedTlb {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.x_end.hash(state);
        self.0.y_end.hash(state);
        if self.0.mcast {
            self.0.x_start.hash(state);
            self.0.y_start.hash(state);
        }
        self.0.noc_sel.hash(state);
        self.0.mcast.hash(state);
        self.0.ordering.hash(state);
        self.0.linked.hash(state);
        self.0.use_static_vc.hash(state);
        self.0.stream_header.hash(state);
        self.0.static_vc.hash(state);

        self.0.stride.hash(state);
    }
}

impl Eq for FixedTlb {}

impl PartialEq for FixedTlb {
    fn eq(&self, other: &FixedTlb) -> bool {
        self.0.x_end.eq(&other.0.x_end)
            && self.0.y_end.eq(&other.0.y_end)
            && if self.0.mcast {
                self.0.y_start.eq(&other.0.y_start) && self.0.x_start.eq(&other.0.x_start)
            } else {
                true
            }
            && self.0.noc_sel.eq(&other.0.noc_sel)
            && self.0.mcast.eq(&other.0.mcast)
            && self.0.ordering.eq(&other.0.ordering)
            && self.0.linked.eq(&other.0.linked)
            && self.0.use_static_vc.eq(&other.0.use_static_vc)
            && self.0.stream_header.eq(&other.0.stream_header)
            && self.0.static_vc.eq(&other.0.static_vc)
            && self.0.stride.eq(&other.0.stride)
    }
}

pub struct HostTlbInfo {
    addr: u64,
    size: u64,
    memory: MemoryType,
}

#[derive(Default)]
pub struct ChipTlbs {
    tlb_allocation_count: Vec<AtomicUsize>,
    tlb_programming: Vec<Tlb>,
    tlb_info: Vec<HostTlbInfo>,
    unallocated_tlbs: BTreeSet<usize>,
    memory_map: HashMap<FixedTlb, BTreeSet<usize>>,
    init: bool,
}

impl ChipTlbs {
    pub fn init(&mut self, pci_device: &PciDevice) {
        if !self.init {
            self.init = true;

            let info = ttkmd_if::tlb::get_tlb_info(pci_device);
            let mut addr = 0;
            for entry in info.tlb_config {
                for _ in 0..entry.count {
                    self.tlb_info.push(HostTlbInfo {
                        addr,
                        size: entry.size,
                        memory: entry.memory_type.clone(),
                    });
                    addr += entry.size;
                }
            }

            self.tlb_allocation_count
                .resize_with(info.total_count as usize, || AtomicUsize::new(0));
            self.tlb_programming
                .resize_with(info.total_count as usize, || Tlb::default());
            self.unallocated_tlbs = (0..info.total_count as usize).collect();
        }
    }
}

static CHIP_TLBS: Mutex<Vec<ChipTlbs>> = Mutex::new(Vec::new());

#[derive(Debug)]
pub struct TlbAllocation {
    address: u64,
    size: u64,
    chip_index: usize,
    tlb_index: usize,
}

impl Drop for TlbAllocation {
    fn drop(&mut self) {
        let lock = CHIP_TLBS.lock();

        if let Ok(mut lock) = lock {
            let result = match lock[self.chip_index].tlb_allocation_count[self.tlb_index]
                .fetch_update(
                    std::sync::atomic::Ordering::SeqCst,
                    std::sync::atomic::Ordering::SeqCst,
                    |v| Some(v.saturating_sub(1)),
                ) {
                Ok(v) | Err(v) => v,
            };

            if result == 0 {
                let base_tlb = lock[self.chip_index].tlb_programming[self.tlb_index].clone();
                lock[self.chip_index]
                    .memory_map
                    .entry(FixedTlb(base_tlb))
                    .or_default()
                    .remove(&self.tlb_index);

                lock[self.chip_index]
                    .unallocated_tlbs
                    .insert(self.tlb_index);
            }
        }
    }
}

impl Drop for Chip {
    fn drop(&mut self) {
        crate::loader::stop_all(self);

        let mut idle = if let Ok(idle) = IDLE.lock() {
            idle
        } else {
            return;
        };

        while idle.len() <= self.device.id {
            idle.push(AtomicBool::new(true));
        }
        idle[self.device.id].store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

impl Chip {
    pub fn arch(&self) -> Arch {
        self.device.arch
    }

    pub fn start(&self) {
        let mut idle = if let Ok(idle) = IDLE.lock() {
            idle
        } else {
            return;
        };

        while idle.len() <= self.device.id {
            idle.push(AtomicBool::new(true));
        }

        if idle[self.device.id].load(std::sync::atomic::Ordering::SeqCst) {
            crate::loader::reset_to_default(self);
            crate::loader::raise_clocks(self);
            idle[self.device.id].store(false, std::sync::atomic::Ordering::SeqCst);
        }
    }

    pub fn stop(&self, force: bool) {
        let mut idle = if let Ok(idle) = IDLE.lock() {
            idle
        } else {
            return;
        };

        while idle.len() <= self.device.id {
            idle.push(AtomicBool::new(true));
        }

        if force || !idle[self.device.id].load(std::sync::atomic::Ordering::SeqCst) {
            crate::loader::stop_all(self);
            idle[self.device.id].store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }

    pub fn get_tlb(&self, tlb: Tlb, size: u64) -> Option<TlbAllocation> {
        let mut tlbs = if let Ok(tlbs) = CHIP_TLBS.lock() {
            tlbs
        } else {
            println!("Panic'd");
            return None;
        };

        while tlbs.len() <= self.device.id {
            tlbs.push(ChipTlbs::default());
        }
        tlbs[self.device.id].init(&self.device);

        let tlb_index = match self.device.arch {
            Arch::Grayskull => 184,
            Arch::Wormhole => 184,
            Arch::Blackhole => 190,
        };

        let allocated_count = tlbs[self.device.id].tlb_allocation_count[tlb_index]
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if allocated_count == 0 {
            tlbs[self.device.id].unallocated_tlbs.remove(&tlb_index);
            tlbs[self.device.id].tlb_programming[tlb_index] = tlb.clone();
            tlbs[self.device.id]
                .memory_map
                .entry(FixedTlb(tlb.clone()))
                .or_default()
                .insert(tlb_index);
            // TODO(drosen): This is really bad, rethink the set of abstractions that
            // lead to this... maybe need to make an explicity thread safe PciDevice.
            let device = unsafe { &mut *(&raw const self.device as *const _ as *mut _) };
            let (bar_addr, size) = ttkmd_if::tlb::setup_tlb(device, tlb_index as u32, tlb).unwrap();
            tlbs[self.device.id].tlb_info[tlb_index].addr = bar_addr;
            tlbs[self.device.id].tlb_info[tlb_index].size = size;

            Some(TlbAllocation {
                address: bar_addr,
                size,
                chip_index: self.device.id,
                tlb_index,
            })
        } else {
            loop {
                if tlbs[self.device.id].tlb_allocation_count[tlb_index]
                    .fetch_update(
                        std::sync::atomic::Ordering::SeqCst,
                        std::sync::atomic::Ordering::SeqCst,
                        |v| Some(v.saturating_sub(1)),
                    )
                    .is_ok()
                {
                    break;
                }
            }
            None
        }
    }

    // pub fn get_tlb(&self, tlb: Tlb, size: u64) -> Option<TlbAllocation> {
    //     let register = true;

    //     let mut tlbs = if let Ok(tlbs) = CHIP_TLBS.lock() {
    //         tlbs
    //     } else {
    //         return None;
    //     };

    //     while tlbs.len() <= self.device.id {
    //         tlbs.push(ChipTlbs::default());
    //     }
    //     tlbs[self.device.id].init(&self.device);

    //     // Don't worry about reusing tlbs just find an unallocated tlb and program it
    //     // We may notind a tlb that matches the size we want so keep track of the best case.
    //     let mut best_case = None;
    //     let mut best_size = 0;
    //     for unallocated in tlbs[self.device.id].unallocated_tlbs.iter().copied() {
    //         // println!("Checking {unallocated} for device {}", self.device.id);
    //         let info = &tlbs[self.device.id].tlb_info[unallocated];
    //         let tlb_size = info.size;

    //         if tlb_size >= size {
    //             // println!("tlb {unallocated} is big enough at {tlb_size} could have gone down to at least {size}");
    //             best_case = Some(unallocated);
    //             break;
    //         } else if tlb_size > best_size {
    //             best_case = Some(unallocated);
    //             best_size = tlb_size;
    //         }
    //     }

    //     if let Some(tlb_index) = best_case {
    //         let allocated_count = tlbs[self.device.id].tlb_allocation_count[tlb_index]
    //             .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    //         if allocated_count == 0 {
    //             // println!("{tlb_index} was unallocated");

    //             tlbs[self.device.id].unallocated_tlbs.remove(&tlb_index);
    //             tlbs[self.device.id].tlb_programming[tlb_index] = tlb.clone();
    //             tlbs[self.device.id]
    //                 .memory_map
    //                 .entry(FixedTlb(tlb.clone()))
    //                 .or_default()
    //                 .insert(tlb_index);
    //             // TODO(drosen): This is really bad, rethink the set of abstractions that
    //             // lead to this... maybe need to make an explicity thread safe PciDevice.
    //             let device = unsafe { &mut *(&raw const self.device as *const _ as *mut _) };
    //             let (bar_addr, size) =
    //                 ttkmd_if::tlb::setup_tlb(device, tlb_index as u32, tlb).unwrap();
    //             tlbs[self.device.id].tlb_info[tlb_index].addr = bar_addr;
    //             tlbs[self.device.id].tlb_info[tlb_index].size = size;
    //         } else {
    //             // println!("{tlb_index} was already allocated with {}", allocated_count);
    //         }

    //         let tlb_info = &tlbs[self.device.id].tlb_info[tlb_index];
    //         Some(TlbAllocation {
    //             address: tlb_info.addr,
    //             size: tlb_info.size,
    //             chip_index: self.device.id,
    //             tlb_index,
    //         })
    //     } else {
    //         None
    //     }
    // }

    pub fn noc_write(
        &self,
        noc_id: u8,
        x: u8,
        y: u8,
        addr: u64,
        data: &[u8],
    ) -> Result<(), PciError> {
        let mut written = 0;

        let len = data.len() as u64;

        while written < len {
            let to_write = len.saturating_sub(written);
            let tlb = loop {
                if let Some(allocation) = self.get_tlb(
                    Tlb {
                        local_offset: addr + written,
                        noc_sel: noc_id,
                        x_end: x,
                        y_end: y,
                        // TODO(drosen): BH should use posted strirct for register access
                        // TODO(drosen): All others should use relaxed
                        ordering: ttkmd_if::tlb::Ordering::STRICT,
                        ..Default::default()
                    },
                    to_write,
                ) {
                    break allocation;
                }
            };

            let programmed_tlb =
                ttkmd_if::tlb::get_tlb(&self.device, tlb.tlb_index as u32).unwrap();
            let specific_tlb_info =
                ttkmd_if::tlb::get_per_tlb_info(&self.device, tlb.tlb_index as u32);
            debug_assert_eq!(programmed_tlb.x_end, x);
            debug_assert_eq!(programmed_tlb.y_end, y);
            debug_assert_eq!(
                programmed_tlb.local_offset * specific_tlb_info.size
                    + (tlb.address - specific_tlb_info.data_base),
                addr
            );

            let to_write = std::cmp::min(tlb.size, to_write);
            self.device.write_block_no_dma(
                tlb.address as u32,
                &data[written as usize..(written as usize + to_write as usize)],
            )?;

            written += to_write;
        }

        Ok(())
    }

    pub fn noc_read(
        &self,
        noc_id: u8,
        x: u8,
        y: u8,
        addr: u64,
        data: &mut [u8],
    ) -> Result<(), PciError> {
        let mut read = 0;

        let len = data.len() as u64;

        while read < len {
            let to_read = len.saturating_sub(read);
            let tlb = loop {
                if let Some(allocation) = self.get_tlb(
                    Tlb {
                        local_offset: addr + read,
                        noc_sel: noc_id,
                        x_end: x,
                        y_end: y,
                        // TODO(drosen): BH should use posted strirct for register access
                        // TODO(drosen): All others should use relaxed
                        ordering: ttkmd_if::tlb::Ordering::STRICT,
                        ..Default::default()
                    },
                    to_read,
                ) {
                    break allocation;
                }
            };

            let programmed_tlb =
                ttkmd_if::tlb::get_tlb(&self.device, tlb.tlb_index as u32).unwrap();
            let specific_tlb_info =
                ttkmd_if::tlb::get_per_tlb_info(&self.device, tlb.tlb_index as u32);
            debug_assert_eq!(programmed_tlb.x_end, x);
            debug_assert_eq!(programmed_tlb.y_end, y);
            debug_assert_eq!(
                programmed_tlb.local_offset * specific_tlb_info.size
                    + (tlb.address - specific_tlb_info.data_base),
                addr
            );
            debug_assert_eq!(specific_tlb_info.memory_type, MemoryType::Uc);

            let to_read = std::cmp::min(tlb.size, to_read);
            self.device.read_block_no_dma(
                tlb.address as u32,
                &mut data[read as usize..(read as usize + to_read as usize)],
            )?;

            read += to_read;
        }

        Ok(())
    }

    pub fn noc_write32(
        &self,
        noc_id: u8,
        x: u8,
        y: u8,
        addr: u64,
        data: u32,
    ) -> Result<(), PciError> {
        let tlb = loop {
            if let Some(allocation) = self.get_tlb(
                Tlb {
                    local_offset: addr,
                    noc_sel: noc_id,
                    x_end: x,
                    y_end: y,
                    // TODO(drosen): BH should use posted strirct for register access
                    // TODO(drosen): All others should use relaxed
                    ordering: ttkmd_if::tlb::Ordering::STRICT,
                    ..Default::default()
                },
                4,
            ) {
                break allocation;
            }
        };

        let programmed_tlb = ttkmd_if::tlb::get_tlb(&self.device, tlb.tlb_index as u32).unwrap();
        let specific_tlb_info = ttkmd_if::tlb::get_per_tlb_info(&self.device, tlb.tlb_index as u32);

        debug_assert_eq!(programmed_tlb.x_end, x);
        debug_assert_eq!(programmed_tlb.y_end, y);
        debug_assert_eq!(
            programmed_tlb.local_offset * specific_tlb_info.size
                + (tlb.address - specific_tlb_info.data_base),
            addr
        );
        debug_assert_eq!(specific_tlb_info.memory_type, MemoryType::Uc);

        self.device.write32(tlb.address as u32, data)
    }

    pub fn noc_read32(&self, noc_id: u8, x: u8, y: u8, addr: u64) -> Result<u32, PciError> {
        let tlb = loop {
            if let Some(allocation) = self.get_tlb(
                Tlb {
                    local_offset: addr,
                    noc_sel: noc_id,
                    x_end: x,
                    y_end: y,
                    // TODO(drosen): BH should use posted strirct for register access
                    // TODO(drosen): All others should use relaxed
                    ordering: ttkmd_if::tlb::Ordering::STRICT,
                    ..Default::default()
                },
                4,
            ) {
                break allocation;
            }
        };

        // println!("Reading from {x}:{y}:{addr:x}");

        let programmed_tlb = ttkmd_if::tlb::get_tlb(&self.device, tlb.tlb_index as u32).unwrap();
        let specific_tlb_info = ttkmd_if::tlb::get_per_tlb_info(&self.device, tlb.tlb_index as u32);
        debug_assert_eq!(programmed_tlb.x_end, x);
        debug_assert_eq!(programmed_tlb.y_end, y);
        debug_assert_eq!(
            programmed_tlb.local_offset * specific_tlb_info.size
                + (tlb.address - specific_tlb_info.data_base),
            addr
        );
        debug_assert_eq!(specific_tlb_info.memory_type, MemoryType::Uc);

        self.device.read32(tlb.address as u32)
    }

    fn broadcast_grid(&self) -> (u8, u8, u8, u8) {
        let (x_start, y_start) = match self.device.arch {
            Arch::Grayskull => (0, 0),
            Arch::Wormhole => (1, 0),
            Arch::Blackhole => (0, 1),
        };

        let (grid_width, grid_height) = match self.device.arch {
            Arch::Grayskull => (13, 12),
            Arch::Wormhole => (10, 12),
            Arch::Blackhole => (17, 12),
        };

        let (x_end, y_end) = (grid_width - 1, grid_height - 1);

        (x_start, y_start, x_end, y_end)
    }

    pub fn noc_broadcast(&self, noc_id: u8, addr: u64, data: &[u8]) -> Result<(), PciError> {
        let mut written = 0;

        let len = data.len() as u64;

        let (x_start, y_start, x_end, y_end) = self.broadcast_grid();

        while written < len {
            let to_write = len.saturating_sub(written);
            let tlb = loop {
                if let Some(allocation) = self.get_tlb(
                    Tlb {
                        local_offset: addr + written,
                        noc_sel: noc_id,
                        x_start,
                        y_start,
                        x_end,
                        y_end,
                        mcast: true,
                        ordering: ttkmd_if::tlb::Ordering::STRICT,
                        ..Default::default()
                    },
                    to_write,
                ) {
                    break allocation;
                }
            };

            let programmed_tlb =
                ttkmd_if::tlb::get_tlb(&self.device, tlb.tlb_index as u32).unwrap();
            let specific_tlb_info =
                ttkmd_if::tlb::get_per_tlb_info(&self.device, tlb.tlb_index as u32);
            debug_assert_eq!(programmed_tlb.mcast, true);
            debug_assert_eq!(programmed_tlb.x_end, x_end);
            debug_assert_eq!(programmed_tlb.y_end, y_end);
            debug_assert_eq!(programmed_tlb.x_start, x_start);
            debug_assert_eq!(programmed_tlb.y_start, y_start);
            debug_assert_eq!(
                programmed_tlb.local_offset * specific_tlb_info.size
                    + (tlb.address - specific_tlb_info.data_base),
                addr
            );
            debug_assert_eq!(specific_tlb_info.memory_type, MemoryType::Uc);

            let to_write = std::cmp::min(tlb.size, to_write);
            self.device.write_block_no_dma(
                tlb.address as u32,
                &data[written as usize..(written as usize + to_write as usize)],
            )?;

            written += to_write;
        }

        Ok(())
    }

    pub fn noc_broadcast32(&self, noc_id: u8, addr: u64, value: u32) -> Result<(), PciError> {
        let (x_start, y_start, x_end, y_end) = self.broadcast_grid();

        let tlb = loop {
            if let Some(allocation) = self.get_tlb(
                Tlb {
                    local_offset: addr,
                    noc_sel: noc_id,
                    x_start,
                    y_start,
                    x_end,
                    y_end,
                    ordering: ttkmd_if::tlb::Ordering::STRICT,
                    mcast: true,
                    ..Default::default()
                },
                4,
            ) {
                break allocation;
            }
        };

        let programmed_tlb = ttkmd_if::tlb::get_tlb(&self.device, tlb.tlb_index as u32).unwrap();
        let specific_tlb_info = ttkmd_if::tlb::get_per_tlb_info(&self.device, tlb.tlb_index as u32);
        debug_assert_eq!(programmed_tlb.mcast, true);
        debug_assert_eq!(programmed_tlb.x_end, x_end);
        debug_assert_eq!(programmed_tlb.y_end, y_end);
        debug_assert_eq!(programmed_tlb.x_start, x_start);
        debug_assert_eq!(programmed_tlb.y_start, y_start);
        debug_assert_eq!(
            programmed_tlb.local_offset * specific_tlb_info.size
                + (tlb.address - specific_tlb_info.data_base),
            addr
        );
        debug_assert_eq!(specific_tlb_info.memory_type, MemoryType::Uc);

        self.device.write32(tlb.address as u32, value)
    }

    pub fn go_idle(&self) {
        match self.device.arch {
            Arch::Grayskull => {
                grayskull::arc_msg(
                    self,
                    &grayskull::ArcMsg::SetPowerState(grayskull::PowerState::LongIdle),
                    true,
                    std::time::Duration::from_secs(1),
                    5,
                    3,
                    &grayskull::ArcMsgAddr {
                        scratch_base: 0x1ff30060,
                        arc_misc_cntl: 0x1ff30100,
                    },
                )
                .unwrap();
            }
            Arch::Wormhole => {
                wormhole::arc_msg(
                    self,
                    &wormhole::ArcMsg::SetPowerState(wormhole::PowerState::LongIdle),
                    true,
                    std::time::Duration::from_secs(1),
                    5,
                    3,
                    &wormhole::ArcMsgAddr {
                        scratch_base: 0x1ff30060,
                        arc_misc_cntl: 0x1ff30100,
                    },
                )
                .unwrap();
            }
            Arch::Blackhole => {}
        }
    }

    pub fn go_busy(&self) {
        match self.device.arch {
            Arch::Grayskull => {
                grayskull::arc_msg(
                    self,
                    &grayskull::ArcMsg::SetPowerState(grayskull::PowerState::Busy),
                    true,
                    std::time::Duration::from_secs(1),
                    5,
                    3,
                    &grayskull::ArcMsgAddr {
                        scratch_base: 0x1ff30060,
                        arc_misc_cntl: 0x1ff30100,
                    },
                )
                .unwrap();
            }
            Arch::Wormhole => {
                wormhole::arc_msg(
                    self,
                    &wormhole::ArcMsg::SetPowerState(wormhole::PowerState::Busy),
                    true,
                    std::time::Duration::from_secs(1),
                    5,
                    3,
                    &wormhole::ArcMsgAddr {
                        scratch_base: 0x1ff30060,
                        arc_misc_cntl: 0x1ff30100,
                    },
                )
                .unwrap();
            }
            Arch::Blackhole => {}
        }
    }

    pub fn deassert_riscv_reset(&self) {
        match self.device.arch {
            Arch::Grayskull => {
                grayskull::arc_msg(
                    self,
                    &grayskull::ArcMsg::DeassertRiscVReset,
                    true,
                    std::time::Duration::from_secs(1),
                    5,
                    3,
                    &grayskull::ArcMsgAddr {
                        scratch_base: 0x1ff30060,
                        arc_misc_cntl: 0x1ff30100,
                    },
                )
                .unwrap();
            }
            Arch::Wormhole => {
                wormhole::arc_msg(
                    self,
                    &wormhole::ArcMsg::DeassertRiscVReset,
                    true,
                    std::time::Duration::from_secs(1),
                    5,
                    3,
                    &wormhole::ArcMsgAddr {
                        scratch_base: 0x1ff30060,
                        arc_misc_cntl: 0x1ff30100,
                    },
                )
                .unwrap();
            }
            Arch::Blackhole => {}
        }
    }

    pub fn load(
        &self,
        name: &str,
        core: (u32, u32),
        options: crate::LoadOptions,
    ) -> crate::loader::Kernel {
        crate::quick_load(name, &self, core.0 as u8, core.1 as u8, options)
    }

    pub fn alloc_dma(&mut self, size: u32) -> DmaBuffer {
        self.device
            .allocate_dma_buffer(size)
            .map_err(|v| v.to_string())
            .unwrap()
    }
}
