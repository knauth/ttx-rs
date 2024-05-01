use std::{
    collections::HashMap,
    fmt::Display,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

#[derive(Clone)]
pub enum TensixTarget {
    Grayskull,
    Wormhole,
    Blackhole,
}

impl Display for TensixTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            TensixTarget::Grayskull => "Grayskull",
            TensixTarget::Wormhole => "Wormhole",
            TensixTarget::Blackhole => "Blackhole",
        };
        f.write_str(name)
    }
}

impl TensixTarget {
    pub fn to_target_json(&self) -> String {
        match self {
            TensixTarget::Grayskull => "target_def/grayskull.json",
            TensixTarget::Wormhole => "target_def/wormhole.json",
            TensixTarget::Blackhole => "target_def/blackhole.json",
        }
        .to_string()
    }

    pub fn to_string(&self) -> String {
        match self {
            TensixTarget::Grayskull => "grayskull",
            TensixTarget::Wormhole => "wormhole",
            TensixTarget::Blackhole => "blackhole",
        }
        .to_string()
    }
}

pub enum CargoProfile {
    Release,
    Debug,
    Other(String),
}

impl CargoProfile {
    pub fn to_string(&self) -> String {
        match self {
            CargoProfile::Release => "release".to_string(),
            CargoProfile::Debug => "dev".to_string(),
            CargoProfile::Other(value) => value.to_string(),
        }
    }
}

pub struct CargoOptions {
    pub target: TensixTarget,
    pub profile: CargoProfile,
    pub lto: bool,
    pub verbose: bool,
    pub build_std: bool,
    pub default_features: bool,
    pub stack_probes: bool,
    pub kernel_name: String,
}

// Check if we might be running inside a cargo invocation.
// Will assume that this is true if we can invoke `cargo metadata`
// If we just append /tensix-builder to it to avoid a deadlock
fn get_target_dir() -> Option<PathBuf> {
    if let Ok(metadata) = cargo_metadata::MetadataCommand::new().exec() {
        Some(metadata.target_directory.as_std_path().to_path_buf())
    } else {
        None
    }
}

fn get_compiler_artifact(stdout: impl AsRef<str>) -> Option<PathBuf> {
    for message in cargo_metadata::Message::parse_stream(stdout.as_ref().as_bytes()) {
        let message = message.unwrap();
        match message {
            cargo_metadata::Message::CompilerArtifact(artifact) => {
                if let Some(artifact) = artifact.executable {
                    return Some(artifact.as_std_path().to_path_buf());
                }
            }
            _ => {}
        }
    }

    None
}

fn invoke_cargo(path: PathBuf, options: CargoOptions) -> PathBuf {
    let target_map = HashMap::from([
        (
            "grayskull",
            include_bytes!("../target_def/grayskull.json").as_slice(),
        ),
        (
            "wormhole",
            include_bytes!("../target_def/wormhole.json").as_slice(),
        ),
        (
            "blackhole",
            include_bytes!("../target_def/blackhole.json").as_slice(),
        ),
    ]);

    let dir = tempfile::tempdir().unwrap();
    let target = options.target.to_string();
    let target = target.as_str();
    let file = dir.path().join(format!("{target}.json"));
    std::fs::write(&file, target_map[target]).unwrap();

    let build_std = if options.build_std {
        "-Zbuild-std"
    } else {
        "-Zbuild-std=core,alloc"
    };

    let mut cargo = Command::new("cargo");
    cargo.args([
        "+nightly",
        "build",
        "--message-format=json-render-diagnostics",
        "--target",
        &file.to_string_lossy(),
        "--profile",
        &options.profile.to_string(),
        build_std,
    ]);

    if options.verbose {
        cargo.arg("--verbose");
    }

    if !options.default_features {
        cargo.arg("--no-default-features");
    }

    if let Some(target) = get_target_dir() {
        cargo.args([
            "--target-dir",
            &target
                .join(format!("tensix-builder/{}", options.kernel_name))
                .to_string_lossy(),
        ]);
    }

    let mut kernel_name = options.kernel_name;
    if !kernel_name.starts_with('"') || !kernel_name.ends_with('"') {
        kernel_name = format!("\"{kernel_name}\"");
    }
    cargo.env(
        "RUSTFLAGS",
        format!("--cfg kernel_name={}", kernel_name),
    );

    if options.lto {
        cargo.env(
            format!("CARGO_PROFILE_{}_LTO", options.profile.to_string()),
            "true",
        );
    }

    if options.stack_probes {
        unimplemented!("Don't know how to configure")
    }

    let build = cargo
        .stderr(Stdio::inherit())
        .current_dir(&path)
        .output()
        .expect("Failed to execute cargo build");

    let stdout = String::from_utf8(build.stdout).unwrap();
    if build.status.success() {
        get_compiler_artifact(&stdout).unwrap_or_else(|| {
            eprintln!("--- build output ---\n{stdout}");
            panic!("build artifact not found in (supposedly successful) build output (see above)");
        })
    } else {
        panic!("Cargo build did not complete successfully (see above)");
    }
}

pub fn build_kernel(path: impl AsRef<Path>, options: CargoOptions) -> PathBuf {
    let path: &Path = &path.as_ref();
    invoke_cargo(path.to_path_buf(), options)
}
