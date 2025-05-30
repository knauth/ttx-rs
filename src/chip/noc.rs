use luwen::{
    luwen_core::Arch,
    ttkmd_if::{tlb::Ordering, PciDevice, PciError, PossibleTlbAllocation, Tlb},
};

#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(u8)]
pub enum NocId {
    Noc0 = 0,
    Noc1 = 1,
}

#[derive(Debug, Default, Clone, Copy, Hash, PartialEq, Eq)]
pub struct NocAddress {
    pub n0: (u8, u8),
    pub n1: (u8, u8),
}

impl Into<u32> for NocAddress {
    fn into(self) -> u32 {
        self.n0.0 as u32
            | ((self.n0.1 as u32) << 8)
            | ((self.n1.0 as u32) << 16)
            | ((self.n1.1 as u32) << 24)
    }
}

impl From<u32> for NocAddress {
    fn from(value: u32) -> Self {
        NocAddress {
            n0: (value as u8, (value >> 8) as u8),
            n1: ((value >> 8) as u8, (value >> 24) as u8),
        }
    }
}

impl NocAddress {
    pub fn get(&self, noc_id: NocId) -> (u8, u8) {
        match noc_id {
            NocId::Noc0 => self.n0,
            NocId::Noc1 => self.n1,
        }
    }
}

#[derive(Debug, Default, Clone, Copy, Hash, PartialEq, Eq)]
pub struct Tile {
    pub addr: NocAddress,
    pub align_read: u8,
    pub align_write: u8,
}

impl Into<NocAddress> for Tile {
    fn into(self) -> NocAddress {
        self.addr
    }
}

impl Into<u32> for Tile {
    fn into(self) -> u32 {
        self.addr.into()
    }
}

impl Tile {
    pub fn get(&self, noc_id: NocId) -> (u8, u8) {
        self.addr.get(noc_id)
    }

    pub fn align_rw_ptr(&self, addr: u64) -> u64 {
        self.align_read_ptr(addr).max(self.align_write_ptr(addr))
    }

    pub fn align_write_ptr(&self, addr: u64) -> u64 {
        let align = self.align_write as u64;
        (addr + (align - 1)) & !(align - 1)
    }

    pub fn align_read_ptr(&self, addr: u64) -> u64 {
        let align = self.align_read as u64;
        (addr + (align - 1)) & !(align - 1)
    }
}

pub fn allocate_tlb(
    device: &mut PciDevice,
    mut size: u64,
) -> Result<PossibleTlbAllocation, PciError> {
    if device.driver_version > 1 {
        while size > 0 {
            if let Ok(tlb) = device.allocate_tlb(size) {
                return Ok(PossibleTlbAllocation::Allocation(tlb));
            }
            size >>= 1;
        }

        tracing::warn!("Failed to allocate a tlb, falling back to fixed default");
    }

    Ok(PossibleTlbAllocation::Hardcoded(match device.arch {
        Arch::Grayskull => 184,
        Arch::Wormhole => 184,
        Arch::Blackhole => 190,
        Arch::Unknown(value) => {
            unimplemented!("Have not implemented support for arch id {value:x}");
        }
    }))
}

pub fn noc_write(
    device: &mut PciDevice,
    tlb: &PossibleTlbAllocation,
    ordering: Ordering,
    noc_id: NocId,
    x: u8,
    y: u8,
    addr: u64,
    data: &[u8],
) -> Result<(), PciError> {
    device.noc_write(
        tlb,
        Tlb {
            local_offset: addr,
            noc_sel: noc_id as u8,
            x_end: x,
            y_end: y,
            // TODO(drosen): BH should use posted strirct for register access
            // TODO(drosen): All others should use relaxed
            ordering,
            ..Default::default()
        },
        data,
    )
}

