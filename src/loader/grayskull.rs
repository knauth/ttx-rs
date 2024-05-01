use std::collections::HashSet;

const DRAM_LOCATIONS: &[(u32, u32)] = &[
    (1, 6),
    (4, 6),
    (7, 6),
    (10, 6),
    (1, 0),
    (4, 0),
    (7, 0),
    (10, 0),
];
const ARC_LOCATIONS: &[(u32, u32)] = &[(0, 2)];
const PCI_LOCATIONS: &[(u32, u32)] = &[(0, 4)];
const GRID_SIZE_X: u32 = 13;
const GRID_SIZE_Y: u32 = 12;
const NUM_TENSIX_X: u32 = GRID_SIZE_X - 1;
const NUM_TENSIX_Y: u32 = GRID_SIZE_Y - 2;

const PHYS_X_TO_NOC_0_X: &[u32] = &[0, 12, 1, 11, 2, 10, 3, 9, 4, 8, 5, 7, 6];
const PHYS_Y_TO_NOC_0_Y: &[u32] = &[0, 11, 1, 10, 2, 9, 3, 8, 4, 7, 5, 6];
const PHYS_X_TO_NOC_1_X: &[u32] = &[12, 0, 11, 1, 10, 2, 9, 3, 8, 4, 7, 5, 6];
const PHYS_Y_TO_NOC_1_Y: &[u32] = &[11, 0, 10, 1, 9, 2, 8, 3, 7, 4, 6, 5];

pub fn get_grid(harvest: u32) -> super::NocGrid {
    let mut bad_rows = harvest << 1;
    let mut disabled_rows = HashSet::new();
    for y in 0..32 {
        if bad_rows & 1 == 1 {
            disabled_rows.insert(PHYS_Y_TO_NOC_0_Y[(GRID_SIZE_Y - y - 1) as usize]);
        }
        bad_rows >>= 1;
    }

    let good_rows = HashSet::from_iter([1u32, 2, 3, 4, 5, 7, 8, 9, 10, 11])
        .difference(&disabled_rows)
        .cloned()
        .collect::<Vec<_>>();

    let mut good_cores = Vec::new();
    for y in good_rows {
        for x in 1..GRID_SIZE_X {
            good_cores.push((x, y));
        }
    }
    good_cores.sort();

    super::NocGrid {
        tensix: good_cores,
        dram: Vec::from_iter(DRAM_LOCATIONS.into_iter().cloned()),
        pci: Vec::from_iter(PCI_LOCATIONS.into_iter().cloned()),
        arc: Vec::from_iter(ARC_LOCATIONS.into_iter().cloned()),
        eth: Vec::new(),
    }
}

pub fn get_tensix_l1_size() -> u32 {
    1024 * 1024
}
