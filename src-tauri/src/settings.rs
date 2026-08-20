use std::{fs, path::Path};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::model::Locale;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateChannelSettings {
    pub last_attempt_utc: Option<DateTime<Utc>>,
    pub last_successful_check_utc: Option<DateTime<Utc>>,
    pub last_notified_version: Option<String>,
    pub skipped_version: Option<String>,
    pub staged_version: Option<String>,
    pub blocked_version: Option<String>,
    #[serde(default)]
    pub blocked_node_version: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub schema_version: u32,
    pub locale: Locale,
    pub auto_download: bool,
    pub controller: UpdateChannelSettings,
    pub dsh: UpdateChannelSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            schema_version: 1,
            locale: Locale::English,
            auto_download: true,
            controller: UpdateChannelSettings::default(),
            dsh: UpdateChannelSettings::default(),
        }
    }
}

impl Settings {
    pub fn load(path: &Path) -> Self {
        let Ok(bytes) = fs::read(path) else {
            return Self::default();
        };
        let Ok(settings) = serde_json::from_slice::<Self>(&bytes) else {
            return Self::default();
        };
        if settings.schema_version == 1 {
            settings
        } else {
            Self::default()
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let parent = path
            .parent()
            .ok_or_else(|| "Settings path has no parent.".to_owned())?;
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        let mut bytes = serde_json::to_vec_pretty(self).map_err(|error| error.to_string())?;
        bytes.push(b'\n');
        crate::atomic_file::write(path, &bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_atomically() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let settings = Settings::default();
        settings.save(&path).unwrap();
        let loaded = Settings::load(&path);
        assert_eq!(loaded.schema_version, 1);
        assert_eq!(loaded.locale, Locale::English);
        assert!(loaded.auto_download);
    }
}
