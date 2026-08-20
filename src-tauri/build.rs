fn main() {
    let manifest_dir =
        std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let config_path = manifest_dir.join("../build-config.json");
    let config: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&config_path).expect("read build-config.json"))
            .expect("parse build-config.json");
    for (field, environment) in [
        ("nodeVersion", "DSH_NODE_VERSION"),
        ("nodeArchiveSha256", "DSH_NODE_ARCHIVE_SHA256"),
        ("dshVersion", "DSH_UPSTREAM_VERSION"),
        ("architecture", "DSH_RUNTIME_ARCHITECTURE"),
    ] {
        let value = config[field]
            .as_str()
            .unwrap_or_else(|| panic!("build-config.json is missing {field}"));
        println!("cargo:rustc-env={environment}={value}");
    }
    println!("cargo:rerun-if-changed={}", config_path.display());
    tauri_build::build()
}
