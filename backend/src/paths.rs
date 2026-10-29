use std::path::PathBuf;

/// `<data dir>/plugin-data/@ryu+token-table/token-table.db`.
pub fn database_path() -> PathBuf {
    ryu_sidecar_runtime::plugin_data_dir("@ryu/token-table").join("token-table.db")
}
