use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex},
};

use luwen::luwen_core::Arch;
use tempfile::TempDir;
use tracing::warn;

use crate::chip::{
    noc::{NocId, Tile},
    Chip,
};
use crate::loader::{self, KernelBinData, KernelData, LoadOptions};

mod gen;

#[derive(Clone, Hash, PartialEq, Eq)]
pub struct FirmwareParameters {
    pub job_server: Tile,
    pub job_server_addr: u64,
}

pub struct FirmwareCache(LazyLock<Mutex<HashMap<Arch, HashMap<FirmwareParameters, Firmware>>>>);

static FIRMWARE_CACHE: FirmwareCache = FirmwareCache(LazyLock::new(|| Mutex::new(HashMap::new())));

impl FirmwareCache {
    fn get_cached_build<'a>(
        &'a self,
        arch: Arch,
        key: &FirmwareParameters,
    ) -> Option<&'a Firmware> {
        if let Ok(value) = self.0.lock() {
            unsafe { std::mem::transmute(value.get(&arch).map(|v| v.get(key)).flatten()) }
        } else {
            warn!("WORKLOAD cache is poisoned!");
            None
        }
    }

    fn cache_build(&self, arch: Arch, key: FirmwareParameters, payload: Firmware) {
        if let Ok(mut value) = self.0.lock() {
            value.entry(arch).or_default().insert(key, payload);
        } else {
            warn!("WORKLOAD cache is poisoned!");
        }
    }
}

pub struct Firmware {
    pub path: Option<TempDir>,
    pub data: KernelData,
    pub bin_data: KernelBinData,
}

impl Firmware {
    pub fn dupe(&self) -> Self {
        Self {
            path: None,
            data: self.data.clone(),
            bin_data: self.bin_data.clone(),
        }
    }
}

impl Firmware {
    pub fn print_state(&mut self, chip: &mut Chip, noc_id: NocId, tile: Tile) {
        self.bin_data.print_state(chip, noc_id, tile, &self.data);
    }

    pub fn print_state_diff(&mut self, chip: &mut Chip, noc_id: NocId, tile: Tile) {
        self.bin_data
            .print_state_diff(chip, noc_id, tile, &self.data);
    }

    pub fn wait(&mut self, chip: &mut Chip, noc_id: NocId, tile: Tile) {
        self.bin_data.wait(chip, noc_id, tile, &self.data);
    }

    pub fn compile(parameters: FirmwareParameters, arch: Arch) -> Self {
        if let Some(cached) = FIRMWARE_CACHE.get_cached_build(arch, &parameters) {
            return cached.dupe();
        }

        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        let src_file = dir.path().join("src").join("main.rs");

        Self::write_cargo_toml(&dir);
        Self::write_main(&src_file, parameters.clone());

        let link_script = match arch {
            Arch::Grayskull => include_str!("workload_link/grayskull.x"),
            Arch::Wormhole => include_str!("workload_link/wormhole.x"),
            Arch::Blackhole => include_str!("workload_link/blackhole.x"),
            Arch::Unknown(_) => todo!(),
        };

        let kernel_data = crate::loader::build_kernel(
            "firmware",
            arch,
            LoadOptions::new(dir.path()).use_cache(tensix_builder::CacheEnable::CustomDir(
                super::SCCACHE_DIR.path().to_path_buf(),
            )),
            Some((link_script.to_string(), vec![])),
        );

        let firmware = if let loader::BinOrLib::Bin { data, bin_data } = kernel_data {
            Firmware {
                path: Some(dir),
                data,
                bin_data,
            }
        } else {
            unreachable!("Should have forced compilation to be a bin");
        };

        FIRMWARE_CACHE.cache_build(arch, parameters, firmware.dupe());

        firmware
    }
}
