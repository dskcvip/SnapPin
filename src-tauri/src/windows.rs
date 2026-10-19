//! 窗口管理：悬浮球 / 截图浮层 / 贴图窗口的创建与关闭。

use tauri::webview::WebviewWindowBuilder;
use tauri::{AppHandle, Manager, PhysicalPosition, WebviewUrl};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::config::AppConfig;

pub const BALL_LABEL: &str = "ball";
pub const OVERLAY_PREFIX: &str = "overlay-";
pub const PIN_PREFIX: &str = "pin-";

/// 截图期间是否因采集而隐藏了悬浮球（用于结束后恢复）。
static BALL_HIDDEN_BY_CAPTURE: AtomicBool = AtomicBool::new(false);

/// 创建悬浮球窗口（已存在则跳过）。
pub fn ensure_ball_window(app: &AppHandle, cfg: &AppConfig) -> tauri::Result<()> {
    if app.get_webview_window(BALL_LABEL).is_some() {
        return Ok(());
    }
    let size = cfg.ball_size as f64;
    let window = WebviewWindowBuilder::new(app, BALL_LABEL, WebviewUrl::App("ball.html".into()))
        .title("SnapPin")
        .inner_size(size, size)
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .shadow(false)
        .build()?;

    let inner = window.inner_size().unwrap_or_default();
    let outer = window.outer_size().unwrap_or_default();
    eprintln!(
        "[snappin] ball window built: inner={:?} outer={:?}",
        inner, outer
    );

    // 位置用物理像素精确还原（position() 是逻辑像素，混合 DPI 下会漂移）
    if let Some(p) = cfg.ball_position {
        window.set_position(PhysicalPosition::new(p.x, p.y))?;
        let pos = window.outer_position().unwrap_or_default();
        eprintln!("[snappin] ball positioned to {:?} (outer_position)", pos);
    }
    Ok(())
}

/// 显示/隐藏悬浮球之间切换。
pub fn toggle_ball(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(BALL_LABEL) {
        match w.is_visible() {
            Ok(true) => {
                let _ = w.hide();
            }
            _ => {
                let _ = w.show();
                let _ = w.set_focus();
            }
        }
    }
}

/// 为每台显示器打开一个全尺寸透明浮层。
///
/// 注意：xcap 给出的显示器几何为物理像素，除以 scale_factor 得到逻辑像素。
/// 混合 DPI 多显示器下位置可能有偏差（已知限制，见 ROADMAP）。
pub fn open_overlays(
    app: &AppHandle,
    monitors: &[snappin_core_platform::MonitorShot],
) -> tauri::Result<()> {
    for (i, m) in monitors.iter().enumerate() {
        let label = format!("{}{}", OVERLAY_PREFIX, i);
        if app.get_webview_window(&label).is_some() {
            continue;
        }
        let sf = m.scale_factor as f64;
        let logical_w = (m.width as f64 / sf).max(1.0);
        let logical_h = (m.height as f64 / sf).max(1.0);
        WebviewWindowBuilder::new(app, &label, WebviewUrl::App("overlay.html".into()))
            .title("SnapPin Capture")
            .inner_size(logical_w, logical_h)
            .position(m.x as f64 / sf, m.y as f64 / sf)
            .decorations(false)
            .transparent(true)
            .always_on_top(true)
            .skip_taskbar(true)
            .resizable(false)
            .shadow(false)
            .focused(true)
            .build()?;
    }
    eprintln!("[snappin] opened {} overlay window(s)", monitors.len());

    // 截图期间隐藏悬浮球：避免遮挡选区拖拽，也避免透传点击误触
    if let Some(ball) = app.get_webview_window(BALL_LABEL) {
        if ball.is_visible().unwrap_or(false) {
            let _ = ball.hide();
            BALL_HIDDEN_BY_CAPTURE.store(true, Ordering::SeqCst);
        }
    }
    Ok(())
}

/// 关闭全部截图浮层。
pub fn close_overlays(app: &AppHandle) {
    let labels: Vec<String> = app
        .webview_windows()
        .keys()
        .filter(|l| l.starts_with(OVERLAY_PREFIX))
        .cloned()
        .collect();
    for label in labels {
        if let Some(w) = app.get_webview_window(&label) {
            let _ = w.close();
        }
    }
    // 恢复因采集而隐藏的悬浮球
    if BALL_HIDDEN_BY_CAPTURE.swap(false, Ordering::SeqCst) {
        if let Some(ball) = app.get_webview_window(BALL_LABEL) {
            let _ = ball.show();
        }
    }
}

/// 创建贴图窗口（无边框置顶透明窗）。
///
/// - `label`：窗口标签（同时作为 pins 状态表的 key）
/// - `disp_w/h`：逻辑像素显示尺寸
/// - `x/y`：逻辑像素位置
pub fn create_pin_window(
    app: &AppHandle,
    label: &str,
    disp_w: f64,
    disp_h: f64,
    x: f64,
    y: f64,
) -> tauri::Result<()> {
    WebviewWindowBuilder::new(app, label, WebviewUrl::App("pin.html".into()))
        .title("SnapPin Pin")
        .inner_size(disp_w.max(8.0), disp_h.max(8.0))
        .position(x, y)
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .shadow(false)
        .build()?;
    Ok(())
}

/// 关闭指定贴图窗口。
pub fn close_pin(app: &AppHandle, label: &str) {
    if let Some(w) = app.get_webview_window(label) {
        let _ = w.close();
    }
}

/// 计算主显示器上的居中位置（逻辑像素）。
pub fn place_centered_on_primary(app: &AppHandle, w: f64, h: f64) -> (f64, f64) {
    if let Some(monitor) = app.primary_monitor().ok().flatten() {
        let pos = monitor.position();
        let size = monitor.size();
        let sf = monitor.scale_factor();
        let lw = size.width as f64 / sf;
        let lh = size.height as f64 / sf;
        (
            pos.x as f64 / sf + (lw - w).max(0.0) / 2.0,
            pos.y as f64 / sf + (lh - h).max(0.0) / 2.0,
        )
    } else {
        (120.0, 120.0)
    }
}
