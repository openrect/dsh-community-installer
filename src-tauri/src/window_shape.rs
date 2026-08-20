const CORNER_RADIUS_LOGICAL_PX: f64 = 15.0;

pub fn attach(window: &tauri::WebviewWindow) {
    apply(window);
    let event_window = window.clone();
    window.on_window_event(move |event| {
        if matches!(
            event,
            tauri::WindowEvent::Resized(_) | tauri::WindowEvent::ScaleFactorChanged { .. }
        ) {
            apply(&event_window);
        }
    });
}

#[cfg(windows)]
fn apply(window: &tauri::WebviewWindow) {
    use windows_sys::Win32::{
        Foundation::{POINT, RECT},
        Graphics::Gdi::{ClientToScreen, CreateRoundRectRgn, DeleteObject, SetWindowRgn},
        UI::WindowsAndMessaging::{GetClientRect, GetWindowRect},
    };

    let Ok(hwnd) = window.hwnd() else {
        return;
    };
    let scale_factor = window.scale_factor().unwrap_or(1.0);
    let diameter = (CORNER_RADIUS_LOGICAL_PX * scale_factor * 2.0).round() as i32;
    unsafe {
        let mut window_rect = RECT::default();
        let mut client_rect = RECT::default();
        let mut client_origin = POINT { x: 0, y: 0 };
        if GetWindowRect(hwnd.0, &mut window_rect) == 0
            || GetClientRect(hwnd.0, &mut client_rect) == 0
            || ClientToScreen(hwnd.0, &mut client_origin) == 0
        {
            return;
        }

        let (left, top, right, bottom) = client_region_bounds(
            window_rect.left,
            window_rect.top,
            client_origin.x,
            client_origin.y,
            client_rect.right.saturating_sub(client_rect.left),
            client_rect.bottom.saturating_sub(client_rect.top),
        );
        // Borderless HWNDs retain an invisible non-client frame. SetWindowRgn
        // uses whole-window coordinates, so the region must begin at the actual
        // client origin to exclude that frame. Windows owns a successful region.
        let region = CreateRoundRectRgn(left, top, right, bottom, diameter, diameter);
        if !region.is_null() && SetWindowRgn(hwnd.0, region, 1) == 0 {
            let _ = DeleteObject(region);
        }
    }
}

#[cfg(not(windows))]
fn apply(_window: &tauri::WebviewWindow) {}

fn client_region_bounds(
    window_left: i32,
    window_top: i32,
    client_screen_x: i32,
    client_screen_y: i32,
    client_width: i32,
    client_height: i32,
) -> (i32, i32, i32, i32) {
    let left = client_screen_x.saturating_sub(window_left);
    let top = client_screen_y.saturating_sub(window_top);
    (
        left,
        top,
        left.saturating_add(client_width).saturating_add(1),
        top.saturating_add(client_height).saturating_add(1),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_radius_tracks_display_scale() {
        assert_eq!((CORNER_RADIUS_LOGICAL_PX * 1.0 * 2.0).round() as i32, 30);
        assert_eq!((CORNER_RADIUS_LOGICAL_PX * 1.5 * 2.0).round() as i32, 45);
    }

    #[test]
    fn native_region_excludes_the_hidden_non_client_frame() {
        assert_eq!(
            client_region_bounds(100, 200, 108, 208, 880, 580),
            (8, 8, 889, 589)
        );
    }
}
