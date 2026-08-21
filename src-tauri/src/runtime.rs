use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
    sync::mpsc,
};

use crate::{
    Controller,
    job::ProcessJob,
    model::{DSH_VERSION, NODE_VERSION, PNPM_VERSION, SetupPhase, SetupProgress},
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
pub const UPDATE_PAUSED_ERROR: &str = "The update was paused by the user.";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum PnpmStage {
    #[default]
    Preparing,
    Resolving,
    Importing,
    Lifecycle,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PnpmProgress {
    pub stage: PnpmStage,
    pub resolved_items: u64,
    pub reused_items: u64,
    pub downloaded_items: u64,
    pub added_items: u64,
    pub total_items: Option<u64>,
    pub elapsed_seconds: u64,
}

impl PnpmProgress {
    pub fn percent(&self) -> f64 {
        match self.stage {
            PnpmStage::Preparing => 5.0,
            PnpmStage::Resolving if self.total_items.is_none() => 10.0,
            PnpmStage::Resolving | PnpmStage::Importing => self.total_items.map_or(10.0, |total| {
                if total == 0 {
                    80.0
                } else {
                    25.0 + 55.0 * (self.added_items.min(total) as f64 / total as f64)
                }
            }),
            PnpmStage::Lifecycle => 85.0,
        }
    }
}

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
    if pointer.format_version != 2 || pointer.package != "@deepseek-ai/dsh" {
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
    let corepack_cli = paths.corepack_cli();
    if !node.is_file() || !corepack_cli.is_file() || !dsh.is_file() {
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
    let (installed_root, installed_version) =
        if resource_seed.as_ref().is_some_and(|path| path.is_file()) {
            install_offline(&app, &controller, resource_seed.as_ref().unwrap()).await?;
            (controller.paths.dsh_root.clone(), DSH_VERSION.to_owned())
        } else {
            install_online(&app, &controller).await?
        };
    progress(
        &app,
        &controller,
        SetupPhase::Validating,
        91.0,
        "validating",
        None,
    )
    .await;
    let resolved = resolve_candidate(&controller.paths, &installed_root, &installed_version)?;
    validate_runtime(&app, &controller, &resolved).await?;
    Ok(resolved)
}

async fn install_online(
    app: &AppHandle,
    controller: &Arc<Controller>,
) -> Result<(PathBuf, String), String> {
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
    let version = crate::updates::latest_compatible_dsh_version(app, controller).await?;
    let target = controller.paths.dsh_version_root(&version);
    install_online_dsh(app, controller, &version, &target).await?;
    Ok((target, version))
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
    paths.node_executable().is_file() && paths.corepack_cli().is_file()
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

async fn install_online_dsh(
    app: &AppHandle,
    controller: &Arc<Controller>,
    version: &str,
    target: &Path,
) -> Result<(), String> {
    let stage = controller.paths.staging.join(format!("setup-{version}"));
    remove_if_exists(&stage)?;
    let runtime = stage.join("runtime");
    fs::create_dir_all(&runtime).map_err(|error| error.to_string())?;
    let manifest = serde_json::json!({
        "name": "dsh-community-runtime",
        "version": "1.0.0",
        "private": true,
        "packageManager": format!("pnpm@{PNPM_VERSION}"),
        "dependencies": { "@deepseek-ai/dsh": version }
    });
    fs::write(
        runtime.join("package.json"),
        serde_json::to_vec_pretty(&manifest).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let progress_app = app.clone();
    let progress_controller = controller.clone();
    let progress_task = tokio::spawn(async move {
        while let Some(pnpm) = receiver.recv().await {
            package_progress(&progress_app, &progress_controller, SetupPhase::Dsh, &pnpm).await;
        }
    });
    let result = run_pnpm(
        app,
        controller,
        &runtime,
        Duration::from_secs(1200),
        false,
        sender,
    )
    .await;
    let _ = progress_task.await;
    if let Err(error) = result {
        let _ = remove_if_exists(&stage);
        return Err(error);
    }
    let manifest: PackageManifest = serde_json::from_slice(
        &fs::read(runtime.join("node_modules/@deepseek-ai/dsh/package.json"))
            .map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    if manifest.name != "@deepseek-ai/dsh" || manifest.version != version {
        let _ = remove_if_exists(&stage);
        return Err("pnpm installed an unexpected DSH package version.".to_owned());
    }
    activate_directory(&stage, target)
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
    run_checked_inner(
        app,
        controller,
        program,
        arguments,
        working_directory,
        timeout,
        false,
    )
    .await
}

async fn run_checked_inner(
    app: &AppHandle,
    controller: &Arc<Controller>,
    program: &Path,
    arguments: &[&str],
    working_directory: &Path,
    timeout: Duration,
    pause_with_update: bool,
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
        _ = wait_for_update_pause(controller), if pause_with_update => {
            drop(job);
            return Err(UPDATE_PAUSED_ERROR.to_owned());
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

async fn wait_for_update_pause(controller: &Arc<Controller>) {
    while !controller.update_cancel.load(Ordering::SeqCst) {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

pub async fn run_pnpm(
    app: &AppHandle,
    controller: &Arc<Controller>,
    working_directory: &Path,
    timeout: Duration,
    pause_with_update: bool,
    progress: mpsc::UnboundedSender<PnpmProgress>,
) -> Result<String, String> {
    let _ = progress.send(PnpmProgress::default());
    let node = controller.paths.node_executable();
    let arguments = pnpm_arguments(&controller.paths);
    let mut command = hidden_command(&node);
    command
        .args(&arguments)
        .current_dir(working_directory)
        .env("COREPACK_HOME", &controller.paths.corepack_home)
        .env("COREPACK_ENABLE_DOWNLOAD_PROMPT", "0")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    prepend_private_path(&mut command, &controller.paths.node_root);
    let mut child = command.spawn().map_err(|error| error.to_string())?;
    let mut job = Some(ProcessJob::assign(&child)?);
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "pnpm stdout is unavailable.".to_owned())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "pnpm stderr is unavailable.".to_owned())?;
    let started = Instant::now();
    let latest = Arc::new(StdMutex::new(PnpmProgress::default()));
    let ticker_done = Arc::new(AtomicBool::new(false));
    let stdout_progress = progress.clone();
    let stdout_latest = latest.clone();
    let stdout_task = tokio::spawn(async move {
        let mut parser = PnpmProgressParser::default();
        let mut lines = BufReader::new(stdout).lines();
        let mut tail = Vec::new();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(update) = parser.observe(&line, started.elapsed().as_secs()) {
                if let Ok(mut latest) = stdout_latest.lock() {
                    *latest = update.clone();
                }
                let _ = stdout_progress.send(update);
            } else if !line.trim().is_empty() {
                tail.push(line);
                if tail.len() > 40 {
                    tail.remove(0);
                }
            }
        }
        tail.join("\n")
    });
    let ticker_progress = progress.clone();
    let ticker_latest = latest.clone();
    let ticker_done_clone = ticker_done.clone();
    let ticker_task = tokio::spawn(async move {
        while !ticker_done_clone.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_secs(1)).await;
            if ticker_done_clone.load(Ordering::SeqCst) {
                break;
            }
            let mut update = ticker_latest
                .lock()
                .map(|state| state.clone())
                .unwrap_or_default();
            update.elapsed_seconds = started.elapsed().as_secs();
            let _ = ticker_progress.send(update);
        }
    });
    let stderr_app = app.clone();
    let stderr_controller = controller.clone();
    let stderr_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        let mut tail = Vec::new();
        while let Ok(Some(line)) = lines.next_line().await {
            if !line.trim().is_empty() {
                stderr_controller.logs.write(&stderr_app, "stderr", &line);
                tail.push(line);
                if tail.len() > 40 {
                    tail.remove(0);
                }
            }
        }
        tail.join("\n")
    });
    let status_result = tokio::select! {
        result = tokio::time::timeout(timeout, child.wait()) => {
            result.map_err(|_| format!("{} timed out.", node.display())).and_then(|result| result.map_err(|error| error.to_string()))
        }
        _ = wait_for_shutdown(controller) => {
            Err("The operation was cancelled because Harness is exiting.".to_owned())
        }
        _ = wait_for_update_pause(controller), if pause_with_update => {
            Err(UPDATE_PAUSED_ERROR.to_owned())
        }
    };
    let status = match status_result {
        Ok(status) => status,
        Err(error) => {
            drop(job.take());
            let _ = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
            ticker_done.store(true, Ordering::SeqCst);
            let _ = ticker_task.await;
            let _ = stdout_task.await;
            let _ = stderr_task.await;
            return Err(error);
        }
    };
    drop(job.take());
    ticker_done.store(true, Ordering::SeqCst);
    let _ = ticker_task.await;
    let stdout = stdout_task.await.map_err(|error| error.to_string())?;
    let stderr = stderr_task.await.map_err(|error| error.to_string())?;
    if !status.success() {
        return Err(format!(
            "pnpm exited with {status}: {}",
            if stderr.is_empty() { stdout } else { stderr }
        ));
    }
    Ok(stdout)
}

fn pnpm_arguments(paths: &AppPaths) -> Vec<String> {
    vec![
        paths.corepack_cli().to_string_lossy().to_string(),
        "pnpm".to_owned(),
        "install".to_owned(),
        "--ignore-workspace".to_owned(),
        "--prod".to_owned(),
        "--reporter=ndjson".to_owned(),
        "--config.node-linker=hoisted".to_owned(),
        "--config.package-import-method=copy".to_owned(),
        "--config.auto-install-peers=true".to_owned(),
        "--config.confirm-modules-purge=false".to_owned(),
        "--dangerously-allow-all-builds".to_owned(),
        "--registry=https://registry.npmjs.org/".to_owned(),
        format!("--store-dir={}", paths.pnpm_store.display()),
    ]
}

#[derive(Default)]
struct PnpmProgressParser {
    state: PnpmProgress,
    resolved: HashSet<String>,
    reused: HashSet<String>,
    downloaded: HashSet<String>,
    added: HashSet<String>,
}

impl PnpmProgressParser {
    fn observe(&mut self, line: &str, elapsed_seconds: u64) -> Option<PnpmProgress> {
        let value: serde_json::Value = serde_json::from_str(line).ok()?;
        let name = value.get("name")?.as_str()?;
        match name {
            "pnpm:progress" => {
                self.state.stage = self.state.stage.max(PnpmStage::Resolving);
                let status = value.get("status")?.as_str()?;
                let key = value
                    .get("packageId")
                    .or_else(|| value.get("pkgId"))
                    .or_else(|| value.get("package"))
                    .map(ToString::to_string)
                    .unwrap_or_else(|| line.to_owned());
                match status {
                    "resolved" => {
                        self.resolved.insert(key);
                    }
                    "found_in_store" => {
                        self.reused.insert(key);
                    }
                    "fetched" => {
                        self.downloaded.insert(key);
                    }
                    "imported" => {
                        self.state.stage = self.state.stage.max(PnpmStage::Importing);
                        self.added.insert(key);
                    }
                    _ => return None,
                }
            }
            "pnpm:stats" => {
                self.state.stage = self.state.stage.max(PnpmStage::Importing);
                self.state.total_items = value.get("added").and_then(serde_json::Value::as_u64);
            }
            "pnpm:stage" => {
                if value
                    .get("stage")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|stage| stage.starts_with("importing"))
                {
                    self.state.stage = self.state.stage.max(PnpmStage::Importing);
                }
            }
            "pnpm:lifecycle" => {
                self.state.stage = self.state.stage.max(PnpmStage::Lifecycle);
            }
            _ => return None,
        }
        self.state.resolved_items = self.resolved.len() as u64;
        self.state.reused_items = self.reused.len() as u64;
        self.state.downloaded_items = self.downloaded.len() as u64;
        self.state.added_items = self.added.len() as u64;
        self.state.elapsed_seconds = elapsed_seconds;
        Some(self.state.clone())
    }
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
        format_version: 2,
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
    ] {
        let _ = remove_if_exists(&path);
    }
    if let Ok(entries) = fs::read_dir(&paths.staging) {
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().starts_with("setup-") {
                let _ = remove_if_exists(&entry.path());
            }
        }
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
        resolved_items: None,
        reused_items: None,
        downloaded_items: None,
        added_items: None,
        total_items: None,
        elapsed_seconds: None,
    };
    let _ = app.emit("setup://progress", event);
}

