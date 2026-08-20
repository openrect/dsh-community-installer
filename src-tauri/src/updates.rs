use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

use chrono::Utc;
use tauri::{AppHandle, Emitter, Manager};

use crate::{
    Controller,
    model::{UpdateDecision, UpdatePhase, UpdateState, UpdateTarget},
};

const SCRIPT_POLICY: &[u8] = include_bytes!("../../payload/script-policy.json");

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScriptPolicy {
    schema_version: u32,
    allowed: BTreeSet<String>,
}

pub async fn check(
    app: AppHandle,
    controller: Arc<Controller>,
    manual: bool,
) -> Result<(), String> {
    if controller.update_running.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    let result = check_inner(&app, &controller, manual).await;
    controller.update_running.store(false, Ordering::SeqCst);
    match completion_notice_phase(manual, &result) {
        Some(UpdatePhase::Current) => {
            set_update(
                &app,
                &controller,
                Some(UpdateState {
                    target: UpdateTarget::Controller,
                    phase: UpdatePhase::Current,
                    version: None,
                    progress: None,
                    message_key: "upToDate".to_owned(),
                }),
            )
            .await;
            show_prompt(&app);
        }
        Some(UpdatePhase::Failed) => {
            let message_key = match result.as_ref().err() {
                Some(error) if error.contains("unapproved install scripts") => {
                    "scriptPolicyBlocked"
                }
                Some(error) if error.contains("requires a newer private Node.js runtime") => {
                    "nodeVersionBlocked"
                }
                _ => "updateCheckFailed",
            };
            set_update(
                &app,
                &controller,
                Some(UpdateState {
                    target: UpdateTarget::Controller,
                    phase: UpdatePhase::Failed,
                    version: None,
                    progress: None,
                    message_key: message_key.to_owned(),
                }),
            )
            .await;
            show_prompt(&app);
        }
        None if !result.as_ref().is_ok_and(|found| *found) => {
            set_update(&app, &controller, None).await
        }
        None => {}
        Some(_) => unreachable!("only terminal manual-check phases are returned"),
    }
    result.map(|_| ())
}

async fn check_inner(
    app: &AppHandle,
    controller: &Arc<Controller>,
    manual: bool,
) -> Result<bool, String> {
    let _maintenance = controller.maintenance.lock().await;
    {
        let mut settings = controller.settings.write().await;
        settings.controller.last_attempt_utc = Some(Utc::now());
        settings.save(&controller.paths.settings)?;
    }
    set_update(
        app,
        controller,
        Some(UpdateState {
            target: UpdateTarget::Controller,
            phase: UpdatePhase::Checking,
            version: None,
            progress: None,
            message_key: "checkingUpdates".to_owned(),
        }),
    )
    .await;
    let controller_result = check_controller(app, controller, manual).await;
    if controller_result.is_ok() {
        record_success(controller, UpdateTarget::Controller).await?;
    }
    let should_check_dsh = manual || controller.settings.read().await.auto_download;
    let result = match controller_result {
        Ok(true) => Ok(true),
        Ok(false) if !should_check_dsh => Ok(false),
        Ok(false) => {
            record_attempt(controller, UpdateTarget::Dsh).await?;
            let dsh_result = check_dsh(app, controller, manual).await;
            if dsh_result.is_ok() {
                record_success(controller, UpdateTarget::Dsh).await?;
            }
            merge_update_results(None, dsh_result)
        }
        Err(controller_error) if !should_check_dsh => Err(format!(
            "Controller update check failed: {controller_error}"
        )),
        Err(controller_error) => {
            controller.logs.write(
                app,
                "app",
                format!("Controller update check failed: {controller_error}"),
            );
            record_attempt(controller, UpdateTarget::Dsh).await?;
            let dsh_result = check_dsh(app, controller, manual).await;
            if dsh_result.is_ok() {
                record_success(controller, UpdateTarget::Dsh).await?;
            }
            merge_update_results(Some(controller_error), dsh_result)
        }
    };
    if let Err(error) = &result {
        controller
            .logs
            .write(app, "app", format!("Update check failed: {error}"));
    }
    result
}

