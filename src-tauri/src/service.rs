use std::{net::TcpListener, process::Stdio, sync::Arc, time::Duration};

use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_opener::OpenerExt;
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    sync::{oneshot, watch},
    task::JoinHandle,
};

use crate::{
    Controller,
    job::ProcessJob,
    model::{HARNESS_URL, ServicePhase, ServiceState},
    runtime::{ResolvedRuntime, prepend_private_path, resolve_runtime},
};

const HARNESS_ADDRESS: &str = "127.0.0.1:3080";
const HARNESS_START_MARKER: &str = "dsh web: http://127.0.0.1:3080";
const STARTUP_TIMEOUT: Duration = Duration::from_secs(45);
const STOP_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Default)]
pub struct ServiceControl {
    generation: u64,
    stop: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
}

impl ServiceControl {
    fn is_current(&self, generation: u64) -> bool {
        self.generation == generation
    }
}

pub async fn start(
    app: AppHandle,
    controller: Arc<Controller>,
    open_browser: bool,
    hide_main_when_ready: bool,
) -> Result<(), String> {
    match controller.snapshot.read().await.service_phase {
        ServicePhase::Ready => {
            if open_browser {
                app.opener()
                    .open_url(HARNESS_URL, None::<&str>)
                    .map_err(|error| error.to_string())?;
            }
            return Ok(());
        }
        ServicePhase::Starting | ServicePhase::Stopping => {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
            return Ok(());
        }
        ServicePhase::Stopped | ServicePhase::Failed => {}
    }
    let runtime = resolve_runtime(&controller.paths)?;
    launch_runtime(
        app,
        controller,
        runtime,
        open_browser,
        hide_main_when_ready,
        None,
    )
    .await
}

pub async fn start_candidate_and_wait(
    app: AppHandle,
    controller: Arc<Controller>,
    runtime: ResolvedRuntime,
) -> Result<(), String> {
    match controller.snapshot.read().await.service_phase {
        ServicePhase::Stopped | ServicePhase::Failed => {}
        ServicePhase::Starting | ServicePhase::Ready | ServicePhase::Stopping => {
            return Err("A Harness service is already running.".to_owned());
        }
    }
    let (ready, wait_until_ready) = oneshot::channel();
    launch_runtime(app, controller, runtime, false, false, Some(ready)).await?;
    wait_until_ready
        .await
        .map_err(|_| "Harness readiness was not reported.".to_owned())?
}