pub fn noc_read(
    device: &mut PciDevice,
    tlb: &PossibleTlbAllocation,
    ordering: Ordering,
    noc_id: NocId,
    x: u8,
    y: u8,
    addr: u64,
    data: &mut [u8],
) -> Result<(), PciError> {
    device.noc_read(
        tlb,
        Tlb {
            local_offset: addr,
            noc_sel: noc_id as u8,
            x_end: x,
            y_end: y,
            // TODO(drosen): BH should use posted strirct for register access
            // TODO(drosen): All others should use relaxed
            ordering,
            ..Default::default()
        },
        data,
    )
}

pub fn noc_write32(
    device: &mut PciDevice,
    tlb: &PossibleTlbAllocation,
    ordering: Ordering,
    noc_id: NocId,
    x: u8,
    y: u8,
    addr: u64,
    data: u32,
) -> Result<(), PciError> {
    device.noc_write32(
        tlb,
        Tlb {
            local_offset: addr,
            noc_sel: noc_id as u8,
            x_end: x,
            y_end: y,
            // TODO(drosen): BH should use posted strirct for register access
            // TODO(drosen): All others should use relaxed
            ordering,
            ..Default::default()
        },
        data,
    )
}

pub fn noc_read32(
    device: &mut PciDevice,
    tlb: &PossibleTlbAllocation,
    ordering: Ordering,
    noc_id: NocId,
    x: u8,
    y: u8,
    addr: u64,
) -> Result<u32, PciError> {
    device.noc_read32(
        tlb,
        Tlb {
            local_offset: addr,
            noc_sel: noc_id as u8,
            x_end: x,
            y_end: y,
            // TODO(drosen): BH should use posted strirct for register access
            // TODO(drosen): All others should use relaxed
            ordering,
            ..Default::default()
        },
    )
}

pub fn noc_multicast(
    device: &mut PciDevice,
    tlb: &PossibleTlbAllocation,
    ordering: Ordering,
    noc_id: NocId,
    start: (u8, u8),
    end: (u8, u8),
    addr: u64,
    data: &[u8],
) -> Result<(), PciError> {
    device.noc_write(
        tlb,
        Tlb {
            local_offset: addr,
            noc_sel: noc_id as u8,
            x_start: start.0,
            y_start: start.1,
            x_end: end.0,
            y_end: end.1,
            mcast: true,
            // TODO(drosen): BH should use posted strirct for register access
            // TODO(drosen): All others should use relaxed
            ordering,
            ..Default::default()
        },
        data,
    )
}

pub fn noc_multicast32(
    device: &mut PciDevice,
    tlb: &PossibleTlbAllocation,
    ordering: Ordering,
    noc_id: NocId,
    start: (u8, u8),
    end: (u8, u8),
    addr: u64,
    value: u32,
) -> Result<(), PciError> {
    device.noc_write32(
        tlb,
        Tlb {
            local_offset: addr,
            noc_sel: noc_id as u8,
            x_start: start.0,
            y_start: start.1,
            x_end: end.0,
            y_end: end.1,
            mcast: true,
            // TODO(drosen): BH should use posted strirct for register access
            // TODO(drosen): All others should use relaxed
            ordering,
            ..Default::default()
        },
        value,
    )
}

pub trait NocInterface {
    fn noc_read<T: Into<NocAddress>>(&mut self, noc_id: NocId, tile: T, addr: u64, data: &mut [u8]);
    fn noc_read32<T: Into<NocAddress>>(&mut self, noc_id: NocId, tile: T, addr: u64) -> u32;
    fn noc_write<T: Into<NocAddress>>(&mut self, noc_id: NocId, tile: T, addr: u64, data: &[u8]);
    fn noc_write32<T: Into<NocAddress>>(&mut self, noc_id: NocId, tile: T, addr: u64, value: u32);

    fn noc_broadcast(&mut self, noc_id: NocId, addr: u64, data: &[u8]);
    fn noc_broadcast32(&mut self, noc_id: NocId, addr: u64, value: u32);
}
