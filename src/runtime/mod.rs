use std::sync::LazyLock;

use tempfile::TempDir;

pub mod firmware;
pub mod workload;

static SCCACHE_DIR: LazyLock<TempDir> = LazyLock::new(|| TempDir::new().unwrap());
