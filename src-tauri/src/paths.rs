use std::path::{Component, Path, PathBuf};

use crate::model::{DSH_VERSION, NODE_VERSION};

#[derive(Clone, Debug)]
pub struct AppPaths {
    pub root: PathBuf,
    pub node_root: PathBuf,
    pub dsh_root: PathBuf,
    pub staging: PathBuf,
    pub logs: PathBuf,
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
            settings: root.join("settings.json"),
            current: root.join("current.json"),
            previous: root.join("previous.json"),
            root,
        }
    }

    pub fn ensure(&self) -> std::io::Result<()> {
        for directory in [&self.root, &self.staging, &self.logs] {
            std::fs::create_dir_all(directory)?;
        }
        Ok(())
    }

    pub fn node_executable(&self) -> PathBuf {
        self.node_root.join("node.exe")
    }

    pub fn npm_cli(&self) -> PathBuf {
        self.node_root.join("node_modules/npm/bin/npm-cli.js")
    }

    pub fn runtime_directory(&self) -> PathBuf {
        self.dsh_root.join("runtime")
    }

    pub fn dsh_manifest(&self) -> PathBuf {
        self.runtime_directory()
            .join("node_modules/@deepseek-ai/dsh/package.json")
    }
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
}
