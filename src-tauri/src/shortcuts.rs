use tauri::{AppHandle, Manager};

#[cfg(windows)]
pub fn publish_desktop(app: &AppHandle) -> Result<bool, String> {
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let staged = executable
        .parent()
        .ok_or_else(|| "The application directory is unavailable.".to_owned())?
        .join("DSH Community.lnk");
    if !staged.is_file() {
        return Ok(false);
    }
    let desktop = app
        .path()
        .desktop_dir()
        .map_err(|error| error.to_string())?;
    publish_staged(&staged, &desktop)
}

#[cfg(windows)]
fn publish_staged(staged: &std::path::Path, desktop: &std::path::Path) -> Result<bool, String> {
    std::fs::create_dir_all(desktop).map_err(|error| error.to_string())?;
    let destination = desktop.join("DSH Community.lnk");
    std::fs::copy(staged, &destination).map_err(|error| error.to_string())?;
    std::fs::remove_file(staged).map_err(|error| error.to_string())?;
    Ok(true)
}

#[cfg(not(windows))]
pub fn publish_desktop(_app: &AppHandle) -> Result<bool, String> {
    Ok(false)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn shortcut_is_published_only_when_requested() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let staged = temporary.path().join("installed/DSH Community.lnk");
        let desktop = temporary.path().join("desktop");
        std::fs::create_dir_all(staged.parent().expect("staged parent")).expect("staged directory");
        std::fs::write(&staged, b"shortcut").expect("staged shortcut");

        assert!(!desktop.join("DSH Community.lnk").exists());
        assert!(publish_staged(&staged, &desktop).expect("publish shortcut"));
        assert!(!staged.exists());
        assert_eq!(
            std::fs::read(desktop.join("DSH Community.lnk")).expect("desktop shortcut"),
            b"shortcut"
        );
    }
}
