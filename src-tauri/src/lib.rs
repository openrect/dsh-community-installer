mod atomic_file;
mod job;
mod logging;
mod model;
mod paths;
mod runtime;
mod service;
mod settings;
mod shortcuts;
mod tray;
mod updates;
mod window_shape;

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use logging::LogStore;
use model::{
    AppSnapshot, Edition, Locale, LogLine, SetupPhase, SetupProgress, UpdateDecision, UpdatePhase,
    UpdateTarget,
};
use paths::AppPaths;
use service::ServiceControl;
use settings::Settings;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder, window::Color};
use tokio::sync::{Mutex, RwLock};

pub struct Controller {
    pub paths: AppPaths,
    pub logs: LogStore,
    pub settings: RwLock<Settings>,
    pub snapshot: RwLock<AppSnapshot>,
    pub service: Mutex<ServiceControl>,
    pub maintenance: Mutex<()>,
    pub setup_running: AtomicBool,
    pub update_running: AtomicBool,
    pub updates_scheduled: AtomicBool,
    pub tray_created: AtomicBool,
    pub shutdown: AtomicBool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MainCloseAction {
    ConfirmSetupCancellation,
    HideToTray,
    Exit,
}

fn main_close_action(setup_running: bool, setup_complete: bool) -> MainCloseAction {
    if setup_running {
        MainCloseAction::ConfirmSetupCancellation
    } else if setup_complete {
        MainCloseAction::HideToTray
    } else {
        MainCloseAction::Exit
    }
}

async fn install_and_start(app: &AppHandle, controller: &Arc<Controller>) -> Result<(), String> {
    let candidate = runtime::install(app.clone(), controller.clone()).await?;
    service::start_candidate_and_wait(app.clone(), controller.clone(), candidate.clone()).await?;
    runtime::commit_install(app, controller, &candidate).await?;
    if let Err(error) = shortcuts::publish_desktop(app) {
        controller.logs.write(
            app,
            "app",
            format!("Desktop shortcut could not be published: {error}"),
        );
    }
    updates::schedule(app.clone(), controller.clone());
    Ok(())
}

#[tauri::command]
async fn get_app_state(
    controller: tauri::State<'_, Arc<Controller>>,
) -> Result<AppSnapshot, String> {
    Ok(controller.snapshot.read().await.clone())
}

#[tauri::command]
async fn begin_setup(
    app: AppHandle,
    controller: tauri::State<'_, Arc<Controller>>,
) -> Result<(), String> {
    if controller.setup_running.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    let controller = controller.inner().clone();
    tauri::async_runtime::spawn(async move {
        let _maintenance = controller.maintenance.lock().await;
        let result = install_and_start(&app, &controller).await;
        if result.is_err() {
            service::stop(&controller).await;
            if controller.shutdown.load(Ordering::SeqCst) {
                runtime::discard_install_staging(&controller.paths);
            }
        }
        controller.setup_running.store(false, Ordering::SeqCst);
        match result {
            Ok(()) => {
                controller
                    .logs
                    .write(&app, "app", "Runtime installation completed.");
            }
            Err(error) => {
                controller
                    .logs
                    .write(&app, "app", format!("Runtime installation failed: {error}"));
                let message_key = match error.as_str() {
                    "Port 3080 is already in use." => "portInUse",
                    "Harness startup timed out." => "startupTimedOut",
                    _ => "failed",
                };
                {
                    let mut snapshot = controller.snapshot.write().await;
                    snapshot.setup_phase = SetupPhase::Failed;
                    snapshot.message_key = message_key.to_owned();
                }
                let _ = app.emit(
                    "setup://progress",
                    SetupProgress {
                        phase: SetupPhase::Failed,
                        percent: controller.snapshot.read().await.progress,
                        message_key: message_key.to_owned(),
                        detail: Some(error),
                    },
                );
            }
        }
        drop(_maintenance);
    });
    Ok(())
}

#[tauri::command]
async fn open_harness(
    app: AppHandle,
    controller: tauri::State<'_, Arc<Controller>>,
) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    if controller.snapshot.read().await.service_phase == model::ServicePhase::Ready {
        tray::ensure(&app, controller.inner()).map_err(|error| error.to_string())?;
        app.opener()
            .open_url(model::HARNESS_URL, None::<&str>)
            .map_err(|error| error.to_string())?;
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.hide();
        }
    } else {
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.show();
            let _ = window.set_focus();
        }
        service::start(app, controller.inner().clone(), true, true).await?;
    }
    Ok(())
}

#[tauri::command]
async fn retry_service(
    app: AppHandle,
    controller: tauri::State<'_, Arc<Controller>>,
) -> Result<(), String> {
    service::start(app, controller.inner().clone(), true, true).await
}

#[tauri::command]
async fn open_logs(app: AppHandle) -> Result<(), String> {
    show_logs_window(&app)
}

