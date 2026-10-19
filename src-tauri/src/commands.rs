//! IPC 命令层：前端的唯一入口。契约见 docs/CONTRACTS.md。

use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;
use tauri::{AppHandle, Manager, State, Window};

use crate::config::AppConfig;
use crate::state::{AppState, PinMeta};
use crate::windows;
use snappin_core_algo::Rect as AlgoRect;

// ---------- 载荷类型（响应字段统一 camelCase，与前端 TS 契约一致） ----------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorInfoPayload {
    pub id: String,
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale_factor: f32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShotAssetPayload {
    pub monitor_id: String,
    pub path: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapturePayload {
    pub monitors: Vec<MonitorInfoPayload>,
    pub shots: Vec<ShotAssetPayload>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayContextPayload {
    pub monitor_id: String,
    pub image_path: String,
    pub natural_width: u32,
    pub natural_height: u32,
    pub scale_factor: f32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmPayload {
    pub path: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PinContextPayload {
    pub label: String,
    pub src: String,
    pub width: f64,
    pub height: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfoPayload {
    pub version: String,
    pub platform: String,
    pub arch: String,
}

/// 选区参数（前端 overlay 窗口的逻辑像素坐标）。
#[derive(Deserialize)]
pub struct RectPayload {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

// ---------- 截图主流程（命令 / 快捷键 / 托盘共用） ----------

pub fn do_capture(app: &AppHandle, state: &AppState) -> Result<CapturePayload, String> {
    let shots = snappin_core_platform::capture_all_monitors().map_err(|e| e.to_string())?;
    {
        let mut guard = state.monitors.lock().unwrap();
        *guard = shots.clone();
    }
    windows::open_overlays(app, &shots).map_err(|e| e.to_string())?;
    Ok(build_capture_payload(&shots))
}

#[tauri::command]
pub async fn start_capture(app: AppHandle) -> Result<CapturePayload, String> {
    // Windows 已知问题：同步命令/主线程内创建 webview 会死锁或导致 webview 空白，
    // 必须放到阻塞线程执行（见 WebviewWindowBuilder 文档的 Known issues）。
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app2.state::<AppState>();
        do_capture(&app2, &state)
    })
    .await
    .map_err(|e| format!("capture task failed: {e}"))?
}

#[tauri::command]
pub fn overlay_context(
    window: Window,
    state: State<'_, AppState>,
) -> Result<OverlayContextPayload, String> {
    let idx: usize = window
        .label()
        .strip_prefix(windows::OVERLAY_PREFIX)
        .ok_or_else(|| "not an overlay window".to_string())?
        .parse()
        .map_err(|_| "invalid overlay window label".to_string())?;
    let monitors = state.monitors.lock().unwrap();
    let m = monitors
        .get(idx)
        .ok_or_else(|| "no active capture for this overlay".to_string())?;
    Ok(OverlayContextPayload {
        monitor_id: m.id.clone(),
        image_path: m.path.to_string_lossy().to_string(),
        natural_width: m.width,
        natural_height: m.height,
        scale_factor: m.scale_factor,
    })
}

#[tauri::command]
pub async fn confirm_selection(
    app: AppHandle,
    state: State<'_, AppState>,
    monitor_id: String,
    rect: RectPayload,
    action: String,
) -> Result<ConfirmPayload, String> {
    // await 之前取好配置，避免跨 await 持有 State
    let save_dir = { state.config.lock().unwrap().save_dir.clone() };

    let (cropped, scale_factor, monitor_x, monitor_y) = {
        let monitors = state.monitors.lock().unwrap();
        let m = monitors
            .iter()
            .find(|m| m.id == monitor_id)
            .ok_or_else(|| "no active capture session".to_string())?;
        let sf = m.scale_factor as f64;
        // 前端选区为逻辑像素，换算成物理像素后再裁剪
        let algo_rect = AlgoRect {
            x: (rect.x as f64 * sf).round() as i32,
            y: (rect.y as f64 * sf).round() as i32,
            w: (rect.w as f64 * sf).round().max(1.0) as u32,
            h: (rect.h as f64 * sf).round().max(1.0) as u32,
        };
        let cropped = m.image.crop(&algo_rect).map_err(|e| e.to_string())?;
        (cropped, sf, m.x, m.y)
    };

    let path = snappin_core_platform::new_temp_png_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    snappin_core_platform::save_png(&cropped, &path).map_err(|e| e.to_string())?;
    let path_str = path.to_string_lossy().to_string();

    match action.as_str() {
        "copy" => {
            snappin_core_platform::clipboard_copy_image(&cropped).map_err(|e| e.to_string())?;
        }
        "pin" => {
            let label = pin_label(&path);
            let disp_w = (cropped.width as f64 / scale_factor).max(8.0);
            let disp_h = (cropped.height as f64 / scale_factor).max(8.0);
            let x = monitor_x as f64 / scale_factor + rect.x as f64;
            let y = monitor_y as f64 / scale_factor + rect.y as f64;
            // 创建 webview 窗口放到阻塞线程（Windows 主线程同步创建会死锁/空白）
            let app2 = app.clone();
            let label2 = label.clone();
            tauri::async_runtime::spawn_blocking(move || {
                windows::create_pin_window(&app2, &label2, disp_w, disp_h, x, y)
            })
            .await
            .map_err(|e| format!("pin window task failed: {e}"))?
            .map_err(|e| e.to_string())?;
            register_pin(&state, &label, &path_str, disp_w, disp_h);
        }
        "save" => {
            if let Some(dest) = save_dialog(&app, "snappin.png", save_dir).await {
                std::fs::copy(&path, &dest).map_err(|e| e.to_string())?;
            }
        }
        other => return Err(format!("unknown action: {}", other)),
    }

    windows::close_overlays(&app);
    Ok(ConfirmPayload { path: path_str })
}

#[tauri::command]
pub fn cancel_capture(app: AppHandle) -> Result<(), String> {
    windows::close_overlays(&app);
    Ok(())
}

// ---------- 贴图 ----------

/// 剪贴板贴图的实际逻辑（供命令与托盘/快捷键路径共用）。
/// 内部会创建贴图 webview 窗口，调用方须确保不在主线程同步执行。
pub fn do_pin_from_clipboard(app: &AppHandle, state: &AppState) -> Result<String, String> {
    let buffer = snappin_core_platform::clipboard_get_image()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "剪贴板中没有图片".to_string())?;
    let path = snappin_core_platform::new_temp_png_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    snappin_core_platform::save_png(&buffer, &path).map_err(|e| e.to_string())?;
    let path_str = path.to_string_lossy().to_string();
    let label = pin_label(&path);

    let sf = app
        .primary_monitor()
        .ok()
        .flatten()
        .map(|m| m.scale_factor())
        .unwrap_or(1.0);
    let disp_w = (buffer.width as f64 / sf).max(8.0);
    let disp_h = (buffer.height as f64 / sf).max(8.0);
    let (x, y) = windows::place_centered_on_primary(app, disp_w, disp_h);
    windows::create_pin_window(app, &label, disp_w, disp_h, x, y).map_err(|e| e.to_string())?;
    register_pin(state, &label, &path_str, disp_w, disp_h);
    Ok(label)
}

#[tauri::command]
pub async fn pin_from_clipboard(app: AppHandle) -> Result<String, String> {
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app2.state::<AppState>();
        do_pin_from_clipboard(&app2, &state)
    })
    .await
    .map_err(|e| format!("pin task failed: {e}"))?
}

/// 贴图窗口取回自己的上下文（按窗口标签查状态表）。
#[tauri::command]
pub fn pin_context(
    window: Window,
    state: State<'_, AppState>,
) -> Result<PinContextPayload, String> {
    let label = window.label().to_string();
    let pins = state.pins.lock().unwrap();
    let meta = pins
        .get(&label)
        .ok_or_else(|| "pin metadata not found".to_string())?;
    Ok(PinContextPayload {
        label,
        src: meta.src.clone(),
        width: meta.width,
        height: meta.height,
    })
}

#[tauri::command]
pub fn pin_close(app: AppHandle, state: State<'_, AppState>, label: String) -> Result<(), String> {
    windows::close_pin(&app, &label);
    state.pins.lock().unwrap().remove(&label);
    Ok(())
}

#[tauri::command]
pub fn pin_copy(app: AppHandle, state: State<'_, AppState>, label: String) -> Result<(), String> {
    let src = pin_src(&state, &label)?;
    let buffer = snappin_core_platform::load_rgba(Path::new(&src)).map_err(|e| e.to_string())?;
    snappin_core_platform::clipboard_copy_image(&buffer).map_err(|e| e.to_string())?;
    // 关闭路径上的窗口不存在也没关系
    let _ = app;
    Ok(())
}

#[tauri::command]
pub async fn pin_save(
    app: AppHandle,
    state: State<'_, AppState>,
    label: String,
) -> Result<bool, String> {
    let save_dir = { state.config.lock().unwrap().save_dir.clone() };
    let src = pin_src(&state, &label)?;
    if let Some(dest) = save_dialog(&app, "snappin.png", save_dir).await {
        std::fs::copy(&src, &dest).map_err(|e| e.to_string())?;
        Ok(true)
    } else {
        Ok(false)
    }
}

// ---------- 悬浮球 / 窗口 ----------

#[tauri::command]
pub async fn ball_command(app: AppHandle, cmd: String) -> Result<(), String> {
    match cmd.as_str() {
        "capture" => {
            let app2 = app.clone();
            tauri::async_runtime::spawn_blocking(move || {
                let state = app2.state::<AppState>();
                do_capture(&app2, &state)
            })
            .await
            .map_err(|e| format!("capture task failed: {e}"))?
            .map_err(|e| e)?;
        }
        "pin-clipboard" => {
            let app2 = app.clone();
            let r = tauri::async_runtime::spawn_blocking(move || {
                let state = app2.state::<AppState>();
                do_pin_from_clipboard(&app2, &state)
            })
            .await
            .map_err(|e| format!("pin task failed: {e}"))?;
            r.map(|_| ())?;
        }
        "hide" => {
            if let Some(w) = app.get_webview_window(windows::BALL_LABEL) {
                w.hide().map_err(|e| e.to_string())?;
            }
        }
        "quit" => app.exit(0),
        other => return Err(format!("unknown ball command: {}", other)),
    }
    Ok(())
}

/// 对调用方窗口启动 OS 原生拖拽循环。
#[tauri::command]
pub fn drag_window(window: Window) -> Result<(), String> {
    window.start_dragging().map_err(|e| e.to_string())
}

// ---------- 配置 / 信息 ----------

#[tauri::command]
pub fn get_config(state: State<'_, AppState>) -> Result<AppConfig, String> {
    Ok(state.config.lock().unwrap().clone())
}

#[tauri::command]
pub fn update_config(
    app: AppHandle,
    state: State<'_, AppState>,
    config: AppConfig,
) -> Result<AppConfig, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    crate::config::save(&dir, &config).map_err(|e| e.to_string())?;
    {
        let mut guard = state.config.lock().unwrap();
        *guard = config.clone();
    }
    // 悬浮球尺寸变化立即生效
    if let Some(w) = app.get_webview_window(windows::BALL_LABEL) {
        let _ = w.set_size(tauri::LogicalSize::new(
            config.ball_size as f64,
            config.ball_size as f64,
        ));
    }
    Ok(config)
}

