//! 窗口管理：悬浮球 / 截图浮层 / 贴图窗口的创建与关闭。

use tauri::webview::WebviewWindowBuilder;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WebviewUrl};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use crate::config::AppConfig;
use crate::state::AppState;

pub const BALL_LABEL: &str = "ball";
pub const OVERLAY_PREFIX: &str = "overlay-";
pub const PIN_PREFIX: &str = "pin-";
/// OCR 结果窗（单例：文字识别结果 + 翻译入口）
pub const OCR_LABEL: &str = "ocr";
/// 翻译窗（单例：左原文右译文）
pub const TRANS_LABEL: &str = "trans";
/// 设置窗（单例）。
///
/// **不能叫 "main"**：那是 Tauri 默认窗口的 label（`default_window_label()`），
/// 撞名会导致 `get_webview_window("main")` 取到那个加载 index.html 失败的黑框，
/// 设置窗永远开不出来。用 "settings" 彻底避开。
pub const SETTINGS_LABEL: &str = "settings";

/// 注入到所有 webview 的守卫脚本（document-start，早于页面脚本执行）。
///
/// 背景：wry 0.57 的 WebView2 默认开启浏览器原生右键菜单（返回/刷新/
/// **打印**/查看源代码…），桌面应用里出现这种菜单是 bug——用户右键悬浮球
/// 点"打印"会弹出一个空白页的打印对话框。Tauri 未暴露 wry 的
/// `default_context_menus` 开关，故在前端层拦截。
///
/// - `contextmenu`：preventDefault 只取消默认菜单，不阻止事件传播，
///   各窗口仍可在 React 里正常响应右键（悬浮球 → 原生菜单 ball_menu）。
/// - 浏览器快捷键：Ctrl+P 打印 / F5、Ctrl+R 刷新 / Ctrl+S 保存 / Ctrl+O 打开。
pub(crate) const WEBVIEW_GUARD: &str = r#"
(function () {
  window.addEventListener('contextmenu', function (e) { e.preventDefault(); });
  window.addEventListener('keydown', function (e) {
    if (e.key === 'F5') { e.preventDefault(); return; }
    if (e.ctrlKey && ['p', 'r', 's', 'o'].indexOf(e.key.toLowerCase()) !== -1) {
      e.preventDefault();
    }
  });
})();
"#;

/// 悬浮球窗口相对球体的留白（逻辑像素）。
///
/// 这个值由**外投影**决定，不是随便留的：透明窗口会把超出窗口的内容直接裁掉，
/// 留白不足时阴影在上下左右变成一条直线（用户看到的"切面"）。
/// 球半径 28，阴影 `0 6px 16px` → 球心到底部需要 28 + 6 + 16 = 50；
/// hover 时 `scale(1.07)` + `0 5px 14px` → 28×1.07 + (5+14)×1.07 ≈ 50.3。
/// 取 24 时窗口半边长 = 28 + 24 = 52 ≥ 50.3，静止与 hover 都不裁。
const BALL_MARGIN: f64 = 24.0;

/// 贴图窗口相对图片的留白（逻辑像素）。
/// 图片有外投影 box-shadow，窗口必须比图片大一圈，否则阴影被窗口边缘裁掉。
const PIN_MARGIN: f64 = 10.0;

/// 截图期间是否因采集而隐藏了悬浮球（用于结束后恢复）。
static BALL_HIDDEN_BY_CAPTURE: AtomicBool = AtomicBool::new(false);