#[tauri::command]
async fn get_recent_logs(
    controller: tauri::State<'_, Arc<Controller>>,
) -> Result<Vec<LogLine>, String> {
    Ok(controller.logs.recent())
}

#[tauri::command]
async fn set_auto_download(
    app: AppHandle,
    controller: tauri::State<'_, Arc<Controller>>,
    enabled: bool,
) -> Result<(), String> {
    let mut settings = controller.settings.write().await;
    settings.auto_download = enabled;
    settings.save(&controller.paths.settings)?;
    controller.snapshot.write().await.auto_download = enabled;
    let _ = app.emit("ui://refresh", ());
    Ok(())
}

#[tauri::command]
async fn set_locale(
    app: AppHandle,
    controller: tauri::State<'_, Arc<Controller>>,
    locale: Locale,
) -> Result<(), String> {
    let mut settings = controller.settings.write().await;
    settings.locale = locale;
    settings.save(&controller.paths.settings)?;
    controller.snapshot.write().await.locale = locale;
    let _ = app.emit("ui://refresh", ());
    Ok(())
}

#[tauri::command]
async fn check_updates(
    app: AppHandle,
    controller: tauri::State<'_, Arc<Controller>>,
    manual: bool,
) -> Result<(), String> {
    updates::check(app, controller.inner().clone(), manual).await
}

#[tauri::command]
async fn respond_to_update(
    app: AppHandle,
    controller: tauri::State<'_, Arc<Controller>>,
    target: UpdateTarget,
    version: String,
    decision: UpdateDecision,
) -> Result<(), String> {
    updates::respond(app, controller.inner().clone(), target, version, decision).await
}

#[tauri::command]
async fn dismiss_update_notice(
    app: AppHandle,
    controller: tauri::State<'_, Arc<Controller>>,
) -> Result<(), String> {
    updates::dismiss_notice(&app, controller.inner()).await;
    Ok(())
}

#[tauri::command]
async fn show_exit_prompt(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("prompt") {
        let _ = app.emit("ui://exit-prompt", ());
        window.show().map_err(|error| error.to_string())?;
        return window.set_focus().map_err(|error| error.to_string());
    }
    build_prompt_window(&app, "exit")
}

#[tauri::command]
async fn request_main_close(
    app: AppHandle,
    controller: tauri::State<'_, Arc<Controller>>,
) -> Result<(), String> {
    request_main_close_inner(&app, controller.inner()).await
}

async fn request_main_close_inner(
    app: &AppHandle,
    controller: &Arc<Controller>,
) -> Result<(), String> {
    let setup_running = controller.setup_running.load(Ordering::SeqCst);
    let setup_complete = controller.snapshot.read().await.setup_complete;
    match main_close_action(setup_running, setup_complete) {
        MainCloseAction::ConfirmSetupCancellation => {
            if let Some(window) = app.get_webview_window("prompt") {
                window.show().map_err(|error| error.to_string())?;
                window.set_focus().map_err(|error| error.to_string())?;
                let _ = app.emit("ui://cancel-setup", ());
                return Ok(());
            }
            build_prompt_window(app, "cancel-setup")
        }
        MainCloseAction::HideToTray => {
            tray::ensure(app, controller).map_err(|error| error.to_string())?;
            if let Some(window) = app.get_webview_window("main") {
                window.hide().map_err(|error| error.to_string())?;
            }
            Ok(())
        }
        MainCloseAction::Exit => {
            app.exit(0);
            Ok(())
        }
    }
}

#[tauri::command]
async fn cancel_setup_and_exit(
    app: AppHandle,
    controller: tauri::State<'_, Arc<Controller>>,
) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("prompt") {
        let _ = window.hide();
    }
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
    controller.shutdown.store(true, Ordering::SeqCst);
    service::stop(controller.inner()).await;
    app.exit(0);
    Ok(())
}

