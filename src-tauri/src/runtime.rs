use std::{
    fs,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, atomic::Ordering},
    time::{Duration, Instant},
};

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager};
use tokio::process::Command;

use crate::{
    Controller,
    job::ProcessJob,
    model::{DSH_VERSION, NODE_VERSION, SetupPhase, SetupProgress},
    paths::{AppPaths, safe_archive_path},
};

const NODE_ARCHIVE_URL: &str = concat!(
    "https://nodejs.org/dist/v",
    env!("DSH_NODE_VERSION"),
    "/node-v",
    env!("DSH_NODE_VERSION"),
    "-",
    env!("DSH_RUNTIME_ARCHITECTURE"),
    ".zip"
);
const NODE_ARCHIVE_ROOT: &str = concat!(
    "node-v",
    env!("DSH_NODE_VERSION"),
    "-",
    env!("DSH_RUNTIME_ARCHITECTURE")
);
const NODE_ARCHIVE_SHA256: &str = env!("DSH_NODE_ARCHIVE_SHA256");
const PACKAGE_JSON: &[u8] = include_bytes!("../../payload/package.json");
const PACKAGE_LOCK: &[u8] = include_bytes!("../../payload/package-lock.json");

#[derive(Clone, Debug)]
pub struct ResolvedRuntime {
    pub node: PathBuf,
    pub dsh: PathBuf,
    pub version: String,
    pub runtime_directory: PathBuf,
}

