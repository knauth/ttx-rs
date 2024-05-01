const PHYS_TO_NOC0_X: &[u32] = &[0, 1, 16, 2, 15, 3, 14, 4, 13, 5, 12, 6, 11, 7, 10, 8, 9];
const PHYS_TO_NOC0_Y: &[u32] = &[0, 1, 11, 2, 10, 3, 9, 4, 8, 5, 7, 6];

const GRID_SIZE_X: u32 = 17;
const GRID_SIZE_Y: u32 = 12;

const NUM_TENSIX_ROWS: u32 = 10;
const NUM_TENSIX_COLS: u32 = 14;

const GDDR_NOC0_COORDS: &[[(u32, u32); 3]] = &[
    [(0, 0), (0, 1), (0, 11)],
    [(0, 2), (0, 10), (0, 3)],
    [(0, 9), (0, 4), (0, 8)],
    [(0, 5), (0, 7), (0, 6)],
    [(9, 11), (9, 1), (9, 0)],
    [(9, 3), (9, 10), (9, 2)],
    [(9, 8), (9, 4), (9, 9)],
    [(9, 6), (9, 7), (9, 5)],
];

pub fn get_grid(_harvest: u32) -> super::NocGrid {
    let mut tensix = Vec::new();
    for y in 0..NUM_TENSIX_ROWS {
        for x in 0..NUM_TENSIX_COLS {
            tensix.push((
                PHYS_TO_NOC0_X[x as usize + 1],
                PHYS_TO_NOC0_Y[y as usize + 2],
            ));
        }
    }
    tensix.sort();

    super::NocGrid {
        tensix,
        dram: Vec::new(),
        pci: vec![(2, 0), (11, 0)],
        arc: Vec::new(),
        eth: Vec::new(),
    }
}

pub fn get_tensix_l1_size() -> u32 {
    1536 * 1024
}
