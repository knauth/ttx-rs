use std::path::PathBuf;

use tensix_builder::CargoOptions;

pub mod chip;
pub mod loader;

#[cfg(test)]
mod chip_test;

pub use chip::{open, Chip};
pub use luwen::luwen_core::Arch;

pub use macros::kernel;
pub use tensix_builder;

pub fn enumerate() -> Vec<usize> {
    luwen::luwen_ref::PciDevice::scan()
}

pub struct LoadOptions {
    no_wait: bool,
    build_std: bool,
    verbose: bool,
    lto: bool,
    base_path: PathBuf,
    path: String,
    profile: String,
    default_features: bool,
    stack_probes: bool,
}

impl LoadOptions {
    pub fn new(base_path: &std::path::Path) -> Self {
        Self {
            no_wait: false,
            build_std: false,
            verbose: false,
            lto: false,
            base_path: base_path.to_path_buf(),
            path: String::new(),
            profile: "release".to_string(),
            default_features: true,
            stack_probes: false,
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

pub fn read32(device: &Chip, x: u8, y: u8, addr: u64) -> u32 {
    device.noc_read32(0, x, y, addr).unwrap()
}

pub fn write32(device: &Chip, x: u8, y: u8, addr: u64, value: u32) {
    device.noc_write32(0, x, y, addr, value).unwrap()
}

pub fn quick_load<'a>(
    name: &str,
    device: &'a Chip,
    x: u8,
    y: u8,
    options: LoadOptions,
) -> crate::loader::Kernel<'a> {
    device.start();

    let arch = match device.arch() {
        luwen::luwen_core::Arch::Grayskull => tensix_builder::TensixTarget::Grayskull,
        luwen::luwen_core::Arch::Wormhole => tensix_builder::TensixTarget::Wormhole,
        luwen::luwen_core::Arch::Blackhole => tensix_builder::TensixTarget::Blackhole,
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
        CargoOptions {
            target: arch.clone(),
            profile,
            lto: options.lto,
            verbose: options.verbose,
            build_std: options.build_std,
            default_features: options.default_features,
            stack_probes: options.stack_probes,
            kernel_name: name.to_string(),
        },
    );

    println!("Loading binary to {arch}");

    loader::stop(device, x, y);

    let mut kernel = loader::load_file(device, x, y, kernel);

    loader::easy_start(device, x, y);

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

#[macro_export]
macro_rules! load {
    ($name:expr, $device:ident, $core:expr) => {
        $crate::load!($name, $device, $core,)
    };
    (
        $name:expr, $device:ident, $core:expr, $( $key:ident = $value:expr ),* $(,)?
    ) => {{
        let base_path = ::std::path::PathBuf::from(file!())
            .parent()
            .unwrap()
            .to_path_buf();
        let options = $crate::LoadOptions::new(base_path.as_path())$(.$key($value))*;

        $crate::quick_load(
            stringify!($name),
            &$device,
            $core.0 as u8,
            $core.1 as u8,
            options,
        )
    }};
}
