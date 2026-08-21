use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

use tauri::{AppHandle, Emitter, Manager};

use crate::{
    Controller,
    model::{UpdateDecision, UpdatePhase, UpdateState, UpdateTarget},
};

pub async fn check(
    app: AppHandle,
    controller: Arc<Controller>,
    manual: bool,
) -> Result<(), String> {
    check_with_mode(app, controller, manual, false).await
}

async fn check_with_mode(
    app: AppHandle,
    controller: Arc<Controller>,
    manual: bool,
    notify_available: bool,
) -> Result<(), String> {
    if controller.update_running.swap(true, Ordering::SeqCst) {
        if manual {
            show_prompt(&app);
        }
        return Ok(());
    }
    controller.update_cancel.store(false, Ordering::SeqCst);
    let result = check_inner(&app, &controller, manual, notify_available).await;
    controller.update_running.store(false, Ordering::SeqCst);
    if result
        .as_ref()
        .is_err_and(|error| error == crate::runtime::UPDATE_PAUSED_ERROR)
    {
        set_update(&app, &controller, None).await;
        return Ok(());
    }
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
                    resolved_items: None,
                    reused_items: None,
                    downloaded_items: None,
                    added_items: None,
                    total_items: None,
                    elapsed_seconds: None,
                    message_key: "upToDate".to_owned(),
                }),
            )
            .await;
            show_prompt(&app);
        }
        Some(UpdatePhase::Failed) => {
            let message_key = match result.as_ref().err() {
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
                    resolved_items: None,
                    reused_items: None,
                    downloaded_items: None,
                    added_items: None,
                    total_items: None,
                    elapsed_seconds: None,
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
    notify_available: bool,
) -> Result<bool, String> {
    let _maintenance = controller.maintenance.lock().await;
    set_update(
        app,
        controller,
        Some(UpdateState {
            target: UpdateTarget::Controller,
            phase: UpdatePhase::Checking,
            version: None,
            progress: None,
            resolved_items: None,
            reused_items: None,
            downloaded_items: None,
            added_items: None,
            total_items: None,
            elapsed_seconds: None,
            message_key: "checkingUpdates".to_owned(),
        }),
    )
    .await;
    if manual {
        show_prompt(app);
    }
    let controller_result = check_controller(app, controller, manual, notify_available).await;
    let should_check_dsh = manual || controller.settings.read().await.auto_check_dsh_updates;
    let result = match controller_result {
        Ok(true) => Ok(true),
        Ok(false) if !should_check_dsh => Ok(false),
        Ok(false) => {
            let dsh_result = check_dsh(app, controller, manual, notify_available).await;
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
            let dsh_result = check_dsh(app, controller, manual, notify_available).await;
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
            match target {
                UpdateTarget::Controller => {
                    settings.controller.skipped_version = Some(version.clone());
                }
                UpdateTarget::Dsh => {
                    settings.dsh.skipped_version = Some(version.clone());
                    settings.dsh.staged_version = None;
                }
            }
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
            if target == UpdateTarget::Dsh && update.phase == UpdatePhase::Available {
                controller.update_cancel.store(false, Ordering::SeqCst);
                controller.update_running.store(true, Ordering::SeqCst);
                set_dsh_indeterminate_state(
                    &app,
                    &controller,
                    &version,
                    UpdatePhase::Checking,
                    "downloadingUpdate",
                )
                .await;
                show_prompt(&app);
                let prepare_result = prepare_confirmed_dsh(&app, &controller, &version).await;
                controller.update_running.store(false, Ordering::SeqCst);
                if prepare_result
                    .as_ref()
                    .is_err_and(|error| error == crate::runtime::UPDATE_PAUSED_ERROR)
                {
                    set_update(&app, &controller, None).await;
                    return Ok(());
                }
                if let Err(error) = prepare_result {
                    show_update_failure(&app, &controller, target, version, &error).await;
                    return Err(error);
                }
            }
            set_update(
                &app,
                &controller,
                Some(UpdateState {
                    target,
                    phase: UpdatePhase::Installing,
                    version: Some(version.clone()),
                    progress: Some(if target == UpdateTarget::Dsh {
                        72.0
                    } else {
                        10.0
                    }),
                    resolved_items: None,
                    reused_items: None,
                    downloaded_items: None,
                    added_items: None,
                    total_items: None,
                    elapsed_seconds: None,
                    message_key: "installingUpdate".to_owned(),
                }),
            )
            .await;
            show_prompt(&app);
            let result = match target {
                UpdateTarget::Controller => install_controller(&app, &controller, &version).await,
                UpdateTarget::Dsh => install_dsh(&app, &controller, &version).await,
            };
            if let Err(error) = result {
                show_update_failure(&app, &controller, target, version, &error).await;
                return Err(error);
            }
        }
    }
    Ok(())
}

async fn prepare_confirmed_dsh(
    app: &AppHandle,
    controller: &Arc<Controller>,
    version: &str,
) -> Result<(), String> {
    ensure_update_not_paused(controller)?;
    let metadata = registry_metadata_for_version(fetch_registry_packument().await?, version)?;
    if let Some(requirement) = metadata
        .engines
        .as_ref()
        .and_then(|engines| engines.node.as_deref())
        && !node_satisfies(requirement)?
    {
        return Err("This DSH release requires a newer private Node.js runtime.".to_owned());
    }
    set_dsh_indeterminate_state(
        app,
        controller,
        version,
        UpdatePhase::Checking,
        "downloadingUpdate",
    )
    .await;
    controller.logs.write(
        app,
        "app",
        format!("Downloading and validating DSH {version} after user confirmation."),
    );
    stage_dsh(app, controller, version).await?;
    let mut settings = controller.settings.write().await;
    settings.dsh.staged_version = Some(version.to_owned());
    settings.save(&controller.paths.settings)
}

async fn show_update_failure(
    app: &AppHandle,
    controller: &Arc<Controller>,
    target: UpdateTarget,
    version: String,
    error: &str,
) {
    controller
        .logs
        .write(app, "app", format!("Update installation failed: {error}"));
    set_update(
        app,
        controller,
        Some(UpdateState {
            target,
            phase: UpdatePhase::Failed,
            version: Some(version),
            progress: None,
            resolved_items: None,
            reused_items: None,
            downloaded_items: None,
            added_items: None,
            total_items: None,
            elapsed_seconds: None,
            message_key: "updateInstallFailed".to_owned(),
        }),
    )
    .await;
    show_prompt(app);
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
        let mut first_check = true;
        loop {
            if !controller.settings.read().await.auto_check_dsh_updates {
                tokio::time::sleep(Duration::from_secs(60)).await;
                continue;
            }
            let result = check_with_mode(app.clone(), controller.clone(), false, first_check).await;
            first_check = false;
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
    notify_available: bool,
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
        || notify_available
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
            resolved_items: None,
            reused_items: None,
            downloaded_items: None,
            added_items: None,
            total_items: None,
            elapsed_seconds: None,
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
struct RegistryEngines {
    node: Option<String>,
}

#[derive(serde::Deserialize)]
struct RegistryMetadata {
    name: String,
    version: String,
    engines: Option<RegistryEngines>,
}

#[derive(serde::Deserialize)]
struct RegistryPackument {
    name: String,
    #[serde(rename = "dist-tags")]
    dist_tags: BTreeMap<String, String>,
    versions: BTreeMap<String, RegistryMetadata>,
}

fn dsh_registry_url() -> String {
    "https://registry.npmjs.org/@deepseek-ai%2Fdsh".to_owned()
}

fn configured_dsh_dist_tags() -> Result<Vec<String>, String> {
    let tags: Vec<String> =
        serde_json::from_str(crate::model::DSH_DIST_TAGS).map_err(|error| error.to_string())?;
    if tags.is_empty() {
        return Err("No npm distribution tags are configured for DSH updates.".to_owned());
    }
    Ok(tags)
}

fn select_registry_candidates(
    mut packument: RegistryPackument,
    tags: &[String],
) -> Result<Vec<RegistryMetadata>, String> {
    if packument.name != "@deepseek-ai/dsh" {
        return Err("The npm registry returned an unexpected package.".to_owned());
    }
    let mut selected = BTreeMap::new();
    for tag in tags {
        let Some(version_text) = packument.dist_tags.get(tag) else {
            continue;
        };
        let metadata = packument
            .versions
            .get(version_text)
            .ok_or_else(|| format!("The npm registry tag {tag} points to missing DSH metadata."))?;
        if metadata.name != packument.name || metadata.version != *version_text {
            return Err(format!(
                "The npm registry returned inconsistent DSH metadata for tag {tag}."
            ));
        }
        let version = semver::Version::parse(version_text).map_err(|error| {
            format!("The npm registry tag {tag} has an invalid DSH version: {error}")
        })?;
        selected.insert(version, version_text.clone());
    }
    if selected.is_empty() {
        return Err("The npm registry returned no configured DSH releases.".to_owned());
    }
    selected
        .into_iter()
        .rev()
        .map(|(_, version)| {
            packument
                .versions
                .remove(&version)
                .ok_or_else(|| "The selected DSH release metadata is missing.".to_owned())
        })
        .collect()
}

#[cfg(test)]
fn select_registry_metadata(
    packument: RegistryPackument,
    tags: &[String],
) -> Result<RegistryMetadata, String> {
    select_registry_candidates(packument, tags)?
        .into_iter()
        .next()
        .ok_or_else(|| "The npm registry returned no configured DSH releases.".to_owned())
}

async fn fetch_registry_packument() -> Result<RegistryPackument, String> {
    reqwest::Client::new()
        .get(dsh_registry_url())
        .header("Accept", "application/vnd.npm.install-v1+json")
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?
        .json()
        .await
        .map_err(|error| error.to_string())
}

fn registry_metadata_for_version(
    mut packument: RegistryPackument,
    version: &str,
) -> Result<RegistryMetadata, String> {
    if packument.name != "@deepseek-ai/dsh" {
        return Err("The npm registry returned an unexpected package.".to_owned());
    }
    let metadata = packument
        .versions
        .remove(version)
        .ok_or_else(|| "The confirmed DSH release is no longer available.".to_owned())?;
    if metadata.name != packument.name || metadata.version != version {
        return Err("The confirmed DSH release metadata is inconsistent.".to_owned());
    }
    Ok(metadata)
}

async fn select_compatible_registry_metadata(
    _app: &AppHandle,
    _controller: &Arc<Controller>,
    packument: RegistryPackument,
) -> Result<RegistryMetadata, String> {
    for metadata in select_registry_candidates(packument, &configured_dsh_dist_tags()?)? {
        let requirement = metadata
            .engines
            .as_ref()
            .and_then(|engines| engines.node.as_deref());
        let compatible = match requirement {
            Some(requirement) => node_satisfies(requirement)?,
            None => true,
        };
        if compatible {
            return Ok(metadata);
        }
    }
    Err("This DSH release requires a newer private Node.js runtime.".to_owned())
}

pub(crate) async fn latest_compatible_dsh_version(
    app: &AppHandle,
    controller: &Arc<Controller>,
) -> Result<String, String> {
    Ok(
        select_compatible_registry_metadata(app, controller, fetch_registry_packument().await?)
            .await?
            .version,
    )
}

async fn check_dsh(
    app: &AppHandle,
    controller: &Arc<Controller>,
    manual: bool,
    notify_available: bool,
) -> Result<bool, String> {
    ensure_update_not_paused(controller)?;
    let packument = fetch_registry_packument().await?;
    ensure_update_not_paused(controller)?;
    let metadata = match select_compatible_registry_metadata(app, controller, packument).await {
        Ok(metadata) => metadata,
        Err(error) if !manual && error.contains("requires a newer private Node.js runtime") => {
            return Ok(false);
        }
        Err(error) => return Err(error),
    };
    let current = semver::Version::parse(&controller.snapshot.read().await.dsh_version)
        .map_err(|error| error.to_string())?;
    let latest = semver::Version::parse(&metadata.version).map_err(|error| error.to_string())?;
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
    let notify = manual
        || notify_available
        || controller
            .settings
            .read()
            .await
            .dsh
            .last_notified_version
            .as_deref()
            != Some(&metadata.version);
    let staged = staged_dsh_is_valid(app, controller, &metadata.version).await?;
    if staged {
        controller.logs.write(
            app,
            "app",
            format!("Reusing validated staged DSH {}.", metadata.version),
        );
    }
    {
        let mut settings = controller.settings.write().await;
        settings.dsh.staged_version = staged.then(|| metadata.version.clone());
        settings.dsh.last_notified_version = Some(metadata.version.clone());
        settings.save(&controller.paths.settings)?;
    }
    set_update(
        app,
        controller,
        Some(UpdateState {
            target: UpdateTarget::Dsh,
            phase: if staged {
                UpdatePhase::Ready
            } else {
                UpdatePhase::Available
            },
            version: Some(metadata.version),
            progress: staged.then_some(100.0),
            resolved_items: None,
            reused_items: None,
            downloaded_items: None,
            added_items: None,
            total_items: None,
            elapsed_seconds: None,
            message_key: if staged {
                "dshUpdate".to_owned()
            } else {
                "dshUpdateAvailable".to_owned()
            },
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
) -> Result<(), String> {
    ensure_update_not_paused(controller)?;
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
        "packageManager": format!("pnpm@{}", crate::model::PNPM_VERSION),
        "dependencies": { "@deepseek-ai/dsh": version }
    });
    fs::write(
        runtime.join("package.json"),
        serde_json::to_vec_pretty(&manifest).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let progress_app = app.clone();
    let progress_controller = controller.clone();
    let progress_version = version.to_owned();
    let progress_task = tokio::spawn(async move {
        while let Some(pnpm) = receiver.recv().await {
            set_dsh_package_state(
                &progress_app,
                &progress_controller,
                &progress_version,
                &pnpm,
            )
            .await;
        }
    });
    let pnpm_result = crate::runtime::run_pnpm(
        app,
        controller,
        &runtime,
        Duration::from_secs(1200),
        true,
        sender,
    )
    .await;
    let _ = progress_task.await;
    if let Err(error) = pnpm_result {
        let _ = fs::remove_dir_all(&stage);
        return Err(error);
    }
    ensure_update_not_paused(controller)?;
    set_dsh_work_state(
        app,
        controller,
        version,
        UpdatePhase::Checking,
        "verifyingUpdate",
        90.0,
    )
    .await;
    let installed = fs::read(runtime.join("node_modules/@deepseek-ai/dsh/package.json"))
        .map_err(|error| error.to_string())
        .and_then(|bytes| {
            serde_json::from_slice::<serde_json::Value>(&bytes).map_err(|error| error.to_string())
        });
    let installed = match installed {
        Ok(installed) => installed,
        Err(error) => {
            let _ = fs::remove_dir_all(&stage);
            return Err(error);
        }
    };
    if installed.get("version").and_then(|value| value.as_str()) != Some(version) {
        let _ = fs::remove_dir_all(stage);
        return Err("The staged DSH package version is invalid.".to_owned());
    }
    Ok(())
}

async fn staged_dsh_is_valid(
    app: &AppHandle,
    controller: &Arc<Controller>,
    version: &str,
) -> Result<bool, String> {
    let stage = controller.paths.staging.join(format!("update-{version}"));
    let candidate = match crate::runtime::resolve_candidate(&controller.paths, &stage, version) {
        Ok(candidate) => candidate,
        Err(_) => return Ok(false),
    };
    if let Err(error) = crate::runtime::validate_runtime(app, controller, &candidate).await {
        controller.logs.write(
            app,
            "app",
            format!("Discarding invalid staged DSH {version}: {error}"),
        );
        fs::remove_dir_all(&stage).map_err(|remove_error| remove_error.to_string())?;
        return Ok(false);
    }
    Ok(true)
}

fn node_satisfies(requirement: &str) -> Result<bool, String> {
    let node = semver::Version::parse(crate::model::NODE_VERSION)
        .map_err(|error| format!("The private Node.js version is invalid: {error}"))?;
    requirement.split("||").try_fold(false, |matched, clause| {
        if matched {
            return Ok(true);
        }
        let normalized = clause.split_whitespace().collect::<Vec<_>>().join(", ");
        let requirement = semver::VersionReq::parse(&normalized)
            .map_err(|error| format!("The DSH Node.js requirement is invalid: {error}"))?;
        Ok(requirement.matches(&node))
    })
}

async fn install_dsh(
    app: &AppHandle,
    controller: &Arc<Controller>,
    version: &str,
) -> Result<(), String> {
    let stage = controller.paths.staging.join(format!("update-{version}"));
    set_dsh_work_state(
        app,
        controller,
        version,
        UpdatePhase::Installing,
        "validatingUpdate",
        90.0,
    )
    .await;
    let staged_runtime = crate::runtime::resolve_candidate(&controller.paths, &stage, version)?;
    crate::runtime::validate_runtime(app, controller, &staged_runtime).await?;
    set_dsh_work_state(
        app,
        controller,
        version,
        UpdatePhase::Installing,
        "restartingUpdate",
        93.0,
    )
    .await;
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
    set_dsh_work_state(
        app,
        controller,
        version,
        UpdatePhase::Installing,
        "testingUpdate",
        95.0,
    )
    .await;
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
    set_dsh_work_state(
        app,
        controller,
        version,
        UpdatePhase::Installing,
        "activatingUpdate",
        99.0,
    )
    .await;
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
    set_dsh_work_state(
        app,
        controller,
        version,
        UpdatePhase::Installing,
        "updateComplete",
        100.0,
    )
    .await;
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

async fn set_dsh_work_state(
    app: &AppHandle,
    controller: &Arc<Controller>,
    version: &str,
    phase: UpdatePhase,
    message_key: &str,
    progress: f64,
) {
    set_update(
        app,
        controller,
        Some(UpdateState {
            target: UpdateTarget::Dsh,
            phase,
            version: Some(version.to_owned()),
            progress: Some(progress),
            resolved_items: None,
            reused_items: None,
            downloaded_items: None,
            added_items: None,
            total_items: None,
            elapsed_seconds: None,
            message_key: message_key.to_owned(),
        }),
    )
    .await;
}

async fn set_dsh_package_state(
    app: &AppHandle,
    controller: &Arc<Controller>,
    version: &str,
    pnpm: &crate::runtime::PnpmProgress,
) {
    let message_key = match pnpm.stage {
        crate::runtime::PnpmStage::Preparing => "preparingPackageManager",
        crate::runtime::PnpmStage::Resolving => "resolvingDependencies",
        crate::runtime::PnpmStage::Importing => "installingPackages",
        crate::runtime::PnpmStage::Lifecycle => "runningInstallScripts",
    };
    set_update(
        app,
        controller,
        Some(UpdateState {
            target: UpdateTarget::Dsh,
            phase: UpdatePhase::Checking,
            version: Some(version.to_owned()),
            progress: Some(pnpm.percent()),
            resolved_items: Some(pnpm.resolved_items),
            reused_items: Some(pnpm.reused_items),
            downloaded_items: Some(pnpm.downloaded_items),
            added_items: Some(pnpm.added_items),
            total_items: pnpm.total_items,
            elapsed_seconds: Some(pnpm.elapsed_seconds),
            message_key: message_key.to_owned(),
        }),
    )
    .await;
}

async fn set_dsh_indeterminate_state(
    app: &AppHandle,
    controller: &Arc<Controller>,
    version: &str,
    phase: UpdatePhase,
    message_key: &str,
) {
    set_update(
        app,
        controller,
        Some(UpdateState {
            target: UpdateTarget::Dsh,
            phase,
            version: Some(version.to_owned()),
            progress: None,
            resolved_items: None,
            reused_items: None,
            downloaded_items: None,
            added_items: None,
            total_items: None,
            elapsed_seconds: None,
            message_key: message_key.to_owned(),
        }),
    )
    .await;
}

pub async fn dismiss_notice(app: &AppHandle, controller: &Arc<Controller>) {
    set_update(app, controller, None).await;
    if let Some(window) = app.get_webview_window("prompt") {
        let _ = window.hide();
    }
}

pub async fn pause(app: &AppHandle, controller: &Arc<Controller>) -> Result<(), String> {
    let update = controller.snapshot.read().await.update.clone();
    if !controller.update_running.load(Ordering::SeqCst)
        || !update.as_ref().is_some_and(is_pauseable_update)
    {
        return Err("No DSH download is currently running.".to_owned());
    }
    controller.update_cancel.store(true, Ordering::SeqCst);
    controller.logs.write(
        app,
        "app",
        "The DSH update download was paused by the user.",
    );
    if let Some(window) = app.get_webview_window("prompt") {
        let _ = window.hide();
    }
    Ok(())
}

fn is_pauseable_update(update: &UpdateState) -> bool {
    update.target == UpdateTarget::Dsh && update.phase == UpdatePhase::Checking
}

fn ensure_update_not_paused(controller: &Arc<Controller>) -> Result<(), String> {
    if controller.update_cancel.load(Ordering::SeqCst) {
        Err(crate::runtime::UPDATE_PAUSED_ERROR.to_owned())
    } else {
        Ok(())
    }
}

fn show_prompt(app: &AppHandle) {
    crate::show_update_prompt(app);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_dsh_download_preparation_is_pauseable() {
        let update = |target, phase| UpdateState {
            target,
            phase,
            version: Some("1.0.0".to_owned()),
            progress: Some(50.0),
            resolved_items: None,
            reused_items: None,
            downloaded_items: None,
            added_items: None,
            total_items: None,
            elapsed_seconds: None,
            message_key: "downloadingUpdate".to_owned(),
        };

        assert!(is_pauseable_update(&update(
            UpdateTarget::Dsh,
            UpdatePhase::Checking
        )));
        assert!(!is_pauseable_update(&update(
            UpdateTarget::Controller,
            UpdatePhase::Checking
        )));
        assert!(!is_pauseable_update(&update(
            UpdateTarget::Dsh,
            UpdatePhase::Installing
        )));
    }

    #[test]
    fn staged_dsh_candidate_can_be_resolved_after_restart() {
        let directory = tempfile::tempdir().unwrap();
        let paths = crate::paths::AppPaths::from_root(directory.path().join("app"));
        fs::create_dir_all(paths.node_executable().parent().unwrap()).unwrap();
        fs::write(paths.node_executable(), b"node").unwrap();
        fs::create_dir_all(paths.corepack_cli().parent().unwrap()).unwrap();
        fs::write(paths.corepack_cli(), b"corepack").unwrap();
        let version = "0.1.0-rc.8";
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
        assert!(
            crate::runtime::resolve_candidate(
                &paths,
                &paths.staging.join(format!("update-{version}")),
                version,
            )
            .is_ok()
        );
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

    fn registry_metadata(version: &str) -> RegistryMetadata {
        RegistryMetadata {
            name: "@deepseek-ai/dsh".to_owned(),
            version: version.to_owned(),
            engines: None,
        }
    }

    fn registry_packument(latest: Option<&str>, next: Option<&str>) -> RegistryPackument {
        let mut dist_tags = BTreeMap::new();
        let mut versions = BTreeMap::new();
        for (tag, version) in [("latest", latest), ("next", next)] {
            if let Some(version) = version {
                dist_tags.insert(tag.to_owned(), version.to_owned());
                versions.insert(version.to_owned(), registry_metadata(version));
            }
        }
        RegistryPackument {
            name: "@deepseek-ai/dsh".to_owned(),
            dist_tags,
            versions,
        }
    }

    fn update_tags() -> Vec<String> {
        vec!["latest".to_owned(), "next".to_owned()]
    }

    #[test]
    fn dsh_update_feed_uses_the_package_packument() {
        assert_eq!(
            dsh_registry_url(),
            "https://registry.npmjs.org/@deepseek-ai%2Fdsh"
        );
        assert_eq!(configured_dsh_dist_tags().unwrap(), update_tags());
    }

    #[test]
    fn dsh_update_selects_the_highest_configured_release() {
        let prerelease = select_registry_metadata(
            registry_packument(Some("0.1.0-rc.7"), Some("0.1.0-rc.9")),
            &update_tags(),
        )
        .unwrap();
        assert_eq!(prerelease.version, "0.1.0-rc.9");

        let stable = select_registry_metadata(
            registry_packument(Some("0.1.0"), Some("0.1.0-rc.9")),
            &update_tags(),
        )
        .unwrap();
        assert_eq!(stable.version, "0.1.0");

        let next_minor = select_registry_metadata(
            registry_packument(Some("0.1.0"), Some("0.2.0-rc.1")),
            &update_tags(),
        )
        .unwrap();
        assert_eq!(next_minor.version, "0.2.0-rc.1");
    }

    #[test]
    fn dsh_update_accepts_one_tag_but_rejects_no_candidates() {
        let metadata =
            select_registry_metadata(registry_packument(Some("0.1.0"), None), &update_tags())
                .unwrap();
        assert_eq!(metadata.version, "0.1.0");

        let error = select_registry_metadata(registry_packument(None, None), &update_tags())
            .err()
            .unwrap();
        assert!(error.contains("no configured DSH releases"));
    }

    #[test]
    fn update_decision_must_match_target_version_and_phase() {
        let update = UpdateState {
            target: UpdateTarget::Dsh,
            phase: UpdatePhase::Ready,
            version: Some("0.1.0-rc.8".to_owned()),
            progress: Some(100.0),
            resolved_items: None,
            reused_items: None,
            downloaded_items: None,
            added_items: None,
            total_items: None,
            elapsed_seconds: None,
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
        let available = UpdateState {
            phase: UpdatePhase::Available,
            progress: None,
            message_key: "dshUpdateAvailable".to_owned(),
            ..update
        };
        assert!(update_request_matches(
            &available,
            UpdateTarget::Dsh,
            "0.1.0-rc.8"
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