async fn record_attempt(controller: &Arc<Controller>, target: UpdateTarget) -> Result<(), String> {
    let mut settings = controller.settings.write().await;
    let channel = match target {
        UpdateTarget::Controller => &mut settings.controller,
        UpdateTarget::Dsh => &mut settings.dsh,
    };
    channel.last_attempt_utc = Some(Utc::now());
    settings.save(&controller.paths.settings)
}

async fn record_success(controller: &Arc<Controller>, target: UpdateTarget) -> Result<(), String> {
    let mut settings = controller.settings.write().await;
    let channel = match target {
        UpdateTarget::Controller => &mut settings.controller,
        UpdateTarget::Dsh => &mut settings.dsh,
    };
    channel.last_successful_check_utc = Some(Utc::now());
    settings.save(&controller.paths.settings)
}

fn merge_update_results(
    controller_error: Option<String>,
    dsh_result: Result<bool, String>,
) -> Result<bool, String> {
    match (controller_error, dsh_result) {
        (_, Ok(true)) => Ok(true),
        (None, Ok(false)) => Ok(false),
        (Some(error), Ok(false)) => Err(format!("Controller update check failed: {error}")),
        (None, Err(error)) => Err(format!("DSH update check failed: {error}")),
        (Some(controller), Err(dsh)) => Err(format!(
            "Controller update check failed: {controller}; DSH update check failed: {dsh}"
        )),
    }
}

fn completion_notice_phase(manual: bool, result: &Result<bool, String>) -> Option<UpdatePhase> {
    if !manual {
        return None;
    }
    match result {
        Ok(false) => Some(UpdatePhase::Current),
        Err(_) => Some(UpdatePhase::Failed),
        Ok(true) => None,
    }
}

pub async fn respond(
    app: AppHandle,
    controller: Arc<Controller>,
    target: UpdateTarget,
    version: String,
    decision: UpdateDecision,
) -> Result<(), String> {
    let _maintenance = controller.maintenance.lock().await;
    let update = controller
        .snapshot
        .read()
        .await
        .update
        .clone()
        .ok_or_else(|| "No update is available.".to_owned())?;
    if !update_request_matches(&update, target, &version) {
        return Err("The update request no longer matches the available update.".to_owned());
    }
    match decision {
        UpdateDecision::Later => {
            if let Some(window) = app.get_webview_window("prompt") {
                let _ = window.hide();
            }
        }
        UpdateDecision::Skip => {
            let mut settings = controller.settings.write().await;
            let channel = if target == UpdateTarget::Controller {
                &mut settings.controller
            } else {
                &mut settings.dsh
            };
            channel.skipped_version = Some(version.clone());
            channel.staged_version = None;
            settings.save(&controller.paths.settings)?;
            if target == UpdateTarget::Dsh {
                let stage = controller.paths.staging.join(format!("update-{version}"));
                let _ = fs::remove_dir_all(stage);
            }
            set_update(&app, &controller, None).await;
            if let Some(window) = app.get_webview_window("prompt") {
                let _ = window.hide();
            }
        }
        UpdateDecision::Install => {
            set_update(
                &app,
                &controller,
                Some(UpdateState {
                    target,
                    phase: UpdatePhase::Installing,
                    version: Some(version.clone()),
                    progress: None,
                    message_key: "installingUpdate".to_owned(),
                }),
            )
            .await;
            let result = match target {
                UpdateTarget::Controller => install_controller(&app, &controller, &version).await,
                UpdateTarget::Dsh => install_dsh(&app, &controller, &version).await,
            };
            if let Err(error) = result {
                controller
                    .logs
                    .write(&app, "app", format!("Update installation failed: {error}"));
                set_update(
                    &app,
                    &controller,
                    Some(UpdateState {
                        target,
                        phase: UpdatePhase::Failed,
                        version: Some(version),
                        progress: None,
                        message_key: if error.contains("unapproved install scripts") {
                            "scriptPolicyBlocked".to_owned()
                        } else {
                            "updateInstallFailed".to_owned()
                        },
                    }),
                )
                .await;
                show_prompt(&app);
                return Err(error);
            }
        }
    }
    Ok(())
}

fn update_request_matches(update: &UpdateState, target: UpdateTarget, version: &str) -> bool {
    matches!(update.phase, UpdatePhase::Available | UpdatePhase::Ready)
        && update.target == target
        && update.version.as_deref() == Some(version)
}

