use std::collections::HashSet;

const DRAM_LOCATIONS: &[(u32, u32)] = &[
    (0, 11),
    (5, 11),
    (5, 2),
    (5, 8),
    (5, 5),
    (0, 5),
    (0, 1),
    (0, 0),
    (5, 1),
    (5, 0),
    (5, 9),
    (5, 10),
    (5, 4),
    (5, 3),
    (5, 6),
    (5, 7),
    (0, 6),
    (0, 7),
];
const ETH_LOCATIONS: &[(u32, u32)] = &[
    (9, 0),
    (1, 0),
    (8, 0),
    (2, 0),
    (7, 0),
    (3, 0),
    (6, 0),
    (4, 0),
    (9, 6),
    (1, 6),
    (8, 6),
    (2, 6),
    (7, 6),
    (3, 6),
    (6, 6),
    (4, 6),
];
const ARC_LOCATIONS: &[(u32, u32)] = &[(0, 10)];
const PCI_LOCATIONS: &[(u32, u32)] = &[(0, 3), (0, 2), (0, 4), (0, 8), (0, 9)];

const GRID_SIZE_X: u32 = 10;
const GRID_SIZE_Y: u32 = 12;
const NUM_TENSIX_X: u32 = GRID_SIZE_X - 2;
const NUM_TENSIX_Y: u32 = GRID_SIZE_Y - 2;

const PHYS_X_TO_NOC_0_X: &[u32] = &[0, 9, 1, 8, 2, 7, 3, 6, 4, 5];
const PHYS_Y_TO_NOC_0_Y: &[u32] = &[0, 11, 1, 10, 2, 9, 3, 8, 4, 7, 5, 6];
const PHYS_X_TO_NOC_1_X: &[u32] = &[9, 0, 8, 1, 7, 2, 6, 3, 5, 4];
const PHYS_Y_TO_NOC_1_Y: &[u32] = &[11, 0, 10, 1, 9, 2, 8, 3, 7, 4, 6, 5];

const ALL_TENSIX_ROWS: &[u32] = &[1, 2, 3, 4, 5, 7, 8, 9, 10, 11];
const ALL_TENSIX_COLS: &[u32] = &[1, 2, 3, 4, 6, 7, 8, 9];

pub fn get_grid(harvest: u32) -> super::NocGrid {
    let mut bad_rows = harvest << 1;
    let mut disabled_rows = HashSet::new();
    for y in 0..32 {
        if bad_rows & 1 == 1 {
            disabled_rows.insert(PHYS_Y_TO_NOC_0_Y[y as usize]);
        }
        bad_rows >>= 1;
    }

    let good_rows = HashSet::from_iter(ALL_TENSIX_ROWS.iter().copied())
        .difference(&disabled_rows)
        .cloned()
        .collect::<Vec<_>>();

    let mut good_cores = Vec::new();
    for y in good_rows {
        for x in ALL_TENSIX_COLS.iter().copied() {
            good_cores.push((x, y));
        }
    }
    good_cores.sort();

    super::NocGrid {
        tensix: good_cores,
        dram: Vec::from_iter(DRAM_LOCATIONS.into_iter().cloned()),
        pci: Vec::from_iter(PCI_LOCATIONS.into_iter().cloned()),
        arc: Vec::from_iter(ARC_LOCATIONS.into_iter().cloned()),
        eth: Vec::from_iter(ETH_LOCATIONS.into_iter().cloned()),
    }
}

pub fn get_tensix_l1_size() -> u32 {
    1536 * 1024
}
