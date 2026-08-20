use std::{
    collections::VecDeque,
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, SystemTime},
};

use chrono::Utc;
use tauri::{AppHandle, Emitter};

use crate::model::LogLine;

pub struct LogStore {
    file: Mutex<std::fs::File>,
    lines: Mutex<VecDeque<LogLine>>,
}

impl LogStore {
    pub fn open(directory: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(directory).map_err(|error| error.to_string())?;
        prune_logs(
            directory,
            Duration::from_secs(7 * 24 * 60 * 60),
            50 * 1024 * 1024,
        );
        let path = directory.join(format!("harness-{}.log", Utc::now().format("%Y%m%d")));
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|error| error.to_string())?;
        Ok(Self {
            file: Mutex::new(file),
            lines: Mutex::new(VecDeque::with_capacity(500)),
        })
    }

    pub fn write(&self, app: &AppHandle, source: &str, value: impl Into<String>) {
        let line = LogLine {
            timestamp: Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            source: source.to_owned(),
            line: value.into(),
        };
        if let Ok(mut file) = self.file.lock() {
            let _ = writeln!(file, "[{}] [{}] {}", line.timestamp, line.source, line.line);
            let _ = file.flush();
        }
        if let Ok(mut lines) = self.lines.lock() {
            if lines.len() == 500 {
                lines.pop_front();
            }
            lines.push_back(line.clone());
        }
        let _ = app.emit("log://line", line);
    }

    pub fn recent(&self) -> Vec<LogLine> {
        self.lines
            .lock()
            .map(|lines| lines.iter().cloned().collect())
            .unwrap_or_default()
    }
}

fn prune_logs(directory: &Path, max_age: Duration, max_bytes: u64) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    let mut logs: Vec<(PathBuf, SystemTime, u64)> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_name()?.to_str()?;
            if !name.starts_with("harness-") || !name.ends_with(".log") {
                return None;
            }
            let metadata = entry.metadata().ok()?;
            Some((path, metadata.modified().ok()?, metadata.len()))
        })
        .collect();
    logs.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| right.0.cmp(&left.0)));
    let now = SystemTime::now();
    let mut retained = 0_u64;
    for (path, modified, bytes) in logs {
        let expired = now.duration_since(modified).is_ok_and(|age| age > max_age);
        let exceeds_quota = retained.saturating_add(bytes) > max_bytes;
        if expired || exceeds_quota {
            let _ = std::fs::remove_file(path);
        } else {
            retained = retained.saturating_add(bytes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_retention_removes_older_files_over_the_quota() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let older = temporary.path().join("harness-20260101.log");
        let newer = temporary.path().join("harness-20260102.log");
        std::fs::write(&older, b"1234").expect("older log");
        std::fs::write(&newer, b"5678").expect("newer log");

        prune_logs(temporary.path(), Duration::MAX, 5);

        assert!(!older.exists());
        assert!(newer.exists());
    }
}