pub fn schedule(app: AppHandle, controller: Arc<Controller>) {
    if controller.updates_scheduled.swap(true, Ordering::SeqCst) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(120)).await;
        loop {
            let result = check(app.clone(), controller.clone(), false).await;
            let delay = if result.is_ok() {
                24 * 60 * 60
            } else {
                60 * 60
            };
            tokio::time::sleep(Duration::from_secs(delay)).await;
        }
    });
}

async fn check_controller(
    app: &AppHandle,
    controller: &Arc<Controller>,
    manual: bool,
) -> Result<bool, String> {
    use tauri_plugin_updater::UpdaterExt;
    let update = app
        .updater()
        .map_err(|error| error.to_string())?
        .check()
        .await
        .map_err(|error| error.to_string())?;
    let Some(update) = update else {
        return Ok(false);
    };
    let version = update.version.clone();
    if !manual
        && controller
            .settings
            .read()
            .await
            .controller
            .skipped_version
            .as_deref()
            == Some(&version)
    {
        return Ok(false);
    }
    let notify = manual
        || controller
            .settings
            .read()
            .await
            .controller
            .last_notified_version
            .as_deref()
            != Some(&version);
    controller.logs.write(
        app,
        "app",
        format!("Controller update {version} is available."),
    );
    {
        let mut settings = controller.settings.write().await;
        settings.controller.staged_version = None;
        settings.controller.last_notified_version = Some(version.clone());
        settings.save(&controller.paths.settings)?;
    }
    set_update(
        app,
        controller,
        Some(UpdateState {
            target: UpdateTarget::Controller,
            phase: UpdatePhase::Available,
            version: Some(version),
            progress: None,
            message_key: "controllerUpdate".to_owned(),
        }),
    )
    .await;
    if notify {
        show_prompt(app);
    }
    Ok(true)
}

#[derive(serde::Deserialize)]
struct RegistryDist {
    integrity: String,
}

#[derive(serde::Deserialize)]
struct RegistryEngines {
    node: Option<String>,
}

#[derive(serde::Deserialize)]
struct RegistryMetadata {
    name: String,
    version: String,
    dist: RegistryDist,
    engines: Option<RegistryEngines>,
}

async fn check_dsh(
    app: &AppHandle,
    controller: &Arc<Controller>,
    manual: bool,
) -> Result<bool, String> {
    let metadata: RegistryMetadata = reqwest::Client::new()
        .get("https://registry.npmjs.org/@deepseek-ai%2Fdsh/latest")
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?
        .json()
        .await
        .map_err(|error| error.to_string())?;
    let current = semver::Version::parse(&controller.snapshot.read().await.dsh_version)
        .map_err(|error| error.to_string())?;
    let latest = semver::Version::parse(&metadata.version).map_err(|error| error.to_string())?;
    if metadata.name != "@deepseek-ai/dsh" {
        return Err("The npm registry returned an unexpected package.".to_owned());
    }
    if latest <= current
        || (!manual
            && controller
                .settings
                .read()
                .await
                .dsh
                .skipped_version
                .as_deref()
                == Some(&metadata.version))
    {
        return Ok(false);
    }
    {
        let settings = controller.settings.read().await;
        if settings.dsh.blocked_version.as_deref() == Some(metadata.version.as_str())
            && settings.dsh.blocked_node_version.as_deref() == Some(crate::model::NODE_VERSION)
        {
            return Err("This DSH release requires a newer private Node.js runtime.".to_owned());
        }
    }
    if let Some(requirement) = metadata
        .engines
        .as_ref()
        .and_then(|engines| engines.node.as_deref())
        && !node_satisfies(app, controller, requirement).await?
    {
        let mut settings = controller.settings.write().await;
        settings.dsh.blocked_version = Some(metadata.version.clone());
        settings.dsh.blocked_node_version = Some(crate::model::NODE_VERSION.to_owned());
        settings.save(&controller.paths.settings)?;
        return Err("This DSH release requires a newer private Node.js runtime.".to_owned());
    }
    {
        let mut settings = controller.settings.write().await;
        if settings.dsh.blocked_version.take().is_some()
            || settings.dsh.blocked_node_version.take().is_some()
        {
            settings.save(&controller.paths.settings)?;
        }
    }
    let notify = manual
        || controller
            .settings
            .read()
            .await
            .dsh
            .last_notified_version
            .as_deref()
            != Some(&metadata.version);
    if !staged_dsh_is_valid(
        &controller.paths,
        &metadata.version,
        &metadata.dist.integrity,
    )? {
        stage_dsh(app, controller, &metadata.version, &metadata.dist.integrity).await?;
    } else {
        controller.logs.write(
            app,
            "app",
            format!("Reusing validated staged DSH {}.", metadata.version),
        );
    }
    {
        let mut settings = controller.settings.write().await;
        settings.dsh.staged_version = Some(metadata.version.clone());
        settings.dsh.last_notified_version = Some(metadata.version.clone());
        settings.save(&controller.paths.settings)?;
    }
    set_update(
        app,
        controller,
        Some(UpdateState {
            target: UpdateTarget::Dsh,
            phase: UpdatePhase::Ready,
            version: Some(metadata.version),
            progress: Some(100.0),
            message_key: "dshUpdate".to_owned(),
        }),
    )
    .await;
    if notify {
        show_prompt(app);
    }
    Ok(true)
}