/// 创建悬浮球窗口（已存在则跳过）。
pub fn ensure_ball_window(app: &AppHandle, cfg: &AppConfig) -> tauri::Result<()> {
    if app.get_webview_window(BALL_LABEL).is_some() {
        return Ok(());
    }
    let size = cfg.ball_size as f64;
    // 窗口比球大一圈：容纳外投影与 hover 放大动画（见 BALL_MARGIN 注释）
    let win = size + BALL_MARGIN * 2.0;
    let window = WebviewWindowBuilder::new(app, BALL_LABEL, WebviewUrl::App("ball.html".into()))
        .title("SnapPin")
        .inner_size(win, win)
        // 实测：Windows 会把小于 SM_CXMINTRACK（本机 136px）的窗口**强行加宽**，
        // 于是 104x104 变成 136x104。试过 min_inner_size(1.0, 1.0)，无效——
        // 这条限制来自系统而非 winit，所以位置一律按**实际** inner_size 换算。
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .shadow(false)
        .initialization_script(WEBVIEW_GUARD)
        .build()?;

    let inner = window.inner_size().unwrap_or_default();
    let outer = window.outer_size().unwrap_or_default();
    eprintln!(
        "[snappin] ball window built: inner={:?} outer={:?}",
        inner, outer
    );

    // 兜底：build() 出来的窗口在某些机器/多显示器切换后处于不可见或最小化
    // （用户报"启动后没有悬浮球"，日志里位置明明设置成功）。
    // 显式三连：show + 取消最小化 + 前置。放在任何 set_position 之前，
    // 避免"窗口隐藏时 set_position 被系统合并丢弃"。
    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();
    eprintln!(
        "[snappin] ball shown: visible={:?} minimized={:?}",
        window.is_visible(),
        window.is_minimized()
    );

    // 位置：**固定主屏右上角**（用户明确要求：每次启动都在主屏右上部）。
    //
    // 为什么不记忆位置了：球允许在会话内拖动，但持久化位置踩过一堆坑——
    // 拔副屏/改分辨率后旧位置落到屏幕外（用户看到"球丢了"）、位置存到副屏
    // 用户在主屏找不到。固定位 + 允许会话内拖动，是截图工具最稳的形态
    // （PixPin/Shottr 默认也都是可预期的固定位）。
    //
    // 两个坑仍在：
    // 1) Windows 有最小跟踪宽度（实测 136px），小于它的窗口会被**加宽**成非正方形，
    //    所以半宽/半高必须按实际 inner_size 分别取，不能假设正方形。
    // 2) "加宽 + 重新放置"发生在 build() 之后，紧接着的 set_position 可能被它覆盖
    //    （现象：球跳回主屏中部）。所以分几拍再落一次，最后一拍为准。
    place_ball_top_right(app, &window, "immediate");
    // 线程要 'static：clone 一份 AppHandle 进去（不能借 app）
    let app_handle = app.clone();
    let win_handle = window.clone();
    std::thread::spawn(move || {
        for (delay, tag) in [(250u64, "t+250"), (900, "t+900")] {
            std::thread::sleep(std::time::Duration::from_millis(delay));
            place_ball_top_right(&app_handle, &win_handle, tag);
        }
        // 最后一拍：再确保一次可见（防"窗口建好了但没显示"）
        if let Some(w) = win_handle.get_webview_window(BALL_LABEL) {
            let _ = w.show();
        }
    });
    Ok(())
}


/// 悬浮球窗口应有的边长（逻辑像素）：球径 + 两侧留白。
///
/// 留白不是装饰：球有外投影、hover 时还会 `scale(1.07)`，窗口正好等于球径
/// 时这些都会被窗口边界裁成直线（用户看到的就是"球显示不完整"）。
pub fn ball_window_size(ball: f64) -> f64 {
    ball + BALL_MARGIN * 2.0
}

/// 按新球径调整悬浮球窗口，并重新贴回主屏右上角。
///
/// 为什么必须重新放：窗口尺寸变了但位置不变时，球的中心会跟着左上角走，
/// 表现为"改完尺寸球跑偏了"；而且 Windows 会把过窄窗口强制加宽，
/// 所以位置一律按**实际** inner_size 重算（见 place_ball_top_right）。
pub fn resize_ball_window(app: &AppHandle, ball: f64) {
    let Some(w) = app.get_webview_window(BALL_LABEL) else {
        return;
    };
    let side = ball_window_size(ball);
    let _ = w.set_size(tauri::LogicalSize::new(side, side));
    place_ball_top_right(app, &w, "resize");
}

