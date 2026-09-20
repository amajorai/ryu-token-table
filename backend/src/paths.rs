use std::path::PathBuf;

pub fn database_path() -> PathBuf {
    ryu_sidecar_runtime::ryu_dir().join("token-table.db")
}