async fn stage_dsh(
    app: &AppHandle,
    controller: &Arc<Controller>,
    version: &str,
    expected_integrity: &str,
) -> Result<(), String> {
    if fs2::available_space(&controller.paths.root).map_err(|error| error.to_string())?
        < 700 * 1024 * 1024
    {
        return Err(
            "At least 700 MB of free disk space is required to download an update.".to_owned(),
        );
    }
    let stage = controller.paths.staging.join(format!("update-{version}"));
    if stage.exists() {
        fs::remove_dir_all(&stage).map_err(|error| error.to_string())?;
    }
    let runtime = stage.join("runtime");
    fs::create_dir_all(&runtime).map_err(|error| error.to_string())?;
    let manifest = serde_json::json!({
        "name": "dsh-community-staged-runtime",
        "version": "1.0.0",
        "private": true,
        "allowScripts": {},
        "dependencies": { "@deepseek-ai/dsh": version }
    });
    fs::write(
        runtime.join("package.json"),
        serde_json::to_vec_pretty(&manifest).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    crate::runtime::run_npm(
        app,
        controller,
        &[
            "install",
            "--omit=dev",
            "--no-audit",
            "--no-fund",
            "--ignore-scripts",
            "--package-lock=true",
            "--registry=https://registry.npmjs.org/",
        ],
        &runtime,
        Duration::from_secs(900),
    )
    .await?;
    let installed: serde_json::Value = serde_json::from_slice(
        &fs::read(runtime.join("node_modules/@deepseek-ai/dsh/package.json"))
            .map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    if installed.get("version").and_then(|value| value.as_str()) != Some(version) {
        let _ = fs::remove_dir_all(stage);
        return Err("The staged DSH package version is invalid.".to_owned());
    }
    let lock: serde_json::Value = serde_json::from_slice(
        &fs::read(runtime.join("package-lock.json")).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let locked = &lock["packages"]["node_modules/@deepseek-ai/dsh"];
    if locked["version"].as_str() != Some(version)
        || locked["integrity"].as_str() != Some(expected_integrity)
    {
        let _ = fs::remove_dir_all(stage);
        return Err("The staged npm lock entry failed integrity validation.".to_owned());
    }
    write_staged_script_policy(&runtime)?;
    Ok(())
}

fn staged_dsh_is_valid(
    paths: &crate::paths::AppPaths,
    version: &str,
    expected_integrity: &str,
) -> Result<bool, String> {
    let stage = paths.staging.join(format!("update-{version}"));
    let runtime = stage.join("runtime");
    if crate::runtime::resolve_candidate(paths, &stage, version).is_err() {
        return Ok(false);
    }
    let Ok(bytes) = fs::read(runtime.join("package-lock.json")) else {
        return Ok(false);
    };
    let Ok(lock) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return Ok(false);
    };
    let locked = &lock["packages"]["node_modules/@deepseek-ai/dsh"];
    if locked["version"].as_str() != Some(version)
        || locked["integrity"].as_str() != Some(expected_integrity)
    {
        return Ok(false);
    }
    match validate_script_policy(&runtime) {
        Ok(_) => Ok(true),
        Err(error) if error.contains("unapproved install scripts") => Err(error),
        Err(_) => Ok(false),
    }
}

fn write_staged_script_policy(runtime: &Path) -> Result<(), String> {
    let requested = validate_script_policy(runtime)?;
    let policy: BTreeMap<_, _> = requested
        .into_iter()
        .map(|package| (package, true))
        .collect();
    let manifest_path = runtime.join("package.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
    manifest["allowScripts"] = serde_json::to_value(policy).map_err(|error| error.to_string())?;
    let mut bytes = serde_json::to_vec_pretty(&manifest).map_err(|error| error.to_string())?;
    bytes.push(b'\n');
    crate::atomic_file::write(&manifest_path, &bytes)
}

fn validate_script_policy(runtime: &Path) -> Result<BTreeSet<String>, String> {
    let policy: ScriptPolicy =
        serde_json::from_slice(SCRIPT_POLICY).map_err(|error| error.to_string())?;
    if policy.schema_version != 1 {
        return Err("The install script policy version is unsupported.".to_owned());
    }
    let mut requested = BTreeSet::new();
    inspect_node_modules(&runtime.join("node_modules"), &mut requested)?;
    let unknown: Vec<_> = requested.difference(&policy.allowed).cloned().collect();
    if !unknown.is_empty() {
        return Err(format!(
            "The DSH update was blocked because it requests unapproved install scripts: {}",
            unknown.join(", ")
        ));
    }
    Ok(requested)
}

fn inspect_package(path: &Path, packages: &mut BTreeSet<String>) -> Result<(), String> {
    let manifest = path.join("package.json");
    if manifest.is_file() {
        let value: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest).map_err(|error| error.to_string())?)
                .map_err(|error| error.to_string())?;
        let has_install_script = value
            .get("scripts")
            .and_then(|scripts| scripts.as_object())
            .is_some_and(|scripts| {
                ["preinstall", "install", "postinstall", "prepare"]
                    .iter()
                    .any(|name| scripts.get(*name).is_some_and(|script| script.is_string()))
            });
        if has_install_script {
            let name = value
                .get("name")
                .and_then(|name| name.as_str())
                .ok_or_else(|| format!("{} has no package name.", manifest.display()))?;
            let version = value
                .get("version")
                .and_then(|version| version.as_str())
                .ok_or_else(|| format!("{} has no package version.", manifest.display()))?;
            packages.insert(format!("{name}@{version}"));
        }
    }
    let nested = path.join("node_modules");
    if nested.is_dir() {
        inspect_node_modules(&nested, packages)?;
    }
    Ok(())
}

fn inspect_node_modules(directory: &Path, packages: &mut BTreeSet<String>) -> Result<(), String> {
    for entry in fs::read_dir(directory).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        if !entry
            .file_type()
            .map_err(|error| error.to_string())?
            .is_dir()
        {
            continue;
        }
        let path = entry.path();
        if entry.file_name().to_string_lossy().starts_with('@') {
            for scoped in fs::read_dir(&path).map_err(|error| error.to_string())? {
                let scoped = scoped.map_err(|error| error.to_string())?;
                if scoped
                    .file_type()
                    .map_err(|error| error.to_string())?
                    .is_dir()
                {
                    inspect_package(&scoped.path(), packages)?;
                }
            }
        } else {
            inspect_package(&path, packages)?;
        }
    }
    Ok(())
}

