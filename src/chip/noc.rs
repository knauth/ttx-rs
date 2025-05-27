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
pub struct Tile {
    pub n0: (u8, u8),
    pub n1: (u8, u8),
}

impl Tile {
    pub fn to_u32(&self) -> u32 {
        self.n0.0 as u32
            | ((self.n0.1 as u32) << 8)
            | ((self.n1.0 as u32) << 16)
            | ((self.n1.1 as u32) << 24)
    }

    pub fn get(&self, noc_id: NocId) -> (u8, u8) {
        match noc_id {
            NocId::Noc0 => self.n0,
            NocId::Noc1 => self.n1,
        }
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
    fn noc_read(&mut self, noc_id: NocId, tile: Tile, addr: u64, data: &mut [u8]);
    fn noc_read32(&mut self, noc_id: NocId, tile: Tile, addr: u64) -> u32;
    fn noc_write(&mut self, noc_id: NocId, tile: Tile, addr: u64, data: &[u8]);
    fn noc_write32(&mut self, noc_id: NocId, tile: Tile, addr: u64, value: u32);

    fn noc_broadcast(&mut self, noc_id: NocId, addr: u64, data: &[u8]);
    fn noc_broadcast32(&mut self, noc_id: NocId, addr: u64, value: u32);
}
