//! IPC 命令层：前端的唯一入口。契约见 docs/CONTRACTS.md。

use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
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
    /// false = 采集中（前端应轮询等待）；true = 冻结图就绪
    pub ready: bool,
    /// 本会话代号：取消/确认时回传，用于丢弃上一轮迟到的请求
    pub session_id: u64,
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

/// 标注图形载荷（前端 overlay 的标注层坐标：逻辑像素，原点=裁剪图左上角）。
///
/// 用 `#[serde(tag = "type")]` 内部标签判别，前端发
/// `{ type: "rect", x, y, w, h, color: [r,g,b,a], width }` 形态的对象。
/// 到 Rust 后由 `to_shapes` 统一乘 DPI 缩放因子转成物理像素。
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ShapePayload {
    Rect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        color: [u8; 4],
        width: f32,
    },
    Ellipse {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        color: [u8; 4],
        width: f32,
    },
    Arrow {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        color: [u8; 4],
        width: f32,
    },
    Pen {
        points: Vec<[f32; 2]>,
        color: [u8; 4],
        width: f32,
    },
    Highlighter {
        points: Vec<[f32; 2]>,
        color: [u8; 4],
        width: f32,
    },
    Mosaic {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        block: u32,
    },
    /// UI 层光栅化的位图（文字 / 序列号等需要字体的标注）。
    ///
    /// `png`：base64 编码的 PNG，**无 `data:` 前缀**，尺寸已是物理像素
    /// （前端按 DPR 光栅化）。用 PNG 而非裸 RGBA，是因为同一段文字
    /// 裸 RGBA 有上百 KB、PNG 只有几 KB，IPC 体积差两个数量级。
    Bitmap {
        x: f32,
        y: f32,
        png: String,
    },
    /// 聚光灯：区域外压暗。`dim` 为 0-255 的压暗强度，
    /// `radius` 为圆角半径（逻辑像素）。
    Spotlight {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radius: f32,
        dim: u8,
    },
}

impl ShapePayload {
    /// 转 core-algo 的纯值类型（逻辑像素 × sf → 物理像素）。
    ///
    /// 为什么返回 `Result`：位图载荷需要解 base64 + 解 PNG，属于可能失败的
    /// 解码操作。宁可把错误回传给前端（前端会显示"复制失败：…"），
    /// 也不要静默丢一层标注让用户拿到缺字的图。
    fn to_shape(&self, sf: f64) -> Result<snappin_core_algo::shapes::Shape, String> {
        use snappin_core_algo::shapes::{Shape, ShapeStyle};
        let sf = sf as f32;
        let style = |color: [u8; 4], width: f32| ShapeStyle::new(color, width * sf);
        let shape = match self {
            ShapePayload::Rect { x, y, w, h, color, width } => Shape::Rect {
                x: (*x * sf) as i32,
                y: (*y * sf) as i32,
                w: (*w * sf).max(1.0) as u32,
                h: (*h * sf).max(1.0) as u32,
                style: style(*color, *width),
            },
            ShapePayload::Ellipse { x, y, w, h, color, width } => Shape::Ellipse {
                x: (*x * sf) as i32,
                y: (*y * sf) as i32,
                w: (*w * sf).max(1.0) as u32,
                h: (*h * sf).max(1.0) as u32,
                style: style(*color, *width),
            },
            ShapePayload::Arrow { x1, y1, x2, y2, color, width } => Shape::Arrow {
                x1: (*x1 * sf) as i32,
                y1: (*y1 * sf) as i32,
                x2: (*x2 * sf) as i32,
                y2: (*y2 * sf) as i32,
                style: style(*color, *width),
            },
            ShapePayload::Pen { points, color, width } => Shape::Pen {
                points: points.iter().map(|p| (p[0] * sf, p[1] * sf)).collect(),
                style: style(*color, *width),
            },
            ShapePayload::Highlighter { points, color, width } => Shape::Highlighter {
                points: points.iter().map(|p| (p[0] * sf, p[1] * sf)).collect(),
                style: style(*color, *width),
            },
            ShapePayload::Mosaic { x, y, w, h, block } => Shape::Mosaic {
                x: (*x * sf) as i32,
                y: (*y * sf) as i32,
                w: (*w * sf).max(1.0) as u32,
                h: (*h * sf).max(1.0) as u32,
                block: (*block).max(2),
            },
            // 位图已是物理像素：只把锚点换算过去，尺寸与像素原样使用
            ShapePayload::Bitmap { x, y, png } => {
                let (w, h, rgba) = decode_png_base64(png)?;
                Shape::Bitmap {
                    x: (*x * sf) as i32,
                    y: (*y * sf) as i32,
                    w,
                    h,
                    rgba,
                }
            }
            ShapePayload::Spotlight { x, y, w, h, radius, dim } => Shape::Spotlight {
                x: (*x * sf) as i32,
                y: (*y * sf) as i32,
                w: (*w * sf).max(1.0) as u32,
                h: (*h * sf).max(1.0) as u32,
                radius: (*radius * sf).max(0.0) as u32,
                dim: *dim,
            },
        };
        Ok(shape)
    }
}