#[tauri::command]
async fn hide_tray_menu(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("tray") {
        window.hide().map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[tauri::command]
async fn exit_harness(
    app: AppHandle,
    controller: tauri::State<'_, Arc<Controller>>,
) -> Result<(), String> {
    controller.shutdown.store(true, Ordering::SeqCst);
    service::stop(controller.inner()).await;
    if let Some(window) = app.get_webview_window("logs") {
        let _ = window.close();
    }
    app.exit(0);
    Ok(())
}

fn create_controller(app: &tauri::App) -> Result<Arc<Controller>, String> {
    let paths = AppPaths::discover()?;
    paths.ensure().map_err(|error| error.to_string())?;
    let mut settings = Settings::load(&paths.settings);
    let mut settings_changed = settings.controller.staged_version.take().is_some();
    if !runtime::cleanup_staging(&paths, settings.dsh.staged_version.as_deref()) {
        settings_changed |= settings.dsh.staged_version.take().is_some();
    }
    if settings_changed {
        settings.save(&paths.settings)?;
    }
    let edition = app
        .path()
        .resource_dir()
        .ok()
        .map(|path| path.join("runtime-seed.zip").is_file())
        .map(|offline| {
            if offline {
                Edition::Offline
            } else {
                Edition::Online
            }
        })
        .unwrap_or(Edition::Online);
    // Runtime executables are validated after the first window is visible. Startup must
    // never wait indefinitely for a damaged private Node executable.
    let resolved = runtime::resolve_or_recover_runtime(&paths).ok().flatten();
    let setup_complete = resolved.is_some();
    let mut snapshot = AppSnapshot::new(
        settings.locale,
        edition,
        settings.auto_download,
        setup_complete,
    );
    if let Some(runtime) = resolved {
        snapshot.dsh_version = runtime.version;
    }
    if let Some(version) = settings.dsh.staged_version.as_ref()
        && paths
            .staging
            .join(format!("update-{version}/runtime"))
            .is_dir()
    {
        snapshot.update = Some(model::UpdateState {
            target: UpdateTarget::Dsh,
            phase: UpdatePhase::Ready,
            version: Some(version.clone()),
            progress: Some(100.0),
            message_key: "dshUpdate".to_owned(),
        });
    }
    let logs = LogStore::open(&paths.logs)?;
    Ok(Arc::new(Controller {
        paths,
        logs,
        settings: RwLock::new(settings),
        snapshot: RwLock::new(snapshot),
        service: Mutex::new(ServiceControl::default()),
        maintenance: Mutex::new(()),
        setup_running: AtomicBool::new(false),
        update_running: AtomicBool::new(false),
        updates_scheduled: AtomicBool::new(false),
        tray_created: AtomicBool::new(false),
        shutdown: AtomicBool::new(false),
    }))
}

fn build_main_window(app: &AppHandle, controller: Arc<Controller>) -> tauri::Result<()> {
    let window = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
        .title("DSH Community Installer")
        .inner_size(880.0, 580.0)
        .min_inner_size(720.0, 480.0)
        .center()
        .background_color(Color(0, 0, 0, 0))
        .transparent(true)
        .decorations(false)
        .resizable(false)
        .visible(false)
        .build()?;
    window_shape::attach(&window);
    let app = app.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event
            && !controller.shutdown.load(Ordering::SeqCst)
        {
            api.prevent_close();
            let app = app.clone();
            let controller = controller.clone();
            tauri::async_runtime::spawn(async move {
                let _ = request_main_close_inner(&app, &controller).await;
            });
        }
    });
    Ok(())
}

async fn start_existing_install(app: AppHandle, controller: Arc<Controller>) {
    if !runtime::private_node_is_valid(&app, &controller).await {
        controller.logs.write(
            &app,
            "app",
            "The private Node runtime is invalid and will be reinstalled.",
        );
        {
            let mut snapshot = controller.snapshot.write().await;
            snapshot.setup_complete = false;
            snapshot.setup_phase = SetupPhase::NotInstalled;
            snapshot.service_phase = model::ServicePhase::Stopped;
            snapshot.message_key = "installTitle".to_owned();
            snapshot.progress = 0.0;
        }
        let _ = app.emit("ui://refresh", ());
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.show();
            let _ = window.set_focus();
        }
        return;
    }
    if let Err(error) = shortcuts::publish_desktop(&app) {
        controller.logs.write(
            &app,
            "app",
            format!("Desktop shortcut could not be published: {error}"),
        );
    }
    updates::schedule(app.clone(), controller.clone());
    let _ = service::start(app, controller, true, true).await;
}

async fn run_setup_smoke(app: AppHandle, controller: Arc<Controller>) {
    controller.setup_running.store(true, Ordering::SeqCst);
    let _maintenance = controller.maintenance.lock().await;
    let result = install_and_start(&app, &controller).await;
    controller.setup_running.store(false, Ordering::SeqCst);
    controller.shutdown.store(true, Ordering::SeqCst);
    service::stop(&controller).await;
    if let Err(error) = &result {
        controller
            .logs
            .write(&app, "app", format!("Setup smoke test failed: {error}"));
        runtime::discard_install_staging(&controller.paths);
    } else {
        controller
            .logs
            .write(&app, "app", "Setup smoke test completed.");
    }
    drop(_maintenance);
    app.exit(if result.is_ok() { 0 } else { 1 });
}