/// 把悬浮球窗口放到**主屏右上角**（内缩 16px）。
///
/// 用实际 inner_size 算右下角：Windows 会把窄窗加宽到最小跟踪尺寸，
/// 用请求尺寸算会让球右侧超出屏幕或被裁。
fn place_ball_top_right<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    window: &tauri::WebviewWindow<R>,
    tag: &str,
) {
    let Some(monitor) = app.primary_monitor().ok().flatten() else {
        eprintln!("[snappin] ball place ({tag}): no primary monitor");
        return;
    };
    let actual = window.inner_size().unwrap_or_default();
    if actual.width == 0 || actual.height == 0 {
        return;
    }
    // 横向贴右 16px；纵向下移 66px（16 + 50，用户要求"离原位置再向下 50px"）——
    // 太贴顶容易被别的置顶窗/系统标题栏盖住，往下挪一点更稳。
    const MARGIN_X: i32 = 16;
    const MARGIN_Y: i32 = 66;
    let pos = monitor.position();
    let msize = monitor.size();
    // 目标位算完**再整体钳进屏幕**——屏幕比窗口还窄的 pathological 情况
    // （超小屏 / 极端 DPI）下，理想位会算成负数，球就飞出屏幕了。
    // 钳完之后任何分辨率下球都完整可见（那种情况下退化为贴左/贴上）。
    let nx = (pos.x + msize.width as i32 - actual.width as i32 - MARGIN_X)
        .clamp(pos.x, (pos.x + msize.width as i32 - actual.width as i32).max(pos.x));
    let ny = (pos.y + MARGIN_Y)
        .clamp(pos.y, (pos.y + msize.height as i32 - actual.height as i32).max(pos.y));
    if let Err(e) = window.set_position(PhysicalPosition::new(nx, ny)) {
        eprintln!("[snappin] ball position ({tag}) failed: {e}");
        return;
    }
    let got = window
        .outer_position()
        .map(|p| format!("({},{})", p.x, p.y))
        .unwrap_or_else(|_| "?".into());
    eprintln!(
        "[snappin] ball placed top-right ({tag}): want=({nx},{ny}) got={got} size={}x{}",
        actual.width, actual.height
    );
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

/// 采集前隐藏悬浮球（必须：否则球体会被截进冻结图）。
/// 窗口操作须在主线程执行，这里经 run_on_main_thread 同步等待完成。
/// 返回是否真的隐藏了（供调用方等待合成器刷新）。
pub fn hide_ball_for_capture(app: &AppHandle) -> bool {
    let visible = app
        .get_webview_window(BALL_LABEL)
        .map(|w| w.is_visible().unwrap_or(false))
        .unwrap_or(false);
    if !visible {
        return false;
    }
    run_on_main(app, |app| {
        if let Some(w) = app.get_webview_window(BALL_LABEL) {
            let _ = w.hide();
        }
    });
    BALL_HIDDEN_BY_CAPTURE.store(true, Ordering::SeqCst);
    true
}

/// 后台并行预创建**隐藏**浮层（与采集重叠，降低点击→冻结的感知延迟）。
///
/// 关键：窗口以 `visible(false)` 创建，采集完成后才由 `show_overlays` 显示——
/// 采集期间屏幕上没有任何本应用的可见像素，冻结图干净（不会混入
/// loading 遮罩/文字/悬浮球）。失败无妨：`show_overlays` 会兜底创建。
///
/// 平台差异：调用方是采集线程（非主线程）。
/// - Windows：主线程同步建窗会死锁，必须在工作线程直接建（当前实现）。
/// - macOS：NSWindow 只能主线程建，这里投递到主线程并**同步等它建完**
///   （show_overlays 依赖窗口已存在，异步会漏显示）。
#[cfg(not(target_os = "macos"))]
pub fn precreate_overlays_hidden(app: &AppHandle) {
    build_overlays_hidden(app);
}

#[cfg(target_os = "macos")]
pub fn precreate_overlays_hidden(app: &AppHandle) {
    run_on_main(app, build_overlays_hidden);
}

fn build_overlays_hidden(app: &AppHandle) {
    let monitors = match app.available_monitors() {
        Ok(m) => m,
        Err(e) => {
            eprintln!("[snappin] precreate overlays: enumerate failed: {e}");
            return;
        }
    };
    for (i, m) in monitors.iter().enumerate() {
        let label = format!("{}{}", OVERLAY_PREFIX, i);
        if app.get_webview_window(&label).is_some() {
            // 复用路径：页面不会重新加载，前端计时基准陈旧（已在前端按
            // overlay-ready 重置）。这里打日志便于排查"上一轮未关干净"。
            eprintln!("[snappin] precreate: {label} already alive, reused");
            continue;
        }
        let sf = m.scale_factor();
        let logical_w = (m.size().width as f64 / sf).max(1.0);
        let logical_h = (m.size().height as f64 / sf).max(1.0);
        let built = WebviewWindowBuilder::new(app, &label, WebviewUrl::App("overlay.html".into()))
            .title("SnapPin Capture")
            .inner_size(logical_w, logical_h)
            .position(m.position().x as f64 / sf, m.position().y as f64 / sf)
            .decorations(false)
            .transparent(true)
            .always_on_top(true)
            .skip_taskbar(true)
            .resizable(false)
            .shadow(false)
            .visible(false) // 先隐藏：采集完成后统一显示
            .initialization_script(WEBVIEW_GUARD)
            .build();
        if let Err(e) = built {
            eprintln!("[snappin] precreate overlay {label} failed: {e}");
        }
    }
    eprintln!(
        "[snappin] precreate done: {} monitor(s) enumerated",
        monitors.len()
    );
}

/// 采集完成后显示全部浮层（不存在则兜底创建并显示）。
///
/// 几何用 xcap 的精确值校正（tauri 的枚举几何为近似值）；
/// set_size/set_position/show 必须在主线程执行，故经 run_on_main_thread。
pub fn show_overlays(app: &AppHandle, monitors: &[snappin_core_platform::MonitorShot]) {
    let corrections: Vec<(String, f64, f64, f64, f64)> = monitors
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let sf = m.scale_factor as f64;
            // 原点单位随平台不同（Windows 物理像素 / macOS 点），统一走 logical_origin
            let (ox, oy) = m.logical_origin();
            (
                format!("{}{}", OVERLAY_PREFIX, i),
                (m.width as f64 / sf).max(1.0),
                (m.height as f64 / sf).max(1.0),
                ox,
                oy,
            )
        })
        .collect();
    run_on_main(app, move |app| {
        for (label, w, h, x, y) in corrections {
            if let Some(win) = app.get_webview_window(&label) {
                let r1 = win.set_size(tauri::LogicalSize::new(w, h));
                let r2 = win.set_position(tauri::LogicalPosition::new(x, y));
                let r3 = win.show();
                // 重新置顶：Windows 的前台窗口锁会让 set_focus 静默失败，
                // 若桌面上有别家置顶窗（如悬浮助手类应用），浮层会被压在下面
                // 既看不见也收不到鼠标。set_always_on_top(true) 内部是
                // SetWindowPos(HWND_TOPMOST)，会把窗口抬到置顶带最前。
                let _ = win.set_always_on_top(true);
                let r4 = win.set_focus();
                eprintln!(
                    "[snappin] show {label}: size={:?} pos={:?} show={:?} focus={:?} visible={:?}",
                    r1.err(),
                    r2.err(),
                    r3.err(),
                    r4.err(),
                    win.is_visible()
                );
                // 主动通知前端"采集完成"：WebView2 的窗口 show() 不保证触发
                // document visibilitychange，前端轮询若只靠 visibilitychange
                // 门控会永远等不到 ready（表现为"截图采集超时"）。
                let _ = app.emit_to(&label, "overlay-ready", ());
            } else {
                // 兜底：预创建缺失（如采集中显示器热插拔），直接创建并显示
                let built = WebviewWindowBuilder::new(
                    app,
                    &label,
                    WebviewUrl::App("overlay.html".into()),
                )
                .title("SnapPin Capture")
                .inner_size(w, h)
                .position(x, y)
                .decorations(false)
                .transparent(true)
                .always_on_top(true)
                .skip_taskbar(true)
                    .resizable(false)
                    .shadow(false)
                    .focused(true)
                    .initialization_script(WEBVIEW_GUARD)
                    .build();
                if let Err(e) = built {
                    eprintln!("[snappin] fallback overlay {label} failed: {e}");
                } else {
                    let _ = app.emit_to(&label, "overlay-ready", ());
                }
            }
        }
    });
    // 多余浮层（tao 枚举数 > 实际采集数，如显示器热插拔、镜像/虚拟显示器）：
    // 直接关掉。否则它会盖在屏幕上、且 overlay_context 取不到对应数据，
    // 表现为永远"截图采集超时"。
    let extra: Vec<String> = app
        .webview_windows()
        .keys()
        .filter(|l| l.starts_with(OVERLAY_PREFIX))
        .filter(|l| {
            l.strip_prefix(OVERLAY_PREFIX)
                .and_then(|s| s.parse::<usize>().ok())
                .map(|i| i >= monitors.len())
                .unwrap_or(true)
        })
        .cloned()
        .collect();
    if !extra.is_empty() {
        eprintln!(
            "[snappin] closing {} extra overlay(s): {:?} (captured={})",
            extra.len(),
            extra,
            monitors.len()
        );
        for label in extra {
            if let Some(w) = app.get_webview_window(&label) {
                let _ = w.close();
            }
        }
    }
    eprintln!("[snappin] showed {} overlay window(s)", monitors.len());
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