/// base64 PNG → (宽, 高, RGBA8)。
///
/// 位图来自可信的本应用前端，但仍按不可信输入处理：长度、尺寸、
/// 解码失败都要给出明确错误，不做任何 `unwrap`。
fn decode_png_base64(png_b64: &str) -> Result<(u32, u32, Vec<u8>), String> {
    use base64::Engine as _;
    // 前端可能带着 data URL 前缀传上来（防御性处理）
    let body = png_b64
        .rsplit_once(',')
        .filter(|(head, _)| head.starts_with("data:"))
        .map(|(_, tail)| tail)
        .unwrap_or(png_b64)
        .trim();
    if body.is_empty() {
        return Err("bitmap payload is empty".to_string());
    }
    let raw = base64::engine::general_purpose::STANDARD
        .decode(body)
        .map_err(|e| format!("bitmap base64 decode failed: {e}"))?;
    let img = image::load_from_memory_with_format(&raw, image::ImageFormat::Png)
        .map_err(|e| format!("bitmap png decode failed: {e}"))?
        .to_rgba8();
    let (w, h) = (img.width(), img.height());
    if w == 0 || h == 0 {
        return Err("bitmap has zero size".to_string());
    }
    Ok((w, h, img.into_raw()))
}

/// 载荷列表 → core-algo 图形列表。
fn to_shapes(
    payloads: &[ShapePayload],
    sf: f64,
) -> Result<Vec<snappin_core_algo::shapes::Shape>, String> {
    payloads.iter().map(|p| p.to_shape(sf)).collect()
}

// ---------- 截图主流程（命令 / 快捷键 / 托盘共用） ----------

/// 截图主流程（可在任意线程调用；内部处理线程亲和性）。
///
/// 顺序至关重要（曾踩坑）：**必须先采集、后显示任何本应用 UI**。
/// 早期版本"先开浮层再采集"，导致浮层的 loading 遮罩/文字被截进冻结图；
/// 悬浮球若未隐藏也会出现在截图里。
///
/// 现行流程：
/// 1. 隐藏悬浮球（主线程执行，等合成器刷新）
/// 2. 后台线程并行预创建**隐藏**浮层（与采集重叠，降低感知延迟）
/// 3. xcap 并行采集全部显示器 + fast 压缩
/// 4. 写入状态 → 主线程显示浮层（页面此时才渲染冻结图）
/// 采集互斥守卫：`Drop` 时释放标志，任何返回路径（含 ? 提前返回）都安全。
struct CaptureGuard<'a>(&'a std::sync::atomic::AtomicBool);

impl Drop for CaptureGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