fn build_tray_window(app: &tauri::App) -> tauri::Result<()> {
    let window =
        WebviewWindowBuilder::new(app, "tray", WebviewUrl::App("index.html?view=tray".into()))
            .title("Harness")
            .inner_size(252.0, 304.0)
            .background_color(Color(0, 0, 0, 0))
            .transparent(true)
            .decorations(false)
            .resizable(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .visible(false)
            .build()?;
    window_shape::attach(&window);
    Ok(())
}

fn show_logs_window(app: &AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("logs") {
        window.show().map_err(|error| error.to_string())?;
        return window.set_focus().map_err(|error| error.to_string());
    }
    let window =
        WebviewWindowBuilder::new(app, "logs", WebviewUrl::App("index.html?view=logs".into()))
            .title("Harness Logs")
            .inner_size(780.0, 480.0)
            .min_inner_size(620.0, 360.0)
            .background_color(Color(0, 0, 0, 0))
            .transparent(true)
            .decorations(false)
            .visible(false)
            .build()
            .map_err(|error| error.to_string())?;
    window_shape::attach(&window);
    Ok(())
}

fn build_prompt_window(app: &AppHandle, prompt: &str) -> Result<(), String> {
    let window = WebviewWindowBuilder::new(
        app,
        "prompt",
        WebviewUrl::App(format!("index.html?view=prompt&prompt={prompt}").into()),
    )
    .title("Harness")
    .inner_size(580.0, 380.0)
    .center()
    .background_color(Color(0, 0, 0, 0))
    .transparent(true)
    .decorations(false)
    .resizable(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .visible(false)
    .build()
    .map_err(|error| error.to_string())?;
    window_shape::attach(&window);
    let prompt_window = window.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = prompt_window.hide();
        }
    });
    Ok(())
}

pub(crate) fn show_update_prompt(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("prompt") {
        let _ = window.show();
        let _ = window.set_focus();
    } else {
        let _ = build_prompt_window(app, "update");
    }
}

pub fn run() {
    tauri::Builder::default()
        .on_page_load(|webview, payload| {
            if matches!(webview.label(), "main" | "logs")
                && payload.event() == tauri::webview::PageLoadEvent::Finished
                && !std::env::args().any(|argument| argument == "--shutdown-for-maintenance")
            {
                let _ = webview.window().show();
                let _ = webview.window().set_focus();
            }
        })
        .plugin(tauri_plugin_single_instance::init(
            |app, arguments, _directory| {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    let controller = app.state::<Arc<Controller>>().inner().clone();
                    if arguments
                        .iter()
                        .any(|argument| argument == "--shutdown-for-maintenance")
                    {
                        controller.shutdown.store(true, Ordering::SeqCst);
                        service::stop(&controller).await;
                        app.exit(0);
                    } else {
                        use tauri_plugin_opener::OpenerExt;
                        let snapshot = controller.snapshot.read().await.clone();
                        if controller.setup_running.load(Ordering::SeqCst)
                            || !snapshot.setup_complete
                        {
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.unminimize();
                                let _ = window.set_focus();
                            }
                        } else if snapshot.service_phase == model::ServicePhase::Ready {
                            let _ = app.opener().open_url(model::HARNESS_URL, None::<&str>);
                        } else {
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                            let _ = service::start(app, controller, true, true).await;
                        }
                    }
                });
            },
        ))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            get_app_state,
            begin_setup,
            open_harness,
            retry_service,
            open_logs,
            get_recent_logs,
            check_updates,
            set_auto_download,
            set_locale,
            respond_to_update,
            dismiss_update_notice,
            show_exit_prompt,
            request_main_close,
            cancel_setup_and_exit,
            hide_tray_menu,
            exit_harness,
        ])
        .setup(|app| {
            let controller = create_controller(app).map_err(std::io::Error::other)?;
            app.manage(controller.clone());
            if std::env::args().any(|argument| argument == "--shutdown-for-maintenance") {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    controller.shutdown.store(true, Ordering::SeqCst);
                    service::stop(&controller).await;
                    handle.exit(0);
                });
                return Ok(());
            }
            build_main_window(app.handle(), controller.clone())?;
            build_tray_window(app)?;
            let initial_prompt = if controller.snapshot.blocking_read().setup_complete {
                "exit"
            } else {
                "cancel-setup"
            };
            build_prompt_window(app.handle(), initial_prompt)?;
            controller.logs.write(
                app.handle(),
                "app",
                format!("DSH Community Installer {} started.", model::APP_VERSION),
            );
            if std::env::args().any(|argument| argument == "--smoke-setup") {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    run_setup_smoke(handle, controller).await;
                });
                return Ok(());
            }
            if controller.snapshot.blocking_read().setup_complete {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    start_existing_install(handle, controller).await;
                });
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running DSH Community Installer");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn main_close_behavior_tracks_installation_lifecycle() {
        assert_eq!(
            main_close_action(true, false),
            MainCloseAction::ConfirmSetupCancellation
        );
        assert_eq!(main_close_action(false, true), MainCloseAction::HideToTray);
        assert_eq!(main_close_action(false, false), MainCloseAction::Exit);
    }
}
