use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, PhysicalPosition, Rect, Runtime,
    WebviewUrl, WebviewWindow, WebviewWindowBuilder,
};

use super::BehaviorTip;

const TIP_WINDOW_LABEL: &str = "behavior-tip";
const TIP_WINDOW_WIDTH: f64 = 320.0;
const TIP_WINDOW_HEIGHT: f64 = 104.0;
const TIP_MARGIN: f64 = 18.0;

/// 简化后的托盘位置缓存
#[derive(Debug, Clone, Copy)]
pub struct TrayRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl TrayRect {
    /// 从 Tauri tray rect 转为可跨线程保存的物理坐标
    pub fn from_tauri_rect(rect: &Rect) -> Self {
        let position = rect.position.to_physical::<f64>(1.0);
        let size = rect.size.to_physical::<f64>(1.0);

        Self {
            x: position.x,
            y: position.y,
            width: size.width,
            height: size.height,
        }
    }
}

/// 显示行为提示窗口并同步当前提示数据
pub fn show_tip_window<R: Runtime>(
    app: &AppHandle<R>,
    tray_rect: Option<TrayRect>,
    tip: &BehaviorTip,
) -> tauri::Result<()> {
    let window = ensure_tip_window(app)?;
    let (x, y) = tip_position(app, tray_rect);

    let _ = window.set_position(LogicalPosition::new(x, y));
    let _ = window.emit("behavior-tip-updated", tip);
    let _ = window.show();

    Ok(())
}

/// 隐藏行为提示窗口
pub fn hide_tip_window<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window(TIP_WINDOW_LABEL) {
        let _ = window.emit("behavior-tip-updated", Option::<BehaviorTip>::None);
        window.hide()?;
    }

    Ok(())
}

fn ensure_tip_window<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<WebviewWindow<R>> {
    if let Some(window) = app.get_webview_window(TIP_WINDOW_LABEL) {
        return Ok(window);
    }

    let builder = WebviewWindowBuilder::new(
        app,
        TIP_WINDOW_LABEL,
        WebviewUrl::App("index.html#/behavior-tip".into()),
    )
    .title("TokenBurger Tip")
    .inner_size(TIP_WINDOW_WIDTH, TIP_WINDOW_HEIGHT)
    .resizable(false)
    .decorations(false)
    .transparent(true)
    .always_on_top(true)
    .skip_taskbar(true)
    .visible(false)
    .focused(false);

    let window = builder.build()?;
    let _ = window.set_visible_on_all_workspaces(true);

    Ok(window)
}

fn tip_position<R: Runtime>(app: &AppHandle<R>, tray_rect: Option<TrayRect>) -> (f64, f64) {
    let (x, y) = if let Some(rect) = tray_rect {
        position_near_tray(app, rect)
    } else {
        fallback_position(app)
    };

    avoid_popup_overlap(app, x, y)
}

fn position_near_tray<R: Runtime>(app: &AppHandle<R>, rect: TrayRect) -> (f64, f64) {
    let scale_factor = monitor_scale_factor(app, rect.x, rect.y);
    let x = rect.x / scale_factor;
    let y = rect.y / scale_factor;
    let width = rect.width / scale_factor;
    let height = rect.height / scale_factor;

    #[cfg(target_os = "windows")]
    {
        (
            x + width - TIP_WINDOW_WIDTH,
            y - TIP_WINDOW_HEIGHT - TIP_MARGIN,
        )
    }

    #[cfg(not(target_os = "windows"))]
    {
        (
            x + (width / 2.0) - (TIP_WINDOW_WIDTH / 2.0),
            y + height + 8.0,
        )
    }
}

fn fallback_position<R: Runtime>(app: &AppHandle<R>) -> (f64, f64) {
    let monitor = app.primary_monitor().ok().flatten();
    let Some(monitor) = monitor else {
        return (TIP_MARGIN, TIP_MARGIN);
    };

    let scale_factor = monitor.scale_factor();
    let position = monitor.position().to_logical::<f64>(scale_factor);
    let size = monitor.size().to_logical::<f64>(scale_factor);
    let x = position.x + size.width - TIP_WINDOW_WIDTH - TIP_MARGIN;

    #[cfg(target_os = "windows")]
    {
        let y = position.y + size.height - TIP_WINDOW_HEIGHT - 54.0;
        (x, y)
    }

    #[cfg(not(target_os = "windows"))]
    {
        let y = position.y + TIP_MARGIN;
        (x, y)
    }
}

fn monitor_scale_factor<R: Runtime>(app: &AppHandle<R>, x: f64, y: f64) -> f64 {
    app.monitor_from_point(x, y)
        .ok()
        .flatten()
        .map(|monitor| monitor.scale_factor())
        .or_else(|| {
            app.primary_monitor()
                .ok()
                .flatten()
                .map(|monitor| monitor.scale_factor())
        })
        .unwrap_or(1.0)
}

fn avoid_popup_overlap<R: Runtime>(app: &AppHandle<R>, x: f64, y: f64) -> (f64, f64) {
    let Some(popup) = app.get_webview_window("popup") else {
        return (x, y);
    };
    if !popup.is_visible().unwrap_or(false) {
        return (x, y);
    }

    let Ok(position) = popup.outer_position() else {
        return (x, y);
    };
    let Ok(size) = popup.outer_size() else {
        return (x, y);
    };
    let scale_factor = monitor_scale_factor(app, position.x as f64, position.y as f64);
    let popup_x = position.x as f64 / scale_factor;
    let popup_y = position.y as f64 / scale_factor;
    let popup_width = size.width as f64 / scale_factor;
    let popup_height = size.height as f64 / scale_factor;

    let overlaps = x < popup_x + popup_width
        && x + TIP_WINDOW_WIDTH > popup_x
        && y < popup_y + popup_height
        && y + TIP_WINDOW_HEIGHT > popup_y;
    if !overlaps {
        return (x, y);
    }

    if y <= popup_y {
        (x, popup_y + popup_height + 8.0)
    } else {
        (x, (popup_y - TIP_WINDOW_HEIGHT - 8.0).max(TIP_MARGIN))
    }
}

#[allow(dead_code)]
fn _logical_size() -> LogicalSize<f64> {
    LogicalSize::new(TIP_WINDOW_WIDTH, TIP_WINDOW_HEIGHT)
}

#[allow(dead_code)]
fn _physical_position(x: f64, y: f64) -> PhysicalPosition<f64> {
    PhysicalPosition::new(x, y)
}