async fn package_progress(
    app: &AppHandle,
    controller: &Arc<Controller>,
    phase: SetupPhase,
    pnpm: &PnpmProgress,
) {
    let message_key = match pnpm.stage {
        PnpmStage::Preparing => "preparingPackageManager",
        PnpmStage::Resolving => "resolvingDependencies",
        PnpmStage::Importing => "installingPackages",
        PnpmStage::Lifecycle => "runningInstallScripts",
    };
    let percent = pnpm.percent();
    {
        let mut snapshot = controller.snapshot.write().await;
        snapshot.setup_phase = phase;
        snapshot.progress = percent;
        snapshot.message_key = message_key.to_owned();
    }
    let _ = app.emit(
        "setup://progress",
        SetupProgress {
            phase,
            percent,
            message_key: message_key.to_owned(),
            detail: None,
            resolved_items: Some(pnpm.resolved_items),
            reused_items: Some(pnpm.reused_items),
            downloaded_items: Some(pnpm.downloaded_items),
            added_items: Some(pnpm.added_items),
            total_items: pnpm.total_items,
            elapsed_seconds: Some(pnpm.elapsed_seconds),
        },
    );
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
        fs::create_dir_all(paths.corepack_cli().parent().expect("corepack parent"))
            .expect("corepack directory");
        fs::write(paths.corepack_cli(), b"corepack").expect("corepack cli");

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
        fs::create_dir_all(paths.corepack_cli().parent().expect("corepack parent"))
            .expect("corepack directory");
        fs::write(paths.corepack_cli(), b"corepack").expect("corepack cli");
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
    fn old_pointer_format_is_rejected() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let paths = AppPaths::from_root(temporary.path().join("app"));
        fs::create_dir_all(&paths.root).expect("app directory");
        fs::write(
            &paths.current,
            format!(
                r#"{{"formatVersion":1,"package":"@deepseek-ai/dsh","version":"{DSH_VERSION}","relativePath":"dsh/{DSH_VERSION}"}}"#
            ),
        )
        .expect("old pointer");

        assert!(resolve_runtime(&paths).is_err());
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
    fn node_runtime_without_corepack_is_not_reusable() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let paths = AppPaths::from_root(temporary.path().join("app"));
        fs::create_dir_all(paths.node_executable().parent().expect("node parent"))
            .expect("node directory");
        fs::write(paths.node_executable(), b"node").expect("node executable");

        assert!(!private_node_files_exist(&paths));
    }

    #[test]
    fn pnpm_ndjson_reports_real_package_activity_and_ignores_bad_lines() {
        let mut parser = PnpmProgressParser::default();
        assert!(parser.observe("not json", 1).is_none());
        let resolved = parser
            .observe(
                r#"{"name":"pnpm:progress","status":"resolved","packageId":"a@1.0.0"}"#,
                2,
            )
            .unwrap();
        assert_eq!(resolved.resolved_items, 1);
        assert_eq!(resolved.percent(), 10.0);
        let stats = parser
            .observe(r#"{"name":"pnpm:stats","added":4}"#, 3)
            .unwrap();
        assert_eq!(stats.total_items, Some(4));
        assert_eq!(stats.percent(), 25.0);
        let reused = parser
            .observe(
                r#"{"name":"pnpm:progress","status":"found_in_store","packageId":"a@1.0.0"}"#,
                4,
            )
            .unwrap();
        assert_eq!(reused.reused_items, 1);
        let downloaded = parser
            .observe(
                r#"{"name":"pnpm:progress","status":"fetched","packageId":"b@1.0.0"}"#,
                5,
            )
            .unwrap();
        assert_eq!(downloaded.downloaded_items, 1);
        let added = parser
            .observe(
                r#"{"name":"pnpm:progress","status":"imported","packageId":"a@1.0.0"}"#,
                6,
            )
            .unwrap();
        assert_eq!(added.added_items, 1);
        assert!(added.percent() > 25.0);
        let lifecycle = parser
            .observe(r#"{"name":"pnpm:lifecycle","depPath":"a@1.0.0"}"#, 7)
            .unwrap();
        assert_eq!(lifecycle.stage, PnpmStage::Lifecycle);
        assert_eq!(lifecycle.percent(), 85.0);
    }

    #[test]
    fn pnpm_command_uses_private_corepack_store_and_fixed_install_layout() {
        let paths = AppPaths::from_root(PathBuf::from("C:/private-app"));
        let arguments = pnpm_arguments(&paths);

        assert_eq!(arguments[0], paths.corepack_cli().to_string_lossy());
        assert_eq!(
            &arguments[1..5],
            ["pnpm", "install", "--ignore-workspace", "--prod"]
        );
        for expected in [
            "--reporter=ndjson",
            "--config.node-linker=hoisted",
            "--config.package-import-method=copy",
            "--config.auto-install-peers=true",
            "--config.confirm-modules-purge=false",
            "--dangerously-allow-all-builds",
        ] {
            assert!(arguments.iter().any(|argument| argument == expected));
        }
        assert!(arguments.iter().any(|argument| {
            argument == &format!("--store-dir={}", paths.pnpm_store.display())
        }));
    }
}