#[tauri::command]
pub fn app_info(app: AppHandle) -> Result<AppInfoPayload, String> {
    Ok(AppInfoPayload {
        version: app.package_info().version.to_string(),
        platform: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
    })
}

// ---------- 内部工具 ----------

fn build_capture_payload(shots: &[snappin_core_platform::MonitorShot]) -> CapturePayload {
    CapturePayload {
        monitors: shots
            .iter()
            .map(|m| MonitorInfoPayload {
                id: m.id.clone(),
                name: m.name.clone(),
                x: m.x,
                y: m.y,
                width: m.width,
                height: m.height,
                scale_factor: m.scale_factor,
            })
            .collect(),
        shots: shots
            .iter()
            .map(|m| ShotAssetPayload {
                monitor_id: m.id.clone(),
                path: m.path.to_string_lossy().to_string(),
            })
            .collect(),
    }
}

/// 由临时文件路径生成贴图窗口标签：pin-snap-<uuid8>。
fn pin_label(path: &Path) -> String {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("snap")
        .to_string();
    format!("{}{}", windows::PIN_PREFIX, stem)
}

/// 登记贴图元数据。
fn register_pin(state: &AppState, label: &str, src: &str, width: f64, height: f64) {
    state.pins.lock().unwrap().insert(
        label.to_string(),
        PinMeta {
            src: src.to_string(),
            width,
            height,
        },
    );
}