/// 在主线程执行闭包并同步等待完成。
///
/// 用途：窗口类操作（hide/show/set_size/set_position/set_always_on_top）
/// 在非主线程调用会失败（"PostMessage failed ... 无效的窗口句柄"）。
pub fn run_on_main<F>(app: &AppHandle, f: F)
where
    F: FnOnce(&AppHandle) + Send + 'static,
{
    let (tx, rx) = mpsc::channel::<()>();
    // 接收者与闭包必须是两个独立的 AppHandle：闭包 move 捕获 app_cb，
    // 而方法调用借用的接收者不能是被 move 的那个绑定（否则 E0505：
    // cannot move out of `app2` because it is borrowed）。
    let app_recv = app.clone();
    let app_cb = app.clone();
    let _ = app_recv.run_on_main_thread(move || {
        f(&app_cb);
        let _ = tx.send(());
    });
    let _ = rx.recv_timeout(Duration::from_secs(2));
}

/// 在主线程执行闭包（**不等待结果**）。
///
/// 与 `run_on_main` 的区别：不阻塞调用方。因此可以在主线程自身调用
/// （闭包会排在当前事件之后执行，不会自等待死锁）。
pub fn post_to_main<F>(app: &AppHandle, f: F)
where
    F: FnOnce(&AppHandle) + Send + 'static,
{
    let app_cb = app.clone();
    let _ = app.run_on_main_thread(move || f(&app_cb));
}