pub fn do_capture(app: &AppHandle, state: &AppState, src: &str) -> Result<CapturePayload, String> {
    let t0 = std::time::Instant::now();
    // 0. 互斥：热键/球点击/前端命令可能在同一瞬间重复触发。没有这道闸，
    // 第二遍采集会复用第一遍预创建的浮层（页面不重载→前端状态陈旧），
    // 并把浮层自身截进冻结图，表现为"整屏发灰 + 截图采集超时"。
    if state
        .capturing
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        eprintln!("[snappin] capture skipped (src={src}, session={} busy)", state.session.load(Ordering::SeqCst));
        return Err("capture already in progress".to_string());
    }
    let _guard = CaptureGuard(&state.capturing);
    // 新会话代号：上一轮迟到的 cancel/confirm 请求据此判定为陈旧并忽略
    let session_id = state.session.fetch_add(1, Ordering::SeqCst) + 1;
    state.cancel_pending.store(false, Ordering::SeqCst);
    eprintln!("[snappin] capture start (session {session_id}, src={src})");

    // 1. 采集前隐藏悬浮球（否则球体会被截进冻结图），并等待合成器刷新
    let ball_hidden = windows::hide_ball_for_capture(app);
    if ball_hidden {
        std::thread::sleep(Duration::from_millis(100));
    }

    // 2. 并行预创建隐藏浮层（与采集重叠；失败无妨，第 4 步会兜底创建）
    let precreate_app = app.clone();
    let precreate = std::thread::spawn(move || {
        windows::precreate_overlays_hidden(&precreate_app);
    });

    // 3. 采集（并行 + fast 压缩，双屏约 0.9s）
    let capture = snappin_core_platform::capture_all_monitors();

    // 等预创建线程结束（采集已完成或接近完成）
    let _ = precreate.join();

    let shots = match capture {
        Ok(s) => s,
        Err(e) => {
            // 采集失败：关掉隐藏浮层、清空会话、恢复悬浮球，返回错误
            eprintln!("[snappin] xcap capture error (session {session_id}): {e}");
            windows::end_capture_session(app, state, "xcap-error");
            return Err(e.to_string());
        }
    };
    eprintln!(
        "[snappin] captured {} monitor(s) in {}ms (session {session_id})",
        shots.len(),
        t0.elapsed().as_millis()
    );
    // 采集期间用户已按取消（前端此时还没有会话代号）：在显示浮层前放弃，
    // 避免"Escl 了却又弹出浮层"。
    if state.cancel_pending.swap(false, Ordering::SeqCst) {
        windows::end_capture_session(app, state, "cancel-pending");
        eprintln!("[snappin] capture canceled before show (session {session_id})");
        return Err("capture canceled".to_string());
    }

    {
        let mut guard = state.monitors.lock().unwrap();
        *guard = shots.clone();
    }

    // 4. 显示浮层（主线程；几何用 xcap 精确值校正）
    windows::show_overlays(app, &shots);
    eprintln!(
        "[snappin] session {session_id} ready, {}ms total",
        t0.elapsed().as_millis()
    );

    Ok(build_capture_payload(&shots))
}

#[tauri::command]
pub async fn start_capture(app: AppHandle) -> Result<CapturePayload, String> {
    // Windows 已知问题：同步命令/主线程内创建 webview 会死锁或导致 webview 空白，
    // 必须放到阻塞线程执行（见 WebviewWindowBuilder 文档的 Known issues）。
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app2.state::<AppState>();
        do_capture(&app2, &state, "cmd")
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
    // 采集中（state.monitors 为空或尚未覆盖到本显示器）：
    // 返回 ready=false 的占位载荷，前端轮询等待，不视为错误。
    let Some(m) = monitors.get(idx) else {
        // 诊断：越界说明"浮层索引 > 采集结果数"，该浮层会永远等不到 ready。
        // 轮询很频繁，用静态计数限流，只打前几条。
        static WAIT_LOGS: AtomicUsize = AtomicUsize::new(0);
        let n = WAIT_LOGS.fetch_add(1, Ordering::Relaxed);
        if n < 3 {
            eprintln!(
                "[snappin] overlay {} waits: idx={} captured={}",
                window.label(),
                idx,
                monitors.len()
            );
        }
        return Ok(OverlayContextPayload {
            monitor_id: String::new(),
            image_path: String::new(),
            natural_width: 0,
            natural_height: 0,
            scale_factor: 1.0,
            ready: false,
            session_id: state.session.load(Ordering::SeqCst),
        });
    };
    Ok(OverlayContextPayload {
        monitor_id: m.id.clone(),
        image_path: m.path.to_string_lossy().to_string(),
        natural_width: m.width,
        natural_height: m.height,
        scale_factor: m.scale_factor,
        ready: true,
        session_id: state.session.load(Ordering::SeqCst),
    })
}

