fn main() {
    let manifest_dir =
        std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let config_path = manifest_dir.join("../build-config.json");
    let config: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&config_path).expect("read build-config.json"))
            .expect("parse build-config.json");
    for (field, environment) in [
        ("nodeVersion", "DSH_NODE_VERSION"),
        ("pnpmVersion", "DSH_PNPM_VERSION"),
        ("nodeArchiveSha256", "DSH_NODE_ARCHIVE_SHA256"),
        ("dshVersion", "DSH_UPSTREAM_VERSION"),
        ("architecture", "DSH_RUNTIME_ARCHITECTURE"),
    ] {
        let value = config[field]
            .as_str()
            .unwrap_or_else(|| panic!("build-config.json is missing {field}"));
        println!("cargo:rustc-env={environment}={value}");
    }
    let dist_tags = config["dshDistTags"]
        .as_array()
        .filter(|tags| !tags.is_empty())
        .unwrap_or_else(|| panic!("build-config.json is missing dshDistTags"));
    if !dist_tags.iter().all(|tag| tag.as_str().is_some()) {
        panic!("build-config.json dshDistTags must contain only strings");
    }
    println!(
        "cargo:rustc-env=DSH_DIST_TAGS={}",
        serde_json::to_string(dist_tags).expect("serialize dshDistTags")
    );
    println!("cargo:rerun-if-changed={}", config_path.display());
    tauri_build::build()
}