/// 在主线程设置指定窗口的置顶状态（同步等待）。
/// 用于弹系统对话框前临时取消置顶，避免盖住对话框。
pub fn set_window_topmost(app: &AppHandle, label: &str, topmost: bool) {
    let label = label.to_string();
    run_on_main(app, move |app| {
        if let Some(w) = app.get_webview_window(&label) {
            let _ = w.set_always_on_top(topmost);
        }
    });
}

/// 在主线程设置指定窗口的鼠标穿透状态（同步等待）。
/// WS_EX_TRANSPARENT 需操作窗口句柄，与置顶同理必须在主线程执行。
pub fn set_window_ignore_cursor_events(app: &AppHandle, label: &str, ignore: bool) {
    let label = label.to_string();
    run_on_main(app, move |app| {
        if let Some(w) = app.get_webview_window(&label) {
            let _ = w.set_ignore_cursor_events(ignore);
        }
    });
}

/// 创建贴图窗口（无边框置顶透明窗）。
///
/// 平台差异（重要）：
/// - **Windows**：主线程同步建窗会死锁/白屏，调用方必须放到阻塞线程；
///   这里就是直接的建窗逻辑。
/// - **macOS**：NSWindow 只能在主线程创建，所以这里把建窗投递到主线程并
///   同步等待结果；调用方在工作线程（async 命令）直接调用即可。
#[cfg(target_os = "macos")]
pub fn create_pin_window(
    app: &AppHandle,
    label: &str,
    disp_w: f64,
    disp_h: f64,
    x: f64,
    y: f64,
) -> Result<(), String> {
    let (tx, rx) = mpsc::channel::<Result<(), String>>();
    let app_cb = app.clone();
    let label_cb = label.to_string();
    let _ = app.run_on_main_thread(move || {
        let _ = tx.send(build_pin_window(&app_cb, &label_cb, disp_w, disp_h, x, y));
    });
    rx.recv_timeout(Duration::from_secs(5))
        .unwrap_or_else(|_| Err("create pin window: main thread timeout".to_string()))
}