#[tauri::command]
pub async fn confirm_selection(
    app: AppHandle,
    state: State<'_, AppState>,
    monitor_id: String,
    rect: RectPayload,
    action: String,
    shapes: Option<Vec<ShapePayload>>,
    session_id: Option<u64>,
) -> Result<ConfirmPayload, String> {
    // 陈旧确认（上一轮会话迟到的请求）：直接丢弃，避免误操作新会话
    if let Some(id) = session_id {
        let current = state.session.load(Ordering::SeqCst);
        if id != current {
            return Err(format!("stale session {id} != {current}"));
        }
    }
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
        let mut cropped = m.image.crop(&algo_rect).map_err(|e| e.to_string())?;
        // 标注光栅化到裁剪图上（复制/贴图/保存三个出口共用同一份像素，
        // 保证所见即所得）。标注坐标同为逻辑像素，这里一起换算。
        if let Some(payloads) = &shapes {
            if !payloads.is_empty() {
                snappin_core_algo::draw::draw_shapes(&mut cropped, &to_shapes(payloads, sf)?);
            }
        }
        let (ox, oy) = m.logical_origin();
        (cropped, sf, ox, oy)
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
            // 武装 Ctrl+V：复制成功后短时间内，全局 Ctrl+V 由 SnapPin
            // 拦截并弹出贴图窗口（跟随鼠标），过期后恢复正常粘贴。
            crate::arm_paste(&app, &state);
        }
        "pin" => {
            let label = pin_label(&path);
            let disp_w = (cropped.width as f64 / scale_factor).max(8.0);
            let disp_h = (cropped.height as f64 / scale_factor).max(8.0);
            let x = monitor_x + rect.x as f64;
            let y = monitor_y + rect.y as f64;
            // 建窗线程约束因平台而异（见 windows::create_pin_window 文档）：
            // Windows 主线程同步建窗会死锁 → 丢到阻塞线程；macOS 必须主线程 → 内部投递。
            let app2 = app.clone();
            let label2 = label.clone();
            #[cfg(windows)]
            tauri::async_runtime::spawn_blocking(move || {
                windows::create_pin_window(&app2, &label2, disp_w, disp_h, x, y)
            })
            .await
            .map_err(|e| format!("pin window task failed: {e}"))??;
            #[cfg(not(windows))]
            windows::create_pin_window(&app2, &label2, disp_w, disp_h, x, y)?;
            register_pin(&state, &label, &path_str, disp_w, disp_h);
        }
        "save" => {
            // 先结束会话再弹对话框：① always-on-top 全屏浮层会盖住系统保存
            // 对话框（无主窗口，z-order 低于置顶浮层）；② 提前 return，
            // 避免对话框停留期间用户开启新截图后，被函数尾部的清理误杀
            // （防重入只看浮层是否存在，此时浮层已关）。
            windows::end_capture_session(&app, &state, "save-before-dialog");
            let dest = save_dialog(
                &app,
                &snappin_core_platform::default_save_name(),
                save_dir,
            )
            .await;
            if let Some(dest) = dest {
                // 临时文件固定为 PNG；按用户选择的扩展名重新编码落盘
                let buffer = snappin_core_platform::load_rgba(Path::new(&path_str))
                    .map_err(|e| e.to_string())?;
                snappin_core_platform::save_image(&buffer, &dest).map_err(|e| e.to_string())?;
                // 记住此次目录，下次保存对话框默认打开这里
                if let Some(parent) = dest.parent() {
                    let dir_str = parent.to_string_lossy().to_string();
                    let mut guard = state.config.lock().unwrap();
                    if guard.save_dir.as_deref() != Some(dir_str.as_str()) {
                        guard.save_dir = Some(dir_str.clone());
                        let snapshot = guard.clone();
                        drop(guard);
                        if let Ok(dir) = app.path().app_config_dir() {
                            let _ = crate::config::save(&dir, &snapshot);
                        }
                    }
                }
            }
            return Ok(ConfirmPayload { path: path_str });
        }
        other => return Err(format!("unknown action: {}", other)),
    }

    // 复制/贴图成功：结束会话（关闭全部浮层 + 清空数据）。
    // 前端每块浮层还会自关 + 轮询兜底，三级保障"完全退出截图模式"。
    windows::end_capture_session(&app, &state, "confirm-ok");
    Ok(ConfirmPayload { path: path_str })
}

#[tauri::command]
pub fn cancel_capture(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: Option<u64>,
) -> Result<(), String> {
    let current = state.session.load(Ordering::SeqCst);
    eprintln!(
        "[snappin] cancel_capture requested (arg={:?}, current={current}, capturing={})",
        session_id,
        state.capturing.load(Ordering::SeqCst)
    );
    if let Some(id) = session_id {
        if id != current {
            // 上一轮会话的迟到取消：新会话已经在进行，忽略它，
            // 否则会关掉新浮层并清空新采集数据（表现为采集超时）。
            eprintln!("[snappin] cancel ignored: stale session {id} != {current}");
            return Ok(());
        }
    } else if state.capturing.load(Ordering::SeqCst) {
        // 采集还没完成，前端尚无会话代号：登记取消，由 do_capture 在
        // 显示浮层之前消费（此刻直接关窗会与预创建流程冲突）。
        state.cancel_pending.store(true, Ordering::SeqCst);
        return Ok(());
    }
    windows::end_capture_session(&app, &state, "cancel-capture");
    Ok(())
}