/// 查询贴图元数据中的图片路径。
fn pin_src(state: &AppState, label: &str) -> Result<String, String> {
    let pins = state.pins.lock().unwrap();
    pins.get(label)
        .map(|m| m.src.clone())
        .ok_or_else(|| "pin window not found".to_string())
}

/// 弹出保存对话框并等待用户选择（取消返回 None）。
async fn save_dialog(
    app: &AppHandle,
    default_name: &str,
    save_dir: Option<String>,
) -> Option<std::path::PathBuf> {
    use tauri_plugin_dialog::DialogExt;

    let (tx, rx) = mpsc::channel::<Option<std::path::PathBuf>>();
    let mut builder = app
        .dialog()
        .file()
        .set_file_name(default_name)
        .add_filter("PNG 图片", &["png"])
        .add_filter("JPEG 图片", &["jpg", "jpeg"]);
    if let Some(dir) = save_dir {
        builder = builder.set_directory(dir);
    }
    builder.save_file(move |picked| {
        let path = match picked {
            Some(tauri_plugin_dialog::FilePath::Path(p)) => Some(p),
            _ => None,
        };
        let _ = tx.send(path);
    });

    // 对话框回调走主线程事件循环；async 命令让出线程后回调才能送达
    tauri::async_runtime::spawn_blocking(move || {
        rx.recv_timeout(Duration::from_secs(600)).ok().flatten()
    })
    .await
    .ok()
    .flatten()
}
