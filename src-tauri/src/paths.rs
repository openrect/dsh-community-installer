use std::path::{Component, Path, PathBuf};

use crate::model::{DSH_VERSION, NODE_VERSION};

const PRIVATE_STATE_FORMAT: u32 = 2;

#[derive(Clone, Debug)]
pub struct AppPaths {
    pub root: PathBuf,
    pub node_root: PathBuf,
    pub dsh_root: PathBuf,
    pub staging: PathBuf,
    pub logs: PathBuf,
    pub corepack_home: PathBuf,
    pub pnpm_store: PathBuf,
    pub state_format: PathBuf,
    pub settings: PathBuf,
    pub current: PathBuf,
    pub previous: PathBuf,
}

impl AppPaths {
    pub fn discover() -> Result<Self, String> {
        #[cfg(windows)]
        let base = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .ok_or_else(|| "LOCALAPPDATA is unavailable.".to_owned())?;

        #[cfg(not(windows))]
        let base = std::env::var_os("HOME")
            .map(|home| PathBuf::from(home).join("Library/Application Support"))
            .ok_or_else(|| "The user application data directory is unavailable.".to_owned())?;

        Ok(Self::from_root(base.join("DSHCommunityInstaller")))
    }

    pub fn from_root(root: PathBuf) -> Self {
        Self {
            node_root: root
                .join("node")
                .join(NODE_VERSION)
                .join(env!("DSH_RUNTIME_ARCHITECTURE")),
            dsh_root: root.join("dsh").join(DSH_VERSION),
            staging: root.join("staging"),
            logs: root.join("logs"),
            corepack_home: root.join("package-managers/corepack"),
            pnpm_store: root.join("package-managers/pnpm-store"),
            state_format: root.join("format.json"),
            settings: root.join("settings.json"),
            current: root.join("current.json"),
            previous: root.join("previous.json"),
            root,
        }
    }

    pub fn ensure(&self) -> std::io::Result<()> {
        for directory in [
            &self.root,
            &self.staging,
            &self.logs,
            &self.corepack_home,
            &self.pnpm_store,
        ] {
            std::fs::create_dir_all(directory)?;
        }
        Ok(())
    }

    pub fn node_executable(&self) -> PathBuf {
        self.node_root.join("node.exe")
    }

    pub fn corepack_cli(&self) -> PathBuf {
        self.node_root
            .join("node_modules/corepack/dist/corepack.js")
    }

    pub fn runtime_directory(&self) -> PathBuf {
        self.dsh_root.join("runtime")
    }

    pub fn dsh_manifest(&self) -> PathBuf {
        self.runtime_directory()
            .join("node_modules/@deepseek-ai/dsh/package.json")
    }

    pub fn dsh_version_root(&self, version: &str) -> PathBuf {
        self.root.join("dsh").join(version)
    }
}

pub fn prepare_private_state(paths: &AppPaths) -> Result<(), String> {
    let compatible = fs_format(&paths.state_format) == Some(PRIVATE_STATE_FORMAT);
    if paths.root.exists() && !compatible {
        std::fs::remove_dir_all(&paths.root).map_err(|error| error.to_string())?;
    }
    paths.ensure().map_err(|error| error.to_string())?;
    let mut bytes = serde_json::to_vec_pretty(&serde_json::json!({
        "formatVersion": PRIVATE_STATE_FORMAT
    }))
    .map_err(|error| error.to_string())?;
    bytes.push(b'\n');
    crate::atomic_file::write(&paths.state_format, &bytes)
}

fn fs_format(path: &Path) -> Option<u32> {
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    value.get("formatVersion")?.as_u64()?.try_into().ok()
}

pub fn safe_archive_path(base: &Path, name: &Path) -> Result<PathBuf, String> {
    if name.is_absolute() {
        return Err("Archive entry uses an absolute path.".to_owned());
    }
    for component in name.components() {
        if matches!(
            component,
            Component::ParentDir | Component::Prefix(_) | Component::RootDir
        ) {
            return Err("Archive entry escapes the destination directory.".to_owned());
        }
    }
    Ok(base.join(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_paths_stay_below_the_destination() {
        let root = Path::new("C:/safe");
        assert!(safe_archive_path(root, Path::new("node/node.exe")).is_ok());
        assert!(safe_archive_path(root, Path::new("../outside.exe")).is_err());
        assert!(safe_archive_path(root, Path::new("C:/outside.exe")).is_err());
    }

    #[test]
    fn incompatible_private_state_is_removed_without_touching_user_data() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("DSHCommunityInstaller");
        let user_data = directory.path().join(".dsh");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&user_data).unwrap();
        std::fs::write(root.join("settings.json"), b"old").unwrap();
        std::fs::write(user_data.join("sessions.json"), b"keep").unwrap();

        let paths = AppPaths::from_root(root);
        prepare_private_state(&paths).unwrap();

        assert!(!paths.settings.exists());
        assert_eq!(fs_format(&paths.state_format), Some(PRIVATE_STATE_FORMAT));
        assert!(user_data.join("sessions.json").is_file());
    }
}