/// 前端诊断日志：release 构建里 DevTools 控制台不可用，
/// 统一汇到 stderr，便于与 Rust 侧日志按同一时间线排查。
#[tauri::command]
pub fn ui_log(window: Window, msg: String) {
    eprintln!("[ui:{}] {msg}", window.label());
}

// ---------- 贴图 ----------

/// 剪贴板贴图的实际逻辑（供命令与托盘/快捷键路径共用）。
/// `at`: Some((逻辑 x, 逻辑 y)) 时贴图左上角对齐该位置（Ctrl+V 跟随鼠标），
/// None 时主屏居中。内部会创建贴图 webview 窗口，
/// 调用方须确保不在主线程同步执行。
pub fn do_pin_from_clipboard(
    app: &AppHandle,
    state: &AppState,
    at: Option<(f64, f64)>,
) -> Result<String, String> {
    let buffer = snappin_core_platform::clipboard_get_image()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "剪贴板中没有图片".to_string())?;
    let path = snappin_core_platform::new_temp_png_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    snappin_core_platform::save_png_fast(&buffer, &path).map_err(|e| e.to_string())?;
    let path_str = path.to_string_lossy().to_string();
    let label = pin_label(&path);

    // 位置换算：光标给的是物理像素，需按所在显示器的缩放因子转逻辑像素
    let (disp_w, disp_h, x, y) = match at {
        Some((px, py)) => {
            let sf = app
                .monitor_from_point(px, py)
                .ok()
                .flatten()
                .map(|m| m.scale_factor())
                .unwrap_or(1.0);
            let dw = (buffer.width as f64 / sf).max(8.0);
            let dh = (buffer.height as f64 / sf).max(8.0);
            // 贴图窗口以光标为左上角，但贴到屏幕右/下边缘时回退为贴住边缘
            let mon = app.monitor_from_point(px, py).ok().flatten();
            let (mon_w, mon_h) = mon
                .map(|m| {
                    let s = m.scale_factor();
                    let sz = m.size();
                    (sz.width as f64 / s, sz.height as f64 / s)
                })
                .unwrap_or((dw, dh));
            let mx = px / sf;
            let my = py / sf;
            let cx = if mx + dw > mon_w { (mon_w - dw).max(0.0) } else { mx };
            let cy = if my + dh > mon_h { (mon_h - dh).max(0.0) } else { my };
            (dw, dh, cx, cy)
        }
        None => {
            let sf = app
                .primary_monitor()
                .ok()
                .flatten()
                .map(|m| m.scale_factor())
                .unwrap_or(1.0);
            let dw = (buffer.width as f64 / sf).max(8.0);
            let dh = (buffer.height as f64 / sf).max(8.0);
            let (cx, cy) = windows::place_centered_on_primary(app, dw, dh);
            (dw, dh, cx, cy)
        }
    };
    windows::create_pin_window(app, &label, disp_w, disp_h, x, y)?;
    register_pin(state, &label, &path_str, disp_w, disp_h);
    Ok(label)
}