async fn launch_runtime(
    app: AppHandle,
    controller: Arc<Controller>,
    runtime: ResolvedRuntime,
    open_browser: bool,
    hide_main_when_ready: bool,
    readiness: Option<oneshot::Sender<Result<(), String>>>,
) -> Result<(), String> {
    if !port_is_available(HARNESS_ADDRESS) {
        emit_phase(&app, &controller, ServicePhase::Failed, "portInUse").await;
        return Err("Port 3080 is already in use.".to_owned());
    }

    let (stop, stop_request) = oneshot::channel();
    let generation;
    {
        let mut control = controller.service.lock().await;
        if control
            .task
            .as_ref()
            .is_some_and(|task| !task.is_finished())
        {
            return Err("A Harness service task is already running.".to_owned());
        }
        control.task.take();
        control.stop.take();
        control.generation = control.generation.wrapping_add(1);
        generation = control.generation;
        control.stop = Some(stop);

        let app_copy = app.clone();
        let controller_copy = controller.clone();
        control.task = Some(tokio::spawn(async move {
            supervise_runtime(
                app_copy,
                controller_copy,
                runtime,
                open_browser,
                hide_main_when_ready,
                readiness,
                stop_request,
                generation,
            )
            .await;
        }));
    }
    set_phase(
        &app,
        &controller,
        generation,
        ServicePhase::Starting,
        "startupTitle",
    )
    .await;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn supervise_runtime(
    app: AppHandle,
    controller: Arc<Controller>,
    runtime: ResolvedRuntime,
    open_browser: bool,
    hide_main_when_ready: bool,
    readiness: Option<oneshot::Sender<Result<(), String>>>,
    mut stop_request: oneshot::Receiver<()>,
    generation: u64,
) {
    let mut readiness = readiness;
    for attempt in 0..=1 {
        if stop_request.try_recv().is_ok() {
            if let Some(sender) = readiness.take() {
                let _ = sender.send(Err("Harness startup was cancelled.".to_owned()));
            }
            return;
        }
        if attempt > 0 {
            if !port_is_available(HARNESS_ADDRESS) {
                report_start_failure(
                    &app,
                    &controller,
                    generation,
                    &mut readiness,
                    "Port 3080 became occupied before Harness could restart.",
                    "portInUse",
                )
                .await;
                return;
            }
            set_phase(
                &app,
                &controller,
                generation,
                ServicePhase::Starting,
                "startupTitle",
            )
            .await;
        }
        controller.logs.write(
            &app,
            "app",
            format!(
                "Starting Harness {} (attempt {}).",
                runtime.version,
                attempt + 1
            ),
        );
        let mut command = tokio::process::Command::new(&runtime.node);
        command
            .arg(&runtime.dsh)
            .args(["web", "--host", "127.0.0.1", "--port", "3080"])
            .current_dir(&runtime.runtime_directory)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        prepend_private_path(&mut command, &controller.paths.node_root);
        #[cfg(windows)]
        {
            command.creation_flags(0x08000000);
        }
        let Ok(mut child) = command.spawn() else {
            controller
                .logs
                .write(&app, "app", "Unable to create the Harness process.");
            continue;
        };
        let job = match ProcessJob::assign(&child) {
            Ok(job) => job,
            Err(error) => {
                controller.logs.write(
                    &app,
                    "app",
                    format!("Unable to contain the Harness process: {error}"),
                );
                let _ = child.kill().await;
                continue;
            }
        };
        let (marker, marker_seen) = watch::channel(false);
        if let Some(stdout) = child.stdout.take() {
            let app_copy = app.clone();
            let controller_copy = controller.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stdout).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if is_harness_start_marker(&line) {
                        let _ = marker.send(true);
                    }
                    controller_copy.logs.write(&app_copy, "stdout", line);
                }
            });
        }
        if let Some(stderr) = child.stderr.take() {
            let app_copy = app.clone();
            let controller_copy = controller.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    controller_copy.logs.write(&app_copy, "stderr", line);
                }
            });
        }

        match wait_until_ready(&mut child, &mut stop_request, marker_seen).await {
            ReadyResult::Stopped => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                drop(job);
                if let Some(sender) = readiness.take() {
                    let _ = sender.send(Err("Harness startup was cancelled.".to_owned()));
                }
                return;
            }
            ReadyResult::Exited => {
                drop(job);
                controller
                    .logs
                    .write(&app, "app", "Harness exited before it became ready.");
            }
            ReadyResult::TimedOut => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                drop(job);
                controller.logs.write(
                    &app,
                    "app",
                    "Harness was still running but did not become ready before the timeout.",
                );
                if attempt == 1 {
                    if let Some(sender) = readiness.take() {
                        let _ = sender.send(Err("Harness startup timed out.".to_owned()));
                    }
                    set_phase(
                        &app,
                        &controller,
                        generation,
                        ServicePhase::Failed,
                        "startupTimedOut",
                    )
                    .await;
                    show_main(&app);
                    return;
                }
            }
            ReadyResult::Ready => {
                set_phase(
                    &app,
                    &controller,
                    generation,
                    ServicePhase::Ready,
                    "complete",
                )
                .await;
                if let Some(sender) = readiness.take() {
                    let _ = sender.send(Ok(()));
                }
                if open_browser {
                    let _ = app.opener().open_url(HARNESS_URL, None::<&str>);
                }
                if hide_main_when_ready && let Some(window) = app.get_webview_window("main") {
                    let _ = crate::tray::ensure(&app, &controller);
                    let _ = window.hide();
                }
                tokio::select! {
                    _ = &mut stop_request => {
                        let _ = child.kill().await;
                        let _ = child.wait().await;
                        drop(job);
                        return;
                    }
                    status = child.wait() => {
                        drop(job);
                        controller.logs.write(&app, "app", format!("Harness exited: {status:?}"));
                    }
                }
            }
        }
        if attempt == 0 {
            tokio::select! {
                _ = &mut stop_request => return,
                _ = tokio::time::sleep(Duration::from_secs(2)) => {}
            }
        }
    }
    report_start_failure(
        &app,
        &controller,
        generation,
        &mut readiness,
        "Harness did not become ready after two attempts.",
        "failed",
    )
    .await;
}