#[derive(Debug, Deserialize)]
struct PackageManifest {
    name: String,
    version: String,
    bin: PackageBin,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum PackageBin {
    String(String),
    Map(std::collections::BTreeMap<String, String>),
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimePointer {
    format_version: u32,
    package: String,
    version: String,
    relative_path: String,
}

pub fn resolve_runtime(paths: &AppPaths) -> Result<ResolvedRuntime, String> {
    resolve_pointer(paths, &paths.current)
}

fn resolve_pointer(paths: &AppPaths, pointer_path: &Path) -> Result<ResolvedRuntime, String> {
    let pointer: RuntimePointer =
        serde_json::from_slice(&fs::read(pointer_path).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
    if pointer.format_version != 1 || pointer.package != "@deepseek-ai/dsh" {
        return Err("The active runtime pointer is invalid.".to_owned());
    }
    let relative = Path::new(&pointer.relative_path);
    let dsh_root = safe_archive_path(&paths.root, relative)?;
    resolve_candidate(paths, &dsh_root, &pointer.version)
}

pub fn resolve_or_recover_runtime(paths: &AppPaths) -> Result<Option<ResolvedRuntime>, String> {
    match resolve_runtime(paths) {
        Ok(runtime) => Ok(Some(runtime)),
        Err(current_error) => {
            if !paths.previous.is_file() {
                return if paths.current.exists() {
                    Err(current_error)
                } else {
                    Ok(None)
                };
            }
            let previous = resolve_pointer(paths, &paths.previous).map_err(|previous_error| {
                format!(
                    "The current runtime is invalid ({current_error}); the previous runtime is also invalid ({previous_error})."
                )
            })?;
            let bytes = fs::read(&paths.previous).map_err(|error| error.to_string())?;
            crate::atomic_file::write(&paths.current, &bytes)?;
            Ok(Some(previous))
        }
    }
}

pub fn resolve_candidate(
    paths: &AppPaths,
    dsh_root: &Path,
    declared_version: &str,
) -> Result<ResolvedRuntime, String> {
    let runtime_directory = dsh_root.join("runtime");
    let package_root = runtime_directory.join("node_modules/@deepseek-ai/dsh");
    let manifest_path = package_root.join("package.json");
    let manifest: PackageManifest =
        serde_json::from_slice(&fs::read(manifest_path).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
    if manifest.name != "@deepseek-ai/dsh" || manifest.version != declared_version {
        return Err("The active DSH package does not match its pointer.".to_owned());
    }
    let relative_bin = match manifest.bin {
        PackageBin::String(value) => value,
        PackageBin::Map(values) => values
            .get("dsh")
            .cloned()
            .ok_or_else(|| "The DSH package has no dsh binary.".to_owned())?,
    };
    let dsh = safe_archive_path(&package_root, Path::new(&relative_bin))?;
    let node = paths.node_executable();
    let npm_cli = paths.npm_cli();
    if !node.is_file() || !npm_cli.is_file() || !dsh.is_file() {
        return Err("The private Harness runtime is incomplete.".to_owned());
    }
    Ok(ResolvedRuntime {
        node,
        dsh,
        version: manifest.version,
        runtime_directory,
    })
}

pub async fn install(
    app: AppHandle,
    controller: Arc<Controller>,
) -> Result<ResolvedRuntime, String> {
    controller
        .paths
        .ensure()
        .map_err(|error| error.to_string())?;
    progress(
        &app,
        &controller,
        SetupPhase::Preparing,
        4.0,
        "preparing",
        None,
    )
    .await;
    let resource_seed = app
        .path()
        .resource_dir()
        .ok()
        .map(|directory| directory.join("runtime-seed.zip"));
    if resource_seed.as_ref().is_some_and(|path| path.is_file()) {
        install_offline(&app, &controller, resource_seed.as_ref().unwrap()).await?;
    } else {
        install_online(&app, &controller).await?;
    }
    progress(
        &app,
        &controller,
        SetupPhase::Validating,
        91.0,
        "validating",
        None,
    )
    .await;
    let resolved = resolve_candidate(&controller.paths, &controller.paths.dsh_root, DSH_VERSION)?;
    validate_runtime(&app, &controller, &resolved).await?;
    Ok(resolved)
}

async fn install_online(app: &AppHandle, controller: &Arc<Controller>) -> Result<(), String> {
    if !private_node_is_valid(app, controller).await {
        progress(app, controller, SetupPhase::Node, 10.0, "node", None).await;
        let archive = controller
            .paths
            .staging
            .join(format!("{NODE_ARCHIVE_ROOT}.zip"));
        download_node(app, controller, &archive).await?;
        progress(app, controller, SetupPhase::Node, 46.0, "extracting", None).await;
        let destination = controller.paths.staging.join("node-extracted");
        remove_if_exists(&destination)?;
        let archive_copy = archive.clone();
        let destination_copy = destination.clone();
        tokio::task::spawn_blocking(move || extract_node_archive(&archive_copy, &destination_copy))
            .await
            .map_err(|error| error.to_string())??;
        activate_directory(&destination, &controller.paths.node_root)?;
        let _ = fs::remove_file(archive);
    }
    progress(app, controller, SetupPhase::Dsh, 57.0, "dsh", None).await;
    install_dsh(app, controller, DSH_VERSION, false).await
}

pub(crate) async fn private_node_is_valid(app: &AppHandle, controller: &Arc<Controller>) -> bool {
    if !private_node_files_exist(&controller.paths) {
        return false;
    }
    run_checked(
        app,
        controller,
        &controller.paths.node_executable(),
        &["--version"],
        &controller.paths.root,
        Duration::from_secs(30),
    )
    .await
    .is_ok_and(|version| version.trim() == format!("v{NODE_VERSION}"))
}

fn private_node_files_exist(paths: &AppPaths) -> bool {
    paths.node_executable().is_file() && paths.npm_cli().is_file()
}

async fn install_offline(
    app: &AppHandle,
    controller: &Arc<Controller>,
    seed: &Path,
) -> Result<(), String> {
    progress(app, controller, SetupPhase::Node, 15.0, "node", None).await;
    let extraction = controller.paths.staging.join("offline-seed");
    remove_if_exists(&extraction)?;
    let seed = seed.to_owned();
    let extraction_copy = extraction.clone();
    tokio::task::spawn_blocking(move || extract_seed_archive(&seed, &extraction_copy))
        .await
        .map_err(|error| error.to_string())??;
    let node = extraction.join("node");
    let dsh = extraction.join("dsh").join(DSH_VERSION);
    if !node.join("node.exe").is_file()
        || !dsh
            .join("runtime/node_modules/@deepseek-ai/dsh/package.json")
            .is_file()
    {
        return Err("The offline runtime seed is incomplete.".to_owned());
    }
    activate_directory(&node, &controller.paths.node_root)?;
    progress(app, controller, SetupPhase::Dsh, 64.0, "dsh", None).await;
    activate_directory(&dsh, &controller.paths.dsh_root)?;
    remove_if_exists(&extraction)
}

async fn download_node(
    app: &AppHandle,
    controller: &Arc<Controller>,
    destination: &Path,
) -> Result<(), String> {
    let response = reqwest::Client::new()
        .get(NODE_ARCHIVE_URL)
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?;
    let total = response.content_length().unwrap_or(0);
    let mut stream = response.bytes_stream();
    let mut file = tokio::fs::File::create(destination)
        .await
        .map_err(|error| error.to_string())?;
    let mut digest = Sha256::new();
    let mut received = 0u64;
    let mut last_reported_percent = 10.0;
    let mut last_report = Instant::now();
    use tokio::io::AsyncWriteExt;
    while let Some(chunk) = stream.next().await {
        if controller.shutdown.load(Ordering::SeqCst) {
            let _ = tokio::fs::remove_file(destination).await;
            return Err("The download was cancelled because Harness is exiting.".to_owned());
        }
        let bytes = chunk.map_err(|error| error.to_string())?;
        file.write_all(&bytes)
            .await
            .map_err(|error| error.to_string())?;
        digest.update(&bytes);
        received += bytes.len() as u64;
        if total > 0 {
            let percent = 10.0 + 34.0 * received as f64 / total as f64;
            if percent - last_reported_percent >= 0.5
                || last_report.elapsed() >= Duration::from_millis(150)
            {
                progress(app, controller, SetupPhase::Node, percent, "node", None).await;
                last_reported_percent = percent;
                last_report = Instant::now();
            }
        }
    }
    file.flush().await.map_err(|error| error.to_string())?;
    let actual = hex::encode(digest.finalize());
    if actual != NODE_ARCHIVE_SHA256 {
        let _ = tokio::fs::remove_file(destination).await;
        return Err("The downloaded Node.js archive failed SHA-256 verification.".to_owned());
    }
    Ok(())
}

async fn install_dsh(
    app: &AppHandle,
    controller: &Arc<Controller>,
    version: &str,
    ignore_scripts_only: bool,
) -> Result<(), String> {
    let stage = controller.paths.staging.join(format!("dsh-{version}"));
    remove_if_exists(&stage)?;
    let runtime = stage.join("runtime");
    fs::create_dir_all(&runtime).map_err(|error| error.to_string())?;
    fs::write(runtime.join("package.json"), PACKAGE_JSON).map_err(|error| error.to_string())?;
    fs::write(runtime.join("package-lock.json"), PACKAGE_LOCK)
        .map_err(|error| error.to_string())?;
    run_npm(
        app,
        controller,
        &[
            "ci",
            "--omit=dev",
            "--no-audit",
            "--no-fund",
            "--ignore-scripts",
            "--registry=https://registry.npmjs.org/",
        ],
        &runtime,
        Duration::from_secs(900),
    )
    .await?;
    if !ignore_scripts_only {
        run_npm(
            app,
            controller,
            &["rebuild", "--no-audit", "--no-fund"],
            &runtime,
            Duration::from_secs(900),
        )
        .await?;
    }
    let manifest: PackageManifest = serde_json::from_slice(
        &fs::read(runtime.join("node_modules/@deepseek-ai/dsh/package.json"))
            .map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    if manifest.name != "@deepseek-ai/dsh" || manifest.version != version {
        return Err("npm installed an unexpected DSH package version.".to_owned());
    }
    activate_directory(&stage, &controller.paths.dsh_root)
}

pub async fn validate_runtime(
    app: &AppHandle,
    controller: &Arc<Controller>,
    runtime: &ResolvedRuntime,
) -> Result<(), String> {
    let node_version = run_checked(
        app,
        controller,
        &runtime.node,
        &["--version"],
        &runtime.runtime_directory,
        Duration::from_secs(30),
    )
    .await?;
    if node_version.trim() != format!("v{NODE_VERSION}") {
        return Err(format!(
            "Node.js reported {} instead of v{}.",
            node_version.trim(),
            NODE_VERSION
        ));
    }
    let output = run_checked(
        app,
        controller,
        &runtime.node,
        &[runtime.dsh.to_string_lossy().as_ref(), "--version"],
        &runtime.runtime_directory,
        Duration::from_secs(60),
    )
    .await?;
    if output.trim() != runtime.version {
        return Err(format!(
            "DSH reported {} instead of {}.",
            output.trim(),
            runtime.version
        ));
    }
    Ok(())
}

pub async fn commit_install(
    app: &AppHandle,
    controller: &Arc<Controller>,
    runtime: &ResolvedRuntime,
) -> Result<(), String> {
    if controller.shutdown.load(Ordering::SeqCst) {
        return Err("Installation was cancelled before activation.".to_owned());
    }
    write_pointer(&controller.paths, &runtime.version)?;
    controller.snapshot.write().await.dsh_version = runtime.version.clone();
    progress(
        app,
        controller,
        SetupPhase::Complete,
        100.0,
        "complete",
        None,
    )
    .await;
    Ok(())
}

pub async fn run_checked(
    app: &AppHandle,
    controller: &Arc<Controller>,
    program: &Path,
    arguments: &[&str],
    working_directory: &Path,
    timeout: Duration,
) -> Result<String, String> {
    let mut command = hidden_command(program);
    command
        .args(arguments)
        .current_dir(working_directory)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    prepend_private_path(&mut command, &controller.paths.node_root);
    let child = command.spawn().map_err(|error| error.to_string())?;
    let job = ProcessJob::assign(&child)?;
    let output = tokio::select! {
        result = tokio::time::timeout(timeout, child.wait_with_output()) => {
            result
                .map_err(|_| format!("{} timed out.", program.display()))?
                .map_err(|error| error.to_string())?
        }
        _ = wait_for_shutdown(controller) => {
            drop(job);
            return Err("The operation was cancelled because Harness is exiting.".to_owned());
        }
    };
    drop(job);
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if !stdout.is_empty() {
        controller.logs.write(app, "stdout", stdout.clone());
    }
    if !stderr.is_empty() {
        controller.logs.write(app, "stderr", stderr.clone());
    }
    if !output.status.success() {
        return Err(format!(
            "{} exited with {}: {}",
            program.display(),
            output.status,
            if stderr.is_empty() { stdout } else { stderr }
        ));
    }
    Ok(stdout)
}

async fn wait_for_shutdown(controller: &Arc<Controller>) {
    while !controller.shutdown.load(Ordering::SeqCst) {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

pub async fn run_npm(
    app: &AppHandle,
    controller: &Arc<Controller>,
    arguments: &[&str],
    working_directory: &Path,
    timeout: Duration,
) -> Result<String, String> {
    let npm_cli = controller.paths.npm_cli().to_string_lossy().to_string();
    let mut npm_arguments = Vec::with_capacity(arguments.len() + 1);
    npm_arguments.push(npm_cli.as_str());
    npm_arguments.extend_from_slice(arguments);
    run_checked(
        app,
        controller,
        &controller.paths.node_executable(),
        &npm_arguments,
        working_directory,
        timeout,
    )
    .await
}

fn hidden_command(program: &Path) -> Command {
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        command.creation_flags(0x08000000);
    }
    command
}

pub fn prepend_private_path(command: &mut Command, node_root: &Path) {
    let current = std::env::var_os("PATH").unwrap_or_default();
    let mut paths = vec![node_root.to_owned()];
    paths.extend(std::env::split_paths(&current));
    if let Ok(joined) = std::env::join_paths(paths) {
        command.env("PATH", joined);
    }
}

fn extract_node_archive(archive: &Path, destination: &Path) -> Result<(), String> {
    let file = fs::File::open(archive).map_err(|error| error.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|error| error.to_string())?;
    fs::create_dir_all(destination).map_err(|error| error.to_string())?;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(|error| error.to_string())?;
        let enclosed = entry
            .enclosed_name()
            .ok_or_else(|| "The Node archive contains an unsafe path.".to_owned())?;
        let mut components = enclosed.components();
        let root = components
            .next()
            .ok_or_else(|| "The Node archive contains an empty path.".to_owned())?;
        if root.as_os_str() != NODE_ARCHIVE_ROOT {
            return Err("The Node archive has an unexpected root directory.".to_owned());
        }
        let relative: PathBuf = components.collect();
        if relative.as_os_str().is_empty() {
            continue;
        }
        let output = safe_archive_path(destination, &relative)?;
        if entry.is_dir() {
            fs::create_dir_all(&output).map_err(|error| error.to_string())?;
        } else {
            if let Some(parent) = output.parent() {
                fs::create_dir_all(parent).map_err(|error| error.to_string())?;
            }
            let mut target = fs::File::create(output).map_err(|error| error.to_string())?;
            std::io::copy(&mut entry, &mut target).map_err(|error| error.to_string())?;
        }
    }
    if !destination.join("node.exe").is_file() {
        return Err("The Node archive did not contain node.exe.".to_owned());
    }
    Ok(())
}

fn extract_seed_archive(archive: &Path, destination: &Path) -> Result<(), String> {
    let file = fs::File::open(archive).map_err(|error| error.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|error| error.to_string())?;
    fs::create_dir_all(destination).map_err(|error| error.to_string())?;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(|error| error.to_string())?;
        let enclosed = entry
            .enclosed_name()
            .ok_or_else(|| "The offline seed contains an unsafe path.".to_owned())?;
        let output = safe_archive_path(destination, &enclosed)?;
        if entry.is_dir() {
            fs::create_dir_all(&output).map_err(|error| error.to_string())?;
        } else {
            if let Some(parent) = output.parent() {
                fs::create_dir_all(parent).map_err(|error| error.to_string())?;
            }
            let mut target = fs::File::create(output).map_err(|error| error.to_string())?;
            std::io::copy(&mut entry, &mut target).map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

pub(crate) fn activate_directory(source: &Path, destination: &Path) -> Result<(), String> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("runtime");
    let backup = destination.with_file_name(format!(".{name}.backup"));
    remove_if_exists(&backup)?;
    if destination.exists() {
        fs::rename(destination, &backup).map_err(|error| error.to_string())?;
    }
    if let Err(error) = fs::rename(source, destination) {
        if backup.exists() {
            let _ = fs::rename(&backup, destination);
        }
        return Err(error.to_string());
    }
    let _ = remove_if_exists(&backup);
    Ok(())
}

fn write_pointer(paths: &AppPaths, version: &str) -> Result<(), String> {
    let pointer = RuntimePointer {
        format_version: 1,
        package: "@deepseek-ai/dsh".to_owned(),
        version: version.to_owned(),
        relative_path: format!("dsh/{version}"),
    };
    let mut bytes = serde_json::to_vec_pretty(&pointer).map_err(|error| error.to_string())?;
    bytes.push(b'\n');
    if paths.current.exists() {
        let current = fs::read(&paths.current).map_err(|error| error.to_string())?;
        crate::atomic_file::write(&paths.previous, &current)?;
    }
    crate::atomic_file::write(&paths.current, &bytes)
}

pub fn activate_pointer(paths: &AppPaths, version: &str) -> Result<(), String> {
    write_pointer(paths, version)
}

fn remove_if_exists(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    if path.is_dir() {
        fs::remove_dir_all(path).map_err(|error| error.to_string())
    } else {
        fs::remove_file(path).map_err(|error| error.to_string())
    }
}

pub fn discard_install_staging(paths: &AppPaths) {
    for path in [
        paths.staging.join(format!("{NODE_ARCHIVE_ROOT}.zip")),
        paths.staging.join("node-extracted"),
        paths.staging.join("offline-seed"),
        paths.staging.join(format!("dsh-{DSH_VERSION}")),
        paths.dsh_root.clone(),
    ] {
        let _ = remove_if_exists(&path);
    }
}

pub fn cleanup_staging(paths: &AppPaths, staged_version: Option<&str>) -> bool {
    let preserved =
        staged_version.map(|version| (version, paths.staging.join(format!("update-{version}"))));
    let mut staged_update_is_valid = false;
    let Ok(entries) = fs::read_dir(&paths.staging) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let keep = preserved.as_ref().is_some_and(|(version, expected)| {
            path == *expected && resolve_candidate(paths, &path, version).is_ok()
        });
        if keep {
            staged_update_is_valid = true;
        } else {
            let _ = remove_if_exists(&path);
        }
    }
    staged_update_is_valid
}

async fn progress(
    app: &AppHandle,
    controller: &Arc<Controller>,
    phase: SetupPhase,
    percent: f64,
    message_key: &str,
    detail: Option<String>,
) {
    {
        let mut snapshot = controller.snapshot.write().await;
        snapshot.setup_phase = phase;
        snapshot.progress = percent;
        snapshot.message_key = message_key.to_owned();
        snapshot.setup_complete = phase == SetupPhase::Complete;
    }
    let event = SetupProgress {
        phase,
        percent,
        message_key: message_key.to_owned(),
        detail,
    };
    let _ = app.emit("setup://progress", event);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_is_not_current_until_activation() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let paths = AppPaths::from_root(temporary.path().join("app"));
        fs::create_dir_all(paths.node_executable().parent().expect("node parent"))
            .expect("node directory");
        fs::write(paths.node_executable(), b"node").expect("node executable");
        fs::create_dir_all(paths.npm_cli().parent().expect("npm parent")).expect("npm directory");
        fs::write(paths.npm_cli(), b"npm").expect("npm cli");

        let package = paths.dsh_root.join("runtime/node_modules/@deepseek-ai/dsh");
        fs::create_dir_all(package.join("bin")).expect("package directory");
        fs::write(
            package.join("package.json"),
            format!(
                r#"{{"name":"@deepseek-ai/dsh","version":"{DSH_VERSION}","bin":"bin/dsh.js"}}"#
            ),
        )
        .expect("package manifest");
        fs::write(package.join("bin/dsh.js"), b"dsh").expect("dsh binary");

        let candidate =
            resolve_candidate(&paths, &paths.dsh_root, DSH_VERSION).expect("candidate runtime");
        assert_eq!(candidate.version, DSH_VERSION);
        assert!(!paths.current.exists());

        activate_pointer(&paths, DSH_VERSION).expect("activate candidate");
        assert_eq!(
            resolve_runtime(&paths).expect("current runtime").version,
            DSH_VERSION
        );
    }

    #[test]
    fn invalid_current_recovers_the_previous_runtime() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let paths = AppPaths::from_root(temporary.path().join("app"));
        fs::create_dir_all(paths.node_executable().parent().expect("node parent"))
            .expect("node directory");
        fs::write(paths.node_executable(), b"node").expect("node executable");
        fs::create_dir_all(paths.npm_cli().parent().expect("npm parent")).expect("npm directory");
        fs::write(paths.npm_cli(), b"npm").expect("npm cli");
        let package = paths.dsh_root.join("runtime/node_modules/@deepseek-ai/dsh");
        fs::create_dir_all(package.join("bin")).expect("package directory");
        fs::write(
            package.join("package.json"),
            format!(
                r#"{{"name":"@deepseek-ai/dsh","version":"{DSH_VERSION}","bin":"bin/dsh.js"}}"#
            ),
        )
        .expect("package manifest");
        fs::write(package.join("bin/dsh.js"), b"dsh").expect("dsh binary");
        activate_pointer(&paths, DSH_VERSION).expect("initial pointer");
        fs::copy(&paths.current, &paths.previous).expect("previous pointer");
        fs::write(&paths.current, b"not json").expect("corrupt current pointer");

        let recovered = resolve_or_recover_runtime(&paths)
            .expect("recovery result")
            .expect("recovered runtime");

        assert_eq!(recovered.version, DSH_VERSION);
        assert_eq!(
            resolve_runtime(&paths).expect("restored current").version,
            DSH_VERSION
        );
    }

    #[test]
    fn directory_activation_restores_the_previous_destination_on_failure() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let destination = temporary.path().join("runtime");
        fs::create_dir_all(&destination).expect("destination");
        fs::write(destination.join("marker"), b"old").expect("old marker");

        assert!(activate_directory(&temporary.path().join("missing"), &destination).is_err());
        assert_eq!(
            fs::read(destination.join("marker")).expect("restored marker"),
            b"old"
        );
    }

    #[test]
    fn node_runtime_without_npm_is_not_reusable() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let paths = AppPaths::from_root(temporary.path().join("app"));
        fs::create_dir_all(paths.node_executable().parent().expect("node parent"))
            .expect("node directory");
        fs::write(paths.node_executable(), b"node").expect("node executable");

        assert!(!private_node_files_exist(&paths));
    }
}