#[tauri::command]
pub async fn pin_from_clipboard(app: AppHandle) -> Result<String, String> {
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app2.state::<AppState>();
        do_pin_from_clipboard(&app2, &state, None)
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

/// 切换贴图窗的置顶状态（工具条"置顶"按钮）。
/// always_on_top 的 setter 须在主线程执行（PostMessage 窗口句柄）。
#[tauri::command]
pub async fn pin_set_topmost(
    app: AppHandle,
    state: State<'_, AppState>,
    label: String,
    topmost: bool,
) -> Result<(), String> {
    // 仅允许操作已登记的贴图窗（防止恶意 label 操作其他窗口）
    if !state.pins.lock().unwrap().contains_key(&label) {
        return Err("pin window not found".to_string());
    }
    windows::set_window_topmost(&app, &label, topmost);
    Ok(())
}

/// 切换贴图窗的鼠标穿透（工具条"穿透"按钮）。
///
/// 穿透 = 窗口对鼠标事件透明（SetWindowLong WS_EX_TRANSPARENT），
/// 用于临时查看/操作被贴图遮住的内容。穿透后本窗工具条也点不到，
/// 故前端必须提供自动恢复（定时）与提示；恢复方式见 PinWindow 注释。
#[tauri::command]
pub async fn pin_set_ignore_cursor_events(
    app: AppHandle,
    state: State<'_, AppState>,
    label: String,
    ignore: bool,
) -> Result<(), String> {
    if !state.pins.lock().unwrap().contains_key(&label) {
        return Err("pin window not found".to_string());
    }
    windows::set_window_ignore_cursor_events(&app, &label, ignore);
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
    window: Window,
    label: String,
) -> Result<bool, String> {
    let save_dir = { state.config.lock().unwrap().save_dir.clone() };
    let src = pin_src(&state, &label)?;
    // 贴图窗是 always-on-top，会盖住系统保存对话框：临时取消置顶
    windows::set_window_topmost(&app, &label, false);
    let picked = save_dialog(
        &app,
        &snappin_core_platform::default_save_name(),
        save_dir,
    )
    .await;
    // 恢复置顶（窗口可能已被关闭，忽略错误）
    windows::set_window_topmost(&app, &label, true);
    if let Some(dest) = picked {
        let buffer =
            snappin_core_platform::load_rgba(Path::new(&src)).map_err(|e| e.to_string())?;
        snappin_core_platform::save_image(&buffer, &dest).map_err(|e| e.to_string())?;
        if let Some(parent) = dest.parent() {
            let dir_str = parent.to_string_lossy().to_string();
            let mut guard = state.config.lock().unwrap();
            if guard.save_dir.as_deref() != Some(dir_str.as_str()) {
                guard.save_dir = Some(dir_str.clone());
                let snapshot = guard.clone();
                drop(guard);
                if let Ok(dir) = app.path().app_config_dir() {
                    let _ = crate::config::save(&dir, &snapshot);
                }
            }
        }
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
            // 防重入：浮层已存在（含采集中 loading 态）时忽略重复触发
            if app
                .webview_windows()
                .keys()
                .any(|l| l.starts_with(windows::OVERLAY_PREFIX))
            {
                return Ok(());
            }
            let app2 = app.clone();
            tauri::async_runtime::spawn_blocking(move || {
                let state = app2.state::<AppState>();
                do_capture(&app2, &state, "ball")
            })
            .await
            .map_err(|e| format!("capture task failed: {e}"))?
            // 已有一轮采集在跑（含热键/托盘同时触发）：视为成功，
            // 不把 "capture already in progress" 当错误弹给用户。
            .map(|_| ())
            .or_else(|e| {
                if e.contains("already in progress") {
                    Ok(())
                } else {
                    Err(e)
                }
            })?;
        }
        "pin-clipboard" => {
            let app2 = app.clone();
            let r = tauri::async_runtime::spawn_blocking(move || {
                let state = app2.state::<AppState>();
                do_pin_from_clipboard(&app2, &state, None)
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

/// 悬浮球右键菜单：构建原生菜单并在光标位置弹出。
///
/// 背景：wry 0.57 默认开启 WebView2 浏览器右键菜单（含"打印"等），
/// 已由 webview 初始化脚本抑制；本命令提供应用自己的右键菜单。
/// 菜单项与托盘共用 `crate::handle_menu_event` 分发，行为一致。
///
/// 必须是 **async** 命令：同步命令在主线程执行，内部的 run_on_main
/// 会自等待死锁；async 命令跑在异步运行时线程，run_on_main 才能
/// 把 popup 闭包投递到主线程（popup_menu 会阻塞主线程直到用户选择，
/// 这是模态菜单的预期行为）。
#[tauri::command]
pub async fn ball_menu(app: AppHandle) -> Result<(), String> {
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};

    let i_capture =
        MenuItem::with_id(&app, "capture", "截图 (Ctrl+1)", true, None::<&str>)
            .map_err(|e| e.to_string())?;
    let i_pin = MenuItem::with_id(&app, "pin-clipboard", "剪贴板贴图 (Ctrl+2)", true, None::<&str>)
        .map_err(|e| e.to_string())?;
    let i_hide = MenuItem::with_id(&app, "hide-ball", "隐藏悬浮球", true, None::<&str>)
        .map_err(|e| e.to_string())?;
    let i_about =
        MenuItem::with_id(&app, "about", "关于v0.1", true, None::<&str>).map_err(|e| e.to_string())?;
    let i_quit =
        MenuItem::with_id(&app, "quit", "退出", true, None::<&str>).map_err(|e| e.to_string())?;
    let sep1 = PredefinedMenuItem::separator(&app).map_err(|e| e.to_string())?;
    let sep2 = PredefinedMenuItem::separator(&app).map_err(|e| e.to_string())?;
    let menu = Menu::with_items(
        &app,
        &[&i_capture, &i_pin, &sep1, &i_hide, &i_about, &sep2, &i_quit],
    )
    .map_err(|e| e.to_string())?;

    windows::run_on_main(&app, move |app| {
        if let Some(w) = app.get_webview_window(windows::BALL_LABEL) {
            if let Err(e) = w.popup_menu(&menu) {
                eprintln!("[snappin] ball menu popup failed: {e}");
            }
        }
    });
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

/// 更新配置并落盘；悬浮球尺寸变化立即生效。
///
/// async 原因：内部 run_on_main 向主线程投递闭包并等待完成，
/// 同步命令在主线程执行会自等待死锁（见 ball_menu 注释）。
#[tauri::command]
pub async fn update_config(
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
    // 悬浮球尺寸变化立即生效（窗口操作须在主线程执行）
    let new_size = config.ball_size as f64;
    windows::run_on_main(&app, move |app| {
        if let Some(w) = app.get_webview_window(windows::BALL_LABEL) {
            let _ = w.set_size(tauri::LogicalSize::new(new_size, new_size));
        }
    });
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

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;

    /// 2x1 的 PNG（左红右绿），base64 编码——模拟前端 `toDataURL` 的尾巴
    fn sample_png_b64() -> String {
        let mut img = image::RgbaImage::new(2, 1);
        img.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
        img.put_pixel(1, 0, image::Rgba([0, 255, 0, 255]));
        let mut buf = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
            .expect("encode png");
        base64::engine::general_purpose::STANDARD.encode(buf)
    }

    #[test]
    fn decodes_base64_png() {
        let (w, h, rgba) = decode_png_base64(&sample_png_b64()).expect("decode");
        assert_eq!((w, h), (2, 1));
        assert_eq!(&rgba[0..4], &[255, 0, 0, 255]);
        assert_eq!(&rgba[4..8], &[0, 255, 0, 255]);
    }

    #[test]
    fn accepts_data_url_prefix_and_rejects_garbage() {
        let with_prefix = format!("data:image/png;base64,{}", sample_png_b64());
        assert!(decode_png_base64(&with_prefix).is_ok());
        assert!(decode_png_base64("").is_err());
        assert!(decode_png_base64("!!!not-base64!!!").is_err());
        // 合法 base64 但不是 PNG
        assert!(decode_png_base64("aGVsbG8=").is_err());
    }

    #[test]
    fn bitmap_payload_scales_anchor_but_keeps_pixels() {
        let p = ShapePayload::Bitmap { x: 10.0, y: 20.0, png: sample_png_b64() };
        match p.to_shape(2.0).expect("to_shape") {
            snappin_core_algo::shapes::Shape::Bitmap { x, y, w, h, rgba } => {
                assert_eq!((x, y), (20, 40), "锚点应乘 DPR");
                assert_eq!((w, h), (2, 1), "位图像素不应再缩放");
                assert_eq!(rgba.len(), 8);
            }
            other => panic!("expect bitmap, got {other:?}"),
        }
    }

    #[test]
    fn bad_bitmap_payload_fails_instead_of_silently_dropping() {
        let p = ShapePayload::Bitmap { x: 0.0, y: 0.0, png: "broken".into() };
        assert!(p.to_shape(1.0).is_err());
    }

    #[test]
    fn spotlight_payload_scales_geometry_but_not_dim() {
        let p = ShapePayload::Spotlight {
            x: 1.0,
            y: 2.0,
            w: 30.0,
            h: 10.0,
            radius: 6.0,
            dim: 150,
        };
        match p.to_shape(2.0).expect("to_shape") {
            snappin_core_algo::shapes::Shape::Spotlight { x, y, w, h, radius, dim } => {
                // 几何量乘 DPR，压暗强度是颜色语义、绝不能乘
                assert_eq!((x, y, w, h, radius, dim), (2, 4, 60, 20, 12, 150));
            }
            other => panic!("expect spotlight, got {other:?}"),
        }
    }
}