/// 创建贴图窗口（Windows：由阻塞线程直接调用建窗）。
#[cfg(not(target_os = "macos"))]
pub fn create_pin_window(
    app: &AppHandle,
    label: &str,
    disp_w: f64,
    disp_h: f64,
    x: f64,
    y: f64,
) -> Result<(), String> {
    build_pin_window(app, label, disp_w, disp_h, x, y)
}

/// 真正的建窗逻辑（Windows 由阻塞线程调用，macOS 由主线程调用）。
fn build_pin_window(
    app: &AppHandle,
    label: &str,
    disp_w: f64,
    disp_h: f64,
    x: f64,
    y: f64,
) -> Result<(), String> {
    let w = disp_w.max(8.0) + PIN_MARGIN * 2.0;
    let h = disp_h.max(8.0) + PIN_MARGIN * 2.0;
    WebviewWindowBuilder::new(app, label, WebviewUrl::App("pin.html".into()))
        .title("SnapPin Pin")
        .inner_size(w, h)
        .position(x - PIN_MARGIN, y - PIN_MARGIN)
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .shadow(false)
        .initialization_script(WEBVIEW_GUARD)
        .build()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// 关闭指定贴图窗口。
pub fn close_pin(app: &AppHandle, label: &str) {
    if let Some(w) = app.get_webview_window(label) {
        let _ = w.close();
    }
}

/// 贴图做图像处理后图片尺寸会变（旋转 90° 宽高互换），窗口要跟着变。
///
/// 保持**窗口中心不动**：用户刚旋转完，图片应在原位置原地转过来，
/// 而不是跳到别处（左上角对齐会让图片看起来"往左上跑"）。
/// 留白 PIN_MARGIN 是四周对称的，所以窗口中心 == 图片中心。
///
/// 尺寸按**物理像素**设置（`outer_size` 也是物理像素）；逻辑/物理换算用
/// 窗口自己的 scale_factor，跨显示器拖动后也能算对。
pub fn resize_pin_keep_center(
    app: &AppHandle,
    label: &str,
    disp_w: f64,
    disp_h: f64,
) -> Result<(), String> {
    let Some(w) = app.get_webview_window(label) else {
        return Err("pin window not found".to_string());
    };
    let sf = w.scale_factor().map_err(|e| e.to_string())?;
    let old_pos = w.outer_position().map_err(|e| e.to_string())?;
    let old_size = w.outer_size().map_err(|e| e.to_string())?;

    let new_w = ((disp_w.max(8.0) + PIN_MARGIN * 2.0) * sf).round().max(1.0) as i32;
    let new_h = ((disp_h.max(8.0) + PIN_MARGIN * 2.0) * sf).round().max(1.0) as i32;
    let dx = (old_size.width as i32 - new_w) / 2;
    let dy = (old_size.height as i32 - new_h) / 2;

    let app_cb = app.clone();
    let label_cb = label.to_string();
    let (tx, rx) = mpsc::channel::<Result<(), String>>();
    let _ = app.run_on_main_thread(move || {
        let r = (|| -> Result<(), String> {
            let w = app_cb
                .get_webview_window(&label_cb)
                .ok_or_else(|| "pin window not found".to_string())?;
            w.set_size(tauri::PhysicalSize::new(new_w as u32, new_h as u32))
                .map_err(|e| e.to_string())?;
            w.set_position(PhysicalPosition::new(old_pos.x + dx, old_pos.y + dy))
                .map_err(|e| e.to_string())?;
            Ok(())
        })();
        let _ = tx.send(r);
    });
    rx.recv_timeout(Duration::from_secs(5))
        .unwrap_or_else(|_| Err("resize pin window: main thread timeout".to_string()))?;
    Ok(())
}
/// 结束截图会话：关闭全部浮层 + 清空会话数据。
///
/// 取消（Esc/右键/取消按钮）与确认成功（复制/贴图/保存）后调用。
/// 清空 `state.monitors` 后，各浮层的 `overlay_context` 轮询会得到
/// ready=false，前端检测到 ready 由 true→false 即自关窗口——
/// 即使 Rust 侧漏关某块浮层（如双显示器某块创建异常），
/// 也不会残留"截图模式关不掉"的状态（前端轮询兜底，见 OverlayWindow）。
///
/// 长截图会话（若还在进行）一并丢弃：驱动循环发现会话被取走后自行退出，
/// 浮层的穿透/采集排除状态在此还原（否则下次截图会把浮层自身截进去）。
pub fn end_capture_session(app: &AppHandle, state: &AppState, why: &str) {
    let before = state.monitors.lock().unwrap().len();
    let overlays = app
        .webview_windows()
        .keys()
        .filter(|l| l.starts_with(OVERLAY_PREFIX))
        .count();
    eprintln!("[snappin] end session (why={why}, monitors={before}, overlays={overlays})");
    crate::commands::discard_long_session(app, state, why);
    close_overlays(app);
    state.monitors.lock().unwrap().clear();
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

// ---------------------------------------------------------------- 文档型窗口
// （OCR 结果 / 翻译 / 设置：要选文字、要滚动、要原生标题栏，因此是普通窗口，
//   不是贴图那种无框透明窗）

/// 文档型窗口的 label → HTML 入口映射。
///
/// **必须一处定义、两处使用**：`build_doc_window` 用它取 URL，测试用它校验
/// `dist/` 下文件真实存在。曾经把设置窗 label 从 `main` 改成 `settings`
/// 却忘了建 `settings.html`，窗口去加载一个 404 → **全白窗口**，
/// 且没有任何测试能发现。有了这张表 + 对应测试，这类错直接拦住。
fn doc_html(label: &str) -> Option<&'static str> {
    match label {
        OCR_LABEL => Some("ocr.html"),
        TRANS_LABEL => Some("trans.html"),
        SETTINGS_LABEL => Some("settings.html"),
        _ => None,
    }
}

/// 建「文档型」单例窗口（已存在则直接复用）。
///
/// macOS 的 `WebviewWindowBuilder::build()` 必须在主线程执行，与贴图窗同一套
/// 处理：`build_doc_window` 只负责真正的建窗逻辑，由平台包装函数决定投递。
fn build_doc_window(
    app: &AppHandle,
    label: &str,
    title: &str,
    w: f64,
    h: f64,
) -> Result<(), String> {
    if app.get_webview_window(label).is_some() {
        return Ok(());
    }
    // label 没有对应入口文件时**直接报错**：绝不能建出一个加载 404 的白窗
    let html = doc_html(label).ok_or_else(|| format!("unknown doc window label: {label}"))?;
    let (x, y) = place_centered_on_primary(app, w, h);
    eprintln!(
        "[snappin] doc window build start: label={label} html={html} thread={:?}",
        std::thread::current().id()
    );
    let built = WebviewWindowBuilder::new(app, label, WebviewUrl::App(html.into()))
        .title(title)
        .inner_size(w, h)
        .position(x, y)
        .min_inner_size(560.0, 420.0)
        .initialization_script(WEBVIEW_GUARD)
        .build();
    eprintln!(
        "[snappin] doc window build returned: label={label} ok={:?}",
        built.is_ok()
    );
    built.map(|_| ()).map_err(|e| e.to_string())
}

/// macOS：建窗投递主线程并同步等待（与 create_pin_window 同模式）。
#[cfg(target_os = "macos")]
fn ensure_doc_window(
    app: &AppHandle,
    label: &str,
    title: &str,
    w: f64,
    h: f64,
) -> Result<(), String> {
    let (tx, rx) = mpsc::channel::<Result<(), String>>();
    let app_cb = app.clone();
    let label_cb = label.to_string();
    let title_cb = title.to_string();
    let _ = app.run_on_main_thread(move || {
        let _ = tx.send(build_doc_window(&app_cb, &label_cb, &title_cb, w, h));
    });
    rx.recv_timeout(Duration::from_secs(5))
        .unwrap_or_else(|_| Err("doc window: main thread timeout".to_string()))
}

/// Windows：直接建窗。**调用方必须保证自己在主线程**。
///
/// 曾经试过"一律起工作线程建窗 + 主线程通道等待"，实测死锁：建窗要枚举
/// 显示器（走事件循环线程），而主线程正堵在通道等待上 → 互相等 15 秒超时。
/// 正确做法是调用方用 `run_on_main_thread` 把整段窗口操作投递到主线程
/// （见 `ocr_show` / `trans_start` / `open_settings_window`）。
#[cfg(not(target_os = "macos"))]
fn ensure_doc_window(
    app: &AppHandle,
    label: &str,
    title: &str,
    w: f64,
    h: f64,
) -> Result<(), String> {
    build_doc_window(app, label, title, w, h)
}

/// 显示（必要时创建）一个文档型窗口。
///
/// **刻意不 `set_focus`**：OCR 结果窗 / 翻译窗都是从截图浮层里打开的，
/// 抢了焦点会让浮层收不到键盘（用户按 Esc 取消截图没反应——
/// 事件跑到新窗口去了），也可能让浮层的工具条点不动。
/// 窗口照样显示在最前（show + unminimize），只是不激活。
fn show_doc_window(app: &AppHandle, label: &str, title: &str, w: f64, h: f64) -> Result<(), String> {
    ensure_doc_window(app, label, title, w, h)?;
    if let Some(win) = app.get_webview_window(label) {
        let _ = win.show();
        let _ = win.unminimize();
    }
    Ok(())
}

/// OCR 结果窗（单例）。
pub fn show_ocr_window(app: &AppHandle) -> Result<(), String> {
    // 左图右文双栏：比单栏时宽不少（原 560）。高也放到 700，左栏看图更舒服。
    show_doc_window(app, OCR_LABEL, "SnapPin 文字识别", 880.0, 700.0)
}

/// 翻译窗（单例）。
pub fn show_trans_window(app: &AppHandle) -> Result<(), String> {
    show_doc_window(app, TRANS_LABEL, "SnapPin 翻译", 820.0, 600.0)
}

/// 设置 / 主窗（单例）。
pub fn show_main_window(app: &AppHandle) -> Result<(), String> {
    show_doc_window(app, SETTINGS_LABEL, "SnapPin 设置", 780.0, 640.0)
}

/// 关闭文档型窗口（内容仍留在 state 里，下次打开重取）。
pub fn close_doc_window(app: &AppHandle, label: &str) {
    if let Some(w) = app.get_webview_window(label) {
        let _ = w.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 每个文档型窗口的 HTML 入口必须真实存在于 dist/。
    ///
    /// 这条测试的存在原因：设置窗 label 改名后忘了建 html，用户看到全白窗口。
    /// 集中映射（doc_html）+ 本测试 = 改 label 不改 html 会直接编译期后红灯。
    #[test]
    fn doc_window_html_entries_exist_in_dist() {
        let dist = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../dist");
        for label in [OCR_LABEL, TRANS_LABEL, SETTINGS_LABEL] {
            let html = doc_html(label)
                .unwrap_or_else(|| panic!("label {label} 没有登记 HTML 入口（见 doc_html）"));
            let path = dist.join(html);
            assert!(
                path.exists(),
                "dist/{html} 缺失（窗口 label={label}）——建窗后会得到全白窗口。\
                 先 npm run build 生成入口，或在 doc_html 里改映射"
            );
        }
    }

    /// 映射表不允许出现重复 html（两个窗口共用入口会互相顶内容）。
    #[test]
    fn doc_html_mapping_is_unique() {
        let all = [doc_html(OCR_LABEL), doc_html(TRANS_LABEL), doc_html(SETTINGS_LABEL)];
        let mut seen = std::collections::HashSet::new();
        for h in all.into_iter().flatten() {
            assert!(seen.insert(h), "html 入口重复：{h}");
        }
    }
}
