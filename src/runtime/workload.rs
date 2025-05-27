use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{LazyLock, Mutex},
};

use luwen::luwen_core::Arch;
use tempfile::TempDir;
use tensix_builder::{CacheEnable, Rewrite};
use tracing::warn;

use crate::loader::{self, KernelData, LoadOptions};

#[derive(Default, Hash, Clone, PartialEq, Eq)]
pub struct OutputBuffer {
    pub name: String,
    pub size: usize,
    pub count: usize,
}

impl OutputBuffer {
    pub fn output_completion(&self, index: usize) -> String {
        assert!(index < self.count);
        format!("_COMPLETION_SLOT_{}_{}", self.name, index)
    }

    pub fn output_count(&self) -> String {
        format!("_COMPLETION_COUNT_{}", self.name)
    }

    pub fn output_buffer(&self) -> String {
        self.name.clone()
    }
}

#[derive(Default, Clone, Hash, PartialEq, Eq)]
pub struct WorkloadBuilder {
    pub available_space: u64,

    pub inputs: Vec<String>,
    pub outputs: Vec<OutputBuffer>,

    pub global: String,
    pub brisc: String,
    pub ncrisc: String,
    pub trisc0: String,
    pub trisc1: String,
    pub trisc2: String,
}

pub struct WorkloadCache(LazyLock<Mutex<HashMap<Arch, HashMap<WorkloadBuilder, KernelData>>>>);

static WORKLOAD_CACHE: WorkloadCache = WorkloadCache(LazyLock::new(|| Mutex::new(HashMap::new())));

impl WorkloadCache {
    fn get_cached_build<'a>(&'a self, arch: Arch, key: &WorkloadBuilder) -> Option<&'a KernelData> {
        if let Ok(value) = self.0.lock() {
            unsafe { std::mem::transmute(value.get(&arch).map(|v| v.get(key)).flatten()) }
        } else {
            warn!("WORKLOAD cache is poisoned!");
            None
        }
    }

    fn cache_build(&self, arch: Arch, key: WorkloadBuilder, payload: KernelData) {
        if let Ok(mut value) = self.0.lock() {
            value.entry(arch).or_default().insert(key, payload);
        } else {
            warn!("WORKLOAD cache is poisoned!");
        }
    }
}

impl WorkloadBuilder {
    pub fn input_buffer(&mut self, name: impl AsRef<str>, size: usize) {
        let name = name.as_ref();

        self.global.push_str(&format!(
            r#"
            #[unsafe(no_mangle)]
            static {name}: NocAlignment<u8, {size}> = NocAlignment::new(0);
          "#
        ));
        self.inputs.push(name.to_string());
    }

    pub fn output_buffer(
        &mut self,
        name: impl AsRef<str>,
        size: usize,
        // The number of nodes waiting on the completion
        completion_slots: usize,
    ) -> OutputBuffer {
        let name = name.as_ref();

        for i in 0..completion_slots {
            let name = format!("_COMPLETION_SLOT_{name}_{i}");
            self.global.push_str(&format!(
                r#"
                #[unsafe(no_mangle)]
                static mut {name}: NocAlignment<u32, 1> = NocAlignment::new(0);
            "#
            ));
        }

        let completion_name = format!("_COMPLETION_COUNT_{name}");
        self.global.push_str(&format!(
            r#"
            #[unsafe(no_mangle)]
            static mut {completion_name}: NocAlignment<u8, 4> = NocAlignment::new(0);
          "#
        ));
        self.global.push_str(&format!(
            r#"
            #[unsafe(no_mangle)]
            static mut {name}: NocAlignment<u8, {size}> = NocAlignment::new(0);
          "#
        ));

        OutputBuffer {
            name: name.to_string(),
            size,
            count: completion_slots,
        }
    }

    pub fn iterate_zip(
        &mut self,
        shape: &[i64],
        strides: &[&[usize]],
        func: impl FnOnce(&[String]) -> String,
    ) -> String {
        let mut loops = Vec::new();
        let mut it = shape.iter().copied().enumerate();

        {
            if let Some((index, dim)) = it.next() {
                loops.push(format!("for _{index}_it in 0..{dim}u32 {{"));
                for (tensor_index, stride) in strides.iter().enumerate() {
                    loops.push(format!(
                        "let _{tensor_index}_tensor_{index}_dim_index = _{index}_it * {stride};",
                        stride = stride[index]
                    ));
                }
            }
        }

        for (index, dim) in it {
            loops.push(format!("for _{index}_it in 0..{dim}u32 {{"));
            for (tensor_index, stride) in strides.iter().enumerate() {
                loops.push(format!("let _{tensor_index}_tensor_{index}_dim_index = _{tensor_index}_tensor_{last_index}_dim_index + _{index}_it * {stride};", last_index = index - 1, stride = stride[index]));
            }
        }

        if !loops.is_empty() {
            let mut final_indexes = Vec::new();
            for tensor_index in 0..strides.len() {
                loops.push(format!(
                    "let _{tensor_index}_tensor_index = _{tensor_index}_tensor_{}_dim_index;",
                    shape.len().saturating_sub(1)
                ));
                final_indexes.push(format!("_{tensor_index}_tensor_index"));
            }
            loops.push(func(&final_indexes));
        }

        for _ in 0..shape.len() {
            loops.push("}".to_string());
        }

        loops.join("\n")
    }

    pub fn compile(self, arch: Arch) -> Workload {
        Workload::compile(arch, self)
    }
}

pub struct Workload {
    pub path: Option<TempDir>,
    pub data: KernelData,
}