async fn report_start_failure(
    app: &AppHandle,
    controller: &Arc<Controller>,
    generation: u64,
    readiness: &mut Option<oneshot::Sender<Result<(), String>>>,
    error: &str,
    message_key: &str,
) {
    if let Some(sender) = readiness.take() {
        let _ = sender.send(Err(error.to_owned()));
    }
    set_phase(
        app,
        controller,
        generation,
        ServicePhase::Failed,
        message_key,
    )
    .await;
    show_main(app);
}

pub async fn stop(controller: &Arc<Controller>) {
    let (generation, sender, mut task) = {
        let mut control = controller.service.lock().await;
        control.generation = control.generation.wrapping_add(1);
        let generation = control.generation;
        (generation, control.stop.take(), control.task.take())
    };
    if sender.is_none() && task.is_none() {
        return;
    }
    set_snapshot_phase(controller, generation, ServicePhase::Stopping).await;
    if let Some(sender) = sender {
        let _ = sender.send(());
    }
    if let Some(task) = task.as_mut()
        && tokio::time::timeout(STOP_TIMEOUT, &mut *task)
            .await
            .is_err()
    {
        task.abort();
        let _ = task.await;
    }
    set_snapshot_phase(controller, generation, ServicePhase::Stopped).await;
}

enum ReadyResult {
    Ready,
    Stopped,
    Exited,
    TimedOut,
}

async fn wait_until_ready(
    child: &mut tokio::process::Child,
    stop: &mut oneshot::Receiver<()>,
    marker_seen: watch::Receiver<bool>,
) -> ReadyResult {
    let client = reqwest::Client::new();
    let deadline = tokio::time::Instant::now() + STARTUP_TIMEOUT;
    loop {
        if child.try_wait().ok().flatten().is_some() {
            return ReadyResult::Exited;
        }
        if *marker_seen.borrow()
            && let Ok(response) = client
                .get(HARNESS_URL)
                .timeout(Duration::from_millis(900))
                .send()
                .await
            && response.status().is_success()
        {
            return ReadyResult::Ready;
        }
        if tokio::time::Instant::now() >= deadline {
            return ReadyResult::TimedOut;
        }
        tokio::select! {
            _ = &mut *stop => return ReadyResult::Stopped,
            _ = tokio::time::sleep(Duration::from_millis(250)) => {}
        }
    }
}

fn port_is_available(address: &str) -> bool {
    TcpListener::bind(address).is_ok()
}

fn is_harness_start_marker(line: &str) -> bool {
    line.trim() == HARNESS_START_MARKER
}

async fn set_phase(
    app: &AppHandle,
    controller: &Arc<Controller>,
    generation: u64,
    phase: ServicePhase,
    message_key: &str,
) {
    if controller.service.lock().await.generation != generation {
        return;
    }
    emit_phase(app, controller, phase, message_key).await;
}

async fn emit_phase(
    app: &AppHandle,
    controller: &Arc<Controller>,
    phase: ServicePhase,
    message_key: &str,
) {
    controller.snapshot.write().await.service_phase = phase;
    let _ = app.emit(
        "service://state",
        ServiceState {
            phase,
            message_key: message_key.to_owned(),
        },
    );
    let _ = app.emit("ui://refresh", ());
}

async fn set_snapshot_phase(controller: &Arc<Controller>, generation: u64, phase: ServicePhase) {
    if controller.service.lock().await.is_current(generation) {
        controller.snapshot.write().await.service_phase = phase;
    }
}

fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn occupied_ports_are_not_available() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("test listener");
        let address = listener.local_addr().expect("listener address");
        assert!(!port_is_available(&address.to_string()));
    }

    #[test]
    fn readiness_requires_the_owned_process_start_marker() {
        assert!(is_harness_start_marker("dsh web: http://127.0.0.1:3080"));
        assert!(!is_harness_start_marker(
            "another service: http://127.0.0.1:3080"
        ));
    }

    #[test]
    fn stale_generations_cannot_update_service_state() {
        let control = ServiceControl {
            generation: 8,
            ..ServiceControl::default()
        };
        assert!(control.is_current(8));
        assert!(!control.is_current(7));
    }
}
