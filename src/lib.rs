pub use chip::{open, Chip};
pub use luwen::luwen_core::Arch;

pub use macros::kernel;
pub use tensix_builder;

pub mod chip;
pub mod loader;
pub mod runtime;

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

        $crate::loader::quick_load(
            stringify!($name),
            &$device,
            $core.0 as u8,
            $core.1 as u8,
            options,
        )
    }};
}
