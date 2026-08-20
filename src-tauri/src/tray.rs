use std::sync::{Arc, atomic::Ordering};

use tauri::{
    AppHandle, Manager, PhysicalPosition,
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

pub fn ensure(app: &AppHandle, controller: &Arc<crate::Controller>) -> tauri::Result<()> {
    if controller.tray_created.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    let result = build(app);
    if result.is_err() {
        controller.tray_created.store(false, Ordering::SeqCst);
    }
    result
}

fn build(app: &AppHandle) -> tauri::Result<()> {
    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or_else(|| tauri::Error::AssetNotFound("default window icon".to_owned()))?;
    TrayIconBuilder::with_id("harness")
        .icon(icon)
        .tooltip("Harness")
        .on_tray_icon_event(|tray, event| match event {
            TrayIconEvent::DoubleClick {
                button: MouseButton::Left,
                ..
            } => {
                let app = tray.app_handle().clone();
                tauri::async_runtime::spawn(async move {
                    let controller = app
                        .state::<std::sync::Arc<crate::Controller>>()
                        .inner()
                        .clone();
                    if controller.snapshot.read().await.service_phase
                        == crate::model::ServicePhase::Ready
                    {
                        use tauri_plugin_opener::OpenerExt;
                        let _ = app
                            .opener()
                            .open_url(crate::model::HARNESS_URL, None::<&str>);
                    } else {
                        let _ = crate::service::start(app, controller, true, true).await;
                    }
                });
            }
            TrayIconEvent::Click {
                button: MouseButton::Right,
                button_state: MouseButtonState::Up,
                position,
                ..
            } => {
                if let Some(window) = tray.app_handle().get_webview_window("tray") {
                    let scale = window.scale_factor().unwrap_or(1.0);
                    let size = window.outer_size().ok();
                    let width = size
                        .map(|value| value.width as f64 / scale)
                        .unwrap_or(252.0);
                    let height = size
                        .map(|value| value.height as f64 / scale)
                        .unwrap_or(304.0);
                    let x = (position.x / scale - width).max(0.0);
                    let y = (position.y / scale - height).max(0.0);
                    let _ = window.set_position(PhysicalPosition::new(
                        (x * scale) as i32,
                        (y * scale) as i32,
                    ));
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            _ => {}
        })
        .build(app)?;
    if let Some(window) = app.get_webview_window("tray") {
        let window_to_hide = window.clone();
        window.on_window_event(move |event| {
            if matches!(event, tauri::WindowEvent::Focused(false)) {
                let _ = window_to_hide.hide();
            }
        });
    }
    Ok(())
}