async fn node_satisfies(
    app: &AppHandle,
    controller: &Arc<Controller>,
    requirement: &str,
) -> Result<bool, String> {
    let script = "const path=require('path');const semver=require(path.join(process.argv[1],'node_modules','npm','node_modules','semver'));process.exit(semver.satisfies(process.version,process.argv[2],{includePrerelease:true})?0:42)";
    let node_root = controller.paths.node_root.to_string_lossy().to_string();
    let result = crate::runtime::run_checked(
        app,
        controller,
        &controller.paths.node_executable(),
        &["-e", script, &node_root, requirement],
        &controller.paths.root,
        Duration::from_secs(30),
    )
    .await;
    match result {
        Ok(_) => Ok(true),
        Err(error) if error.contains("42") => Ok(false),
        Err(error) => Err(error),
    }
}

async fn install_dsh(
    app: &AppHandle,
    controller: &Arc<Controller>,
    version: &str,
) -> Result<(), String> {
    let stage = controller.paths.staging.join(format!("update-{version}"));
    let runtime = stage.join("runtime");
    write_staged_script_policy(&runtime)?;
    crate::runtime::run_npm(
        app,
        controller,
        &["rebuild", "--no-audit", "--no-fund"],
        &runtime,
        Duration::from_secs(900),
    )
    .await?;
    let staged_runtime = crate::runtime::resolve_candidate(&controller.paths, &stage, version)?;
    crate::runtime::validate_runtime(app, controller, &staged_runtime).await?;
    crate::service::stop(controller).await;
    let target = controller.paths.root.join("dsh").join(version);
    if let Err(error) = crate::runtime::activate_directory(&stage, &target) {
        let restart = crate::service::start(app.clone(), controller.clone(), false, true).await;
        return Err(match restart {
            Ok(()) => format!(
                "The update could not be prepared and the previous DSH version was restarted: {error}"
            ),
            Err(restart_error) => format!(
                "The update could not be prepared ({error}) and the previous DSH version could not be restarted ({restart_error})."
            ),
        });
    }
    let candidate = match crate::runtime::resolve_candidate(&controller.paths, &target, version) {
        Ok(candidate) => candidate,
        Err(error) => {
            discard_failed_candidate(app, controller, &target);
            let restart = crate::service::start(app.clone(), controller.clone(), false, true).await;
            return Err(match restart {
                Ok(()) => format!(
                    "The moved update could not be resolved and the previous DSH version was restarted: {error}"
                ),
                Err(restart_error) => format!(
                    "The moved update could not be resolved ({error}) and the previous DSH version could not be restarted ({restart_error})."
                ),
            });
        }
    };
    if let Err(error) =
        crate::service::start_candidate_and_wait(app.clone(), controller.clone(), candidate).await
    {
        crate::service::stop(controller).await;
        discard_failed_candidate(app, controller, &target);
        let restart = crate::service::start(app.clone(), controller.clone(), false, true).await;
        return Err(match restart {
            Ok(()) => format!(
                "The updated service failed to start; the previous DSH version was restarted: {error}"
            ),
            Err(restart_error) => format!(
                "The updated service failed to start ({error}) and the previous DSH version could not be restarted ({restart_error})."
            ),
        });
    }
    // The pointer replacement is the transaction commit point. Failures after this
    // point cannot make a successfully activated runtime appear to have failed.
    if let Err(error) = crate::runtime::activate_pointer(&controller.paths, version) {
        crate::service::stop(controller).await;
        discard_failed_candidate(app, controller, &target);
        let restart = crate::service::start(app.clone(), controller.clone(), false, true).await;
        return Err(match restart {
            Ok(()) => format!(
                "The updated service passed validation but could not be activated; the previous DSH version was restarted: {error}"
            ),
            Err(restart_error) => format!(
                "The update could not be activated ({error}) and the previous DSH version could not be restarted ({restart_error})."
            ),
        });
    }
    controller.snapshot.write().await.dsh_version = version.to_owned();
    {
        let mut settings = controller.settings.write().await;
        settings.dsh.staged_version = None;
        if let Err(error) = settings.save(&controller.paths.settings) {
            controller.logs.write(
                app,
                "app",
                format!("Warning: the committed update settings could not be saved: {error}"),
            );
        }
    }
    if let Err(error) = prune_versions(controller) {
        controller.logs.write(
            app,
            "app",
            format!("Warning: old DSH versions could not be pruned: {error}"),
        );
    }
    set_update(app, controller, None).await;
    if let Some(window) = app.get_webview_window("prompt") {
        let _ = window.hide();
    }
    use tauri_plugin_opener::OpenerExt;
    if let Err(error) = app
        .opener()
        .open_url(crate::model::HARNESS_URL, None::<&str>)
    {
        controller.logs.write(
            app,
            "app",
            format!("Warning: Harness could not be opened after the committed update: {error}"),
        );
    }
    Ok(())
}

