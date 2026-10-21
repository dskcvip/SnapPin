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

/// 旧版留白（10px）。仅用于把历史配置里"窗口左上角"换算成球心，
/// 避免窗口变大后悬浮球在屏幕上平移 14px。
const BALL_MARGIN_LEGACY: f64 = 10.0;

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

    // 位置是**球心**（物理像素）：窗口留白变化时球不会在屏幕上平移。
    // 历史配置存的是窗口左上角（旧留白 10px），这里换算一次。
    //
    // 两个坑：
    // 1) Windows 有最小跟踪宽度（实测 136px），小于它的窗口会被**加宽**成非正方形，
    //    所以半宽/半高必须按实际 inner_size 分别取，否则球心横向偏 (实宽-目标宽)/2。
    // 2) 这次"加宽 + 重新放置"发生在 build() 之后，紧接着的 set_position 会被它覆盖
    //    （现象：球固定跳回主屏中部）。所以等窗口真正 show 完成后，再按最终尺寸定位一次。
    if let Some(p) = cfg.ball_position {
        let sf = window.scale_factor().unwrap_or(1.0);
        let (cx, cy) = if cfg.ball_pos_center {
            (p.x, p.y)
        } else {
            let legacy_half = (size + BALL_MARGIN_LEGACY * 2.0) * sf / 2.0;
            (p.x + legacy_half, p.y + legacy_half)
        };
        place_ball_center(&window, cx, cy, "immediate");

        let win_handle = window.clone();
        std::thread::spawn(move || {
            // 窗口从 build() 到真正 show 之间会被系统重放置（最小尺寸修正），
            // 期间任何 set_position 都可能被覆盖，所以分几拍再落一次，最后一拍为准。
            for (delay, tag) in [(250u64, "t+250"), (900, "t+900")] {
                std::thread::sleep(std::time::Duration::from_millis(delay));
                place_ball_center(&win_handle, cx, cy, tag);
            }
        });
    }
    Ok(())
}

/// 按球心放置悬浮球窗口（内部按实际 inner_size 取半宽半高）。
fn place_ball_center<R: tauri::Runtime>(
    window: &tauri::WebviewWindow<R>,
    cx: f64,
    cy: f64,
    tag: &str,
) {
    let actual = window.inner_size().unwrap_or_default();
    if actual.width == 0 || actual.height == 0 {
        return;
    }
    let nx = (cx - actual.width as f64 / 2.0).round() as i32;
    let ny = (cy - actual.height as f64 / 2.0).round() as i32;
    if let Err(e) = window.set_position(PhysicalPosition::new(nx, ny)) {
        eprintln!("[snappin] ball position ({tag}) failed: {e}");
        return;
    }
    let got = window
        .outer_position()
        .map(|p| format!("({},{})", p.x, p.y))
        .unwrap_or_else(|_| "?".into());
    eprintln!(
        "[snappin] ball positioned ({tag}): want=({nx},{ny}) got={got} center=({cx:.0},{cy:.0}) size={}x{}",
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