impl Workload {
    pub fn dupe(&self) -> Self {
        Self {
            path: None,
            data: self.data.clone(),
        }
    }
}

impl Workload {
    fn write_cargo_toml(dir: &TempDir) {
        std::fs::write(
            dir.path().join("Cargo.toml"),
            r#"
        [package]
        name = "kernel"
        version = "0.1.0"
        edition = "2024"

        [lib]
        crate-type = ["cdylib"]

        [dependencies]
        tensix-std = {path = "/home/drosen/work/tensix-std"}
        "#,
        )
        .unwrap();
    }

    fn write_lib(src_file: &PathBuf, builder: WorkloadBuilder) {
        std::fs::write(
            src_file,
            format!(
                r#"
        #![no_std]
        #![no_main]

        #[repr(align(64))]
        struct NocAlignment<T, const N: usize>(pub [T; N]);

        impl<T, const N: usize> core::ops::Index<usize> for NocAlignment<T, N> {{
            type Output = T;

            fn index(&self, index: usize) -> &Self::Output {{
                &self.0[index]
            }}
        }}

        impl<T, const N: usize> core::ops::IndexMut<usize> for NocAlignment<T, N> {{
            fn index_mut(&mut self, index: usize) -> &mut Self::Output {{
                &mut self.0[index]
            }}
        }}

        impl<T, const N: usize> core::ops::Index<u32> for NocAlignment<T, N> {{
            type Output = T;

            fn index(&self, index: u32) -> &Self::Output {{
                &self.0[index as usize]
            }}
        }}

        impl<T, const N: usize> core::ops::IndexMut<u32> for NocAlignment<T, N> {{
            fn index_mut(&mut self, index: u32) -> &mut Self::Output {{
                &mut self.0[index as usize]
            }}
        }}

        impl<T: Copy, const N: usize> NocAlignment<T, N> {{
            pub const fn new(value: T) -> Self {{
                NocAlignment([value; N])
            }}
        }}

        impl<T, const N: usize> NocAlignment<T, N> {{
            pub fn addr(&self) -> u32 {{
                self.0.as_ptr() as u32
            }}

            pub fn len(&self) -> u32 {{
                N as u32
            }}
        }}

        #[panic_handler]
        fn panic(_info: &core::panic::PanicInfo) -> ! {{
            loop {{}}
        }}

        {global}

        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn brisc_kmain() {{
            unsafe {{
                {brisc}
            }}
        }}

        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn ncrisc_kmain() {{
            unsafe {{
                {ncrisc}
            }}
        }}

        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn trisc0_kmain() {{
            unsafe {{
                {trisc0}
            }}
        }}

        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn trisc1_kmain() {{
            unsafe {{
                {trisc1}
            }}
        }}

        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn trisc2_kmain() {{
            unsafe {{
                {trisc2}
            }}
        }}
        "#,
                global = builder.global,
                brisc = builder.brisc,
                ncrisc = builder.ncrisc,
                trisc0 = builder.trisc0,
                trisc1 = builder.trisc1,
                trisc2 = builder.trisc2,
            ),
        )
        .unwrap();
    }

    pub fn compile(arch: Arch, builder: WorkloadBuilder) -> Self {
        if let Some(cached) = WORKLOAD_CACHE.get_cached_build(arch, &builder) {
            return Workload {
                path: None,
                data: cached.clone(),
            };
        }

        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        let src_file = dir.path().join("src").join("lib.rs");

        let available_space_for_workload = builder.available_space;

        Self::write_cargo_toml(&dir);
        Self::write_lib(&src_file, builder.clone());

        let mut hasher = std::hash::RandomState::new();
        let name = std::hash::BuildHasher::hash_one(&mut hasher, std::fs::read(src_file).unwrap());

        let link_script = match arch {
            Arch::Grayskull => include_str!("workload_link/grayskull-kernel.x"),
            Arch::Wormhole => include_str!("workload_link/wormhole-kernel.x"),
            Arch::Blackhole => include_str!("workload_link/blackhole-kernel.x"),
            Arch::Unknown(_) => todo!(),
        };
        let link_script = link_script.replace(
            "{available_space}",
            &available_space_for_workload.to_string(),
        );

        let kernel_data = crate::loader::build_kernel(
            &name.to_string(),
            arch,
            LoadOptions::new(dir.path()).use_cache(CacheEnable::CustomDir(
                super::SCCACHE_DIR.path().to_path_buf(),
            )),
            Some((
                link_script,
                vec![
                    Rewrite::Replace {
                        start: "\"relocation-model\"".to_string(),
                        end: ",".to_string(),
                        replace: "\"relocation-model\": \"pic\"".to_string(),
                    },
                    Rewrite::Add {
                        value: ",\n\"dynamic-linking\": true\n".to_string(),
                    },
                ],
            )),
        );

        let data = if let loader::BinOrLib::Lib(data) = kernel_data {
            data
        } else {
            unreachable!("Should have forced compilation to be a lib");
        };

        WORKLOAD_CACHE.cache_build(arch, builder, data.clone());

        Workload {
            path: Some(dir),
            data: data,
        }
    }

    pub fn get_binary(&self) -> Vec<u8> {
        let mut kernel_binary = Vec::new();
        for write in &self.data.writes {
            // u32
            kernel_binary.extend_from_slice(&write.addr.to_le_bytes());
            if write.addr as usize + write.len() > kernel_binary.len() {
                kernel_binary.resize(write.addr as usize + write.len(), 0);
            }
            kernel_binary[write.addr as usize..][..write.len()].copy_from_slice(&write.data.0);
        }

        kernel_binary
    }
}