fn discard_failed_candidate(app: &AppHandle, controller: &Arc<Controller>, target: &Path) {
    if target.is_dir()
        && let Err(error) = fs::remove_dir_all(target)
    {
        controller.logs.write(
            app,
            "app",
            format!("Warning: a failed DSH candidate could not be removed: {error}"),
        );
    }
}

fn prune_versions(controller: &Arc<Controller>) -> Result<(), String> {
    let mut keep = std::collections::BTreeSet::new();
    for pointer in [&controller.paths.current, &controller.paths.previous] {
        if let Ok(bytes) = fs::read(pointer)
            && let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes)
            && let Some(version) = value.get("version").and_then(|value| value.as_str())
        {
            keep.insert(version.to_owned());
        }
    }
    let root = controller.paths.root.join("dsh");
    if !root.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(root).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        if entry
            .file_type()
            .map_err(|error| error.to_string())?
            .is_dir()
        {
            let name = entry.file_name().to_string_lossy().to_string();
            if !keep.contains(&name) {
                fs::remove_dir_all(entry.path()).map_err(|error| error.to_string())?;
            }
        }
    }
    Ok(())
}

async fn install_controller(
    app: &AppHandle,
    controller: &Arc<Controller>,
    expected_version: &str,
) -> Result<(), String> {
    use tauri_plugin_updater::UpdaterExt;
    let update = app
        .updater()
        .map_err(|error| error.to_string())?
        .check()
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "The controller update is no longer available.".to_owned())?;
    if update.version != expected_version {
        return Err("The controller update no longer matches the confirmed version.".to_owned());
    }
    let mut downloaded = 0u64;
    let bytes = update
        .download(
            |chunk, _| {
                downloaded += chunk as u64;
            },
            || {},
        )
        .await
        .map_err(|error| error.to_string())?;
    controller.logs.write(
        app,
        "app",
        format!("Downloaded controller update {expected_version} ({downloaded} bytes)."),
    );
    crate::service::stop(controller).await;
    if let Err(error) = update.install(bytes) {
        let restart = crate::service::start(app.clone(), controller.clone(), false, true).await;
        return Err(match restart {
            Ok(()) => format!("The Installer update could not be applied: {error}"),
            Err(restart_error) => format!(
                "The Installer update could not be applied ({error}) and Harness could not be restarted ({restart_error})."
            ),
        });
    }
    app.restart();
}

