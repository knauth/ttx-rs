use std::path::PathBuf;

use tempfile::TempDir;

use super::{Firmware, FirmwareParameters};

impl Firmware {
    pub fn write_cargo_toml(dir: &TempDir) {
        let path = env!("CARGO_MANIFEST_DIR").parse::<PathBuf>().unwrap();
        let shared_path = path.parent().unwrap().join("shared");

        std::fs::write(
            dir.path().join("Cargo.toml"),
            format!(
                r#"
        [package]
        name = "firmware"
        version = "0.1.0"
        edition = "2024"

        [dependencies]
        tensix-std = {{path = "/home/drosen/work/tensix-std"}}
        shared = {{path = "{}"}}
        "#,
                shared_path.display()
            ),
        )
        .unwrap();
    }

    pub fn write_main(src_file: &PathBuf, parameters: FirmwareParameters) {
        std::fs::write(
            src_file,
            format!(
                r#"
        #![no_std]
        #![no_main]

        struct JobInfo {{
            entry: u32,
            stack: Option<u32>,
        }}

        static mut JOB_INFO: shared::LaunchData = shared::LaunchData::cdefault();

        #[repr(align(64))]
        pub struct NocAligned<T>(T);

        impl<T> core::ops::Deref for NocAligned<T> {{
            type Target = T;

            fn deref(&self) -> &<Self as core::ops::Deref>::Target {{
                &self.0
            }}
        }}

        impl<T> core::ops::DerefMut for NocAligned<T> {{
            fn deref_mut(&mut self) -> &mut <Self as core::ops::Deref>::Target {{
                &mut self.0
            }}
        }}

        #[repr(transparent)]
        pub struct SyncUnsafeCell<T>(core::cell::UnsafeCell<T>);
        unsafe impl<T: Sync> Sync for SyncUnsafeCell<T> {{}}

        impl<T> SyncUnsafeCell<T> {{
            pub const fn new(value: T) -> Self {{
                SyncUnsafeCell(core::cell::UnsafeCell::new(value))
            }}

            pub fn get(&self) -> *mut T {{
                self.0.get()
            }}
        }}

        #[unsafe(no_mangle)]
        static mut JOB_LAUNCHED: NocAligned<u32> = NocAligned(0);

        #[unsafe(no_mangle)]
        static CORE_ID: SyncUnsafeCell<NocAligned<i32>> = SyncUnsafeCell::new(NocAligned(-1));

        static NCRISC_JOB_POINTER: SyncUnsafeCell<Option<shared::CoreLaunchData>> = SyncUnsafeCell::new(None);
        static NCRISC_JOB_RESULT: SyncUnsafeCell<Option<()>> = SyncUnsafeCell::new(None);
        static TRISC0_JOB_POINTER: SyncUnsafeCell<Option<shared::CoreLaunchData>> = SyncUnsafeCell::new(None);
        static TRISC0_JOB_RESULT: SyncUnsafeCell<Option<()>> = SyncUnsafeCell::new(None);
        static TRISC1_JOB_POINTER: SyncUnsafeCell<Option<shared::CoreLaunchData>> = SyncUnsafeCell::new(None);
        static TRISC1_JOB_RESULT: SyncUnsafeCell<Option<()>> = SyncUnsafeCell::new(None);
        static TRISC2_JOB_POINTER: SyncUnsafeCell<Option<shared::CoreLaunchData>> = SyncUnsafeCell::new(None);
        static TRISC2_JOB_RESULT: SyncUnsafeCell<Option<()>> = SyncUnsafeCell::new(None);

        use tensix_std::entry;

        fn dyn_base() -> u32 {{
            unsafe extern "Rust" {{
                unsafe static mut __firmware_end: u8;
            }}

            core::ptr::addr_of!(__firmware_end) as u32
        }}

        fn request_job() -> shared::LaunchData {{
            unsafe {{
                JOB_LAUNCHED.0 = 23;
                tensix_std::target::noc_map::write32(
                    0, {job_server_x}, {job_server_y},
                    0x{job_server_addr:x} + (16 * (*CORE_ID.get()).0) as u64,
                    (&raw const JOB_INFO) as u32
                );
                JOB_LAUNCHED.0 = 3;

                while (&raw mut JOB_INFO).read_volatile().flag == 0 {{}}

                JOB_LAUNCHED.0 = 4;

                let output = (&raw mut JOB_INFO).read_volatile();
                JOB_INFO = shared::LaunchData::default();

                JOB_LAUNCHED.0 = 5;

                output
            }}
        }}

        fn load_job(job: &shared::LaunchData) {{
            unsafe {{
                tensix_std::target::noc_map::read(
                    1, job.workload_bank.0, job.workload_bank.1, job.workload_bank_offset,
                    core::slice::from_raw_parts_mut(dyn_base() as *mut u8, job.workload_bank_size as usize)
                );
            }}
        }}

        fn jump_to_stack(addr: u32, new_sp: u32) {{
            unsafe {{
                let mut old_sp: usize;

                core::arch::asm!(
                    "mv {{0}}, sp",
                    out(reg) old_sp,
                );

                let mut old_sp_ref = &mut old_sp;

                core::arch::asm!(
                    "mv sp, {{0}}",
                    in(reg) new_sp,
                );

                core::mem::transmute::<u32, fn()>(addr)();

                let old_sp = *old_sp_ref;

                core::arch::asm!(
                    "mv {{0}}, sp",
                    in(reg) old_sp,
                );
            }}
        }}

        fn jump_to(addr: u32, new_sp: Option<u32>) {{
            if let Some(new_sp) = new_sp {{
                jump_to_stack(addr, new_sp)
            }} else {{
                unsafe {{
                    core::mem::transmute::<u32, fn()>(addr)()
                }}
            }}
        }}

        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn memcpy(dest: *mut u8, src: *const u8, n: usize) -> *mut u8 {{
            unsafe {{
                let mut i = 0;
                while i < n {{
                    dest.add(i).write(src.add(i).read());
                    i += 1;
                }}
                dest
            }}
        }}

        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn memset(dest: *mut u8, src: core::ffi::c_int, n: usize) -> *mut u8 {{
            unsafe {{
                let mut i = 0;
                while i < n {{
                    dest.add(i).write(src as u8);
                    i += 1;
                }}
                dest
            }}
        }}

        #[entry(brisc)]
        unsafe fn brisc_main() -> ! {{
            tensix_std::reset::start_cores();

            unsafe {{
                loop {{
                    JOB_LAUNCHED.0 = 0;

                    while (*CORE_ID.get()).0 == -1 {{}}

                    JOB_LAUNCHED.0 = 1;

                    let job = request_job();

                    JOB_LAUNCHED.0 = 100;

                    load_job(&job);

                    JOB_LAUNCHED.0 = 101;

                    (*NCRISC_JOB_RESULT.get()) = None;
                    (*TRISC0_JOB_RESULT.get()) = None;
                    (*TRISC1_JOB_RESULT.get()) = None;
                    (*TRISC2_JOB_RESULT.get()) = None;

                    // Zero BSS
                    for addr in job.bss..job.ebss {{
                        *(addr as *mut u32) = 0;
                    }}

                    JOB_LAUNCHED.0 = 200;

                    (*NCRISC_JOB_POINTER.get()) = Some(job.ncrisc);
                    (*TRISC0_JOB_POINTER.get()) = Some(job.trisc0);
                    (*TRISC1_JOB_POINTER.get()) = Some(job.trisc1);
                    (*TRISC2_JOB_POINTER.get()) = Some(job.trisc2);

                    jump_to(dyn_base() + job.brisc.entry, job.brisc.stack);

                    JOB_LAUNCHED.0 = 201;

                    loop {{
                        if (*NCRISC_JOB_RESULT.get()).is_none() {{
                            continue;
                        }}

                        if (*TRISC0_JOB_RESULT.get()).is_none() {{
                            continue;
                        }}

                        if (*TRISC1_JOB_RESULT.get()).is_none() {{
                            continue;
                        }}

                        if (*TRISC2_JOB_RESULT.get()).is_none() {{
                            continue;
                        }}
                    }}

                    *JOB_LAUNCHED = 202;
                }}
            }}
        }}

        #[entry(ncrisc)]
        unsafe fn ncrisc_main() -> ! {{
            unsafe {{
                loop {{
                    let info = NCRISC_JOB_POINTER.get();
                    if let Some(info) = (*info).take() {{
                        jump_to(dyn_base() + info.entry, info.stack);
                        (*NCRISC_JOB_RESULT.get()) = Some(());
                    }}
                }}
            }}
        }}

        #[entry(trisc0)]
        unsafe fn trisc0_main() -> ! {{
            unsafe {{
                loop {{
                    let info = TRISC0_JOB_POINTER.get();
                    if let Some(info) = (*info).take() {{
                        jump_to(dyn_base() + info.entry, info.stack);
                        (*TRISC0_JOB_RESULT.get()) = Some(());
                    }}
                }}
            }}
        }}

        #[entry(trisc1)]
        unsafe fn trisc1_main() -> ! {{
            unsafe {{
                loop {{
                    let info = TRISC1_JOB_POINTER.get();
                    if let Some(info) = (*info).take() {{
                        jump_to(dyn_base() + info.entry, info.stack);
                        (*TRISC1_JOB_RESULT.get()) = Some(());
                    }}
                }}
            }}
        }}

        #[entry(trisc2)]
        unsafe fn trisc2_main() -> ! {{
            unsafe {{
                loop {{
                    let info = TRISC2_JOB_POINTER.get();
                    if let Some(info) = (*info).take() {{
                        jump_to(dyn_base() + info.entry, info.stack);
                        (*TRISC2_JOB_RESULT.get()) = Some(());
                    }}
                }}
            }}
        }}
        "#,
            job_server_x = parameters.job_server.n0.0,
            job_server_y = parameters.job_server.n0.1,
            job_server_addr = parameters.job_server_addr
            ),
        )
        .unwrap()
    }
}