async fn set_update(app: &AppHandle, controller: &Arc<Controller>, update: Option<UpdateState>) {
    controller.snapshot.write().await.update = update.clone();
    if let Some(update) = update {
        let _ = app.emit("update://state", update);
    }
    let _ = app.emit("ui://refresh", ());
}

pub async fn dismiss_notice(app: &AppHandle, controller: &Arc<Controller>) {
    set_update(app, controller, None).await;
    if let Some(window) = app.get_webview_window("prompt") {
        let _ = window.hide();
    }
}

fn show_prompt(app: &AppHandle) {
    crate::show_update_prompt(app);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staged_script_policy_pins_only_approved_installed_script_packages() {
        let directory = tempfile::tempdir().unwrap();
        let runtime = directory.path();
        fs::write(runtime.join("package.json"), r#"{"private":true}"#).unwrap();
        let scripted = runtime.join("node_modules/koffi");
        let plain = runtime.join("node_modules/plain");
        fs::create_dir_all(&scripted).unwrap();
        fs::create_dir_all(&plain).unwrap();
        fs::write(
            scripted.join("package.json"),
            r#"{"name":"koffi","version":"3.1.5","scripts":{"postinstall":"node setup.js"}}"#,
        )
        .unwrap();
        fs::write(
            plain.join("package.json"),
            r#"{"name":"plain","version":"4.5.6"}"#,
        )
        .unwrap();

        write_staged_script_policy(runtime).unwrap();

        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(runtime.join("package.json")).unwrap()).unwrap();
        assert_eq!(manifest["allowScripts"]["koffi@3.1.5"], true);
        assert!(manifest["allowScripts"].get("plain@4.5.6").is_none());
    }

    #[test]
    fn staged_script_policy_rejects_unknown_install_scripts() {
        let directory = tempfile::tempdir().unwrap();
        let runtime = directory.path();
        fs::write(runtime.join("package.json"), r#"{"private":true}"#).unwrap();
        let scripted = runtime.join("node_modules/unreviewed");
        fs::create_dir_all(&scripted).unwrap();
        fs::write(
            scripted.join("package.json"),
            r#"{"name":"unreviewed","version":"1.0.0","scripts":{"install":"node setup.js"}}"#,
        )
        .unwrap();

        let error = write_staged_script_policy(runtime).unwrap_err();
        assert!(error.contains("unreviewed@1.0.0"));
    }

    #[test]
    fn validated_staged_dsh_can_be_reused_after_restart() {
        let directory = tempfile::tempdir().unwrap();
        let paths = crate::paths::AppPaths::from_root(directory.path().join("app"));
        fs::create_dir_all(paths.node_executable().parent().unwrap()).unwrap();
        fs::write(paths.node_executable(), b"node").unwrap();
        fs::create_dir_all(paths.npm_cli().parent().unwrap()).unwrap();
        fs::write(paths.npm_cli(), b"npm").unwrap();
        let version = "0.1.0-rc.8";
        let integrity = "sha512-test";
        let runtime = paths.staging.join(format!("update-{version}/runtime"));
        let package = runtime.join("node_modules/@deepseek-ai/dsh");
        fs::create_dir_all(package.join("bin")).unwrap();
        fs::write(
            package.join("package.json"),
            format!(r#"{{"name":"@deepseek-ai/dsh","version":"{version}","bin":"bin/dsh.js"}}"#),
        )
        .unwrap();
        fs::write(package.join("bin/dsh.js"), b"dsh").unwrap();
        fs::write(runtime.join("package.json"), r#"{"private":true}"#).unwrap();
        fs::write(
            runtime.join("package-lock.json"),
            format!(
                r#"{{"packages":{{"node_modules/@deepseek-ai/dsh":{{"version":"{version}","integrity":"{integrity}"}}}}}}"#
            ),
        )
        .unwrap();

        assert!(staged_dsh_is_valid(&paths, version, integrity).unwrap());
    }

    #[test]
    fn manual_checks_report_current_and_failed_results() {
        assert_eq!(
            completion_notice_phase(true, &Ok(false)),
            Some(UpdatePhase::Current)
        );
        assert_eq!(
            completion_notice_phase(true, &Err("offline".to_owned())),
            Some(UpdatePhase::Failed)
        );
        assert_eq!(completion_notice_phase(true, &Ok(true)), None);
        assert_eq!(completion_notice_phase(false, &Ok(false)), None);
        assert_eq!(
            completion_notice_phase(false, &Err("offline".to_owned())),
            None
        );
    }

    #[test]
    fn update_decision_must_match_target_version_and_phase() {
        let update = UpdateState {
            target: UpdateTarget::Dsh,
            phase: UpdatePhase::Ready,
            version: Some("0.1.0-rc.8".to_owned()),
            progress: Some(100.0),
            message_key: "dshUpdate".to_owned(),
        };
        assert!(update_request_matches(
            &update,
            UpdateTarget::Dsh,
            "0.1.0-rc.8"
        ));
        assert!(!update_request_matches(
            &update,
            UpdateTarget::Controller,
            "0.1.0-rc.8"
        ));
        assert!(!update_request_matches(
            &update,
            UpdateTarget::Dsh,
            "0.1.0-rc.9"
        ));
    }

    #[test]
    fn a_failed_controller_check_cannot_claim_everything_is_current() {
        assert_eq!(merge_update_results(None, Ok(false)), Ok(false));
        assert_eq!(
            merge_update_results(Some("feed".to_owned()), Ok(true)),
            Ok(true)
        );
        assert_eq!(
            merge_update_results(Some("feed".to_owned()), Ok(false)),
            Err("Controller update check failed: feed".to_owned())
        );
        assert_eq!(
            merge_update_results(Some("feed".to_owned()), Err("registry".to_owned())),
            Err(
                "Controller update check failed: feed; DSH update check failed: registry"
                    .to_owned()
            )
        );
    }
}
