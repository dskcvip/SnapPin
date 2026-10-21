//! 自绘菜单窗（托盘图标点击 / 悬浮球右键 / 贴图右键）。
//!
//! ## 菜单种类
//!
//! 同一个菜单窗服务三种入口（单例复用，内容由前端按 `menu-open` 载荷的
//! `kind` 切换）：`app` = 托盘/悬浮球（应用级动作）；`pin` = 贴图右键
//! （动作由 Rust 转发给对应贴图窗，载荷带 `pinLabel`）。
//!
//! ## 为什么不用系统原生菜单
//!
//! 原生菜单（`WebviewWindow::popup_menu` → Win32 `TrackPopupMenu`）的样式完全由
//! 系统决定：左侧不能放勾选、右侧不能放快捷键提示、圆角与间距都不受控。
//! 要做到"浅色圆角面板 + 勾选列 + 右侧快捷键提示 + 分组分隔线"，
//! 只能自己用一个**透明无边框的 webview 窗口**渲染菜单。
//!
//! ## 生命周期（三段式，避免尺寸跳动）
//!
//! 1. `open_menu_at`：记录锚点（物理屏幕坐标）→ 窗口移出屏幕 → emit `menu-open`；
//! 2. 前端渲染菜单、量出自己的 `offsetWidth/offsetHeight` → `menu_fit(w, h)`；
//! 3. `fit_and_show`：按锚点算位置（含屏幕边界翻转）→ resize → 移到锚点 → 抢焦点。
//!
//! 之所以先量后显：菜单高度依赖字体渲染，后端猜的尺寸必然差几像素，
//! 直接显示会看到一次抖动。
//!
//! ## 为什么"关闭"是移出屏幕而不是 `hide()`
//!
//! WebView2 里**不可见窗口的 `requestAnimationFrame` 会被暂停/节流**，
//! 隐藏着的菜单窗收不到 `menu-open` 后的下一帧，测量就永远不发生。
//! 所以窗口自始至终保持 `visible`，只是把位置挪到 (-32000, -32000)：
//! 对用户完全不可见，页面却照常渲染。代价是它始终是一个"可见窗口"，
//! 因此关闭判定不能再用 `is_visible()`——统一走"有没有锚点"。
//!
//! ## 关闭
//!
//! - **失焦即关**：用户点桌面/别的窗口时 webview 收到 `Focused(false)` → park。
//!   这是自绘菜单唯一的"点外部关闭"来源（窗口必须先拿到焦点）。
//! - `Esc` / 点到面板外的透明留白：前端调 `menu_close`（见 MenuWindow.tsx）。

use crate::state::{AppState, MenuAnchor, MenuKind};
use crate::windows::WEBVIEW_GUARD;
use tauri::webview::WebviewWindowBuilder;
use tauri::{AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, WebviewUrl, WindowEvent};

/// 菜单窗 label（单例，反复复用，不销毁）。
pub const MENU_LABEL: &str = "ball-menu";

/// 菜单窗相对面板的留白（逻辑像素）：容纳 CSS 的外阴影。
///
/// **必须与 `src/styles/menu.css` 的 `.menu-stage` padding 以及
/// `MenuWindow.tsx` 的 `SHADOW_PAD` 一致**，否则面板会贴到窗口边缘、
/// 阴影被裁成直角。
pub const MENU_SHADOW_PAD: f64 = 18.0;

/// 初始尺寸（逻辑像素）：只为建窗时有个合理大小，真实尺寸由前端量出后 resize。
const INIT_W: f64 = 268.0;
const INIT_H: f64 = 332.0;

/// "收起"位置：远超任何显示器的坐标（Windows 虚拟屏幕约 ±32767）。
const PARK_X: i32 = -32000;
const PARK_Y: i32 = -32000;

/// 懒创建菜单窗（已存在则直接返回）。创建后立即 park（屏幕外）。
/// 建窗/移窗是否必须由主线程执行。
///
/// - macOS：`WebviewWindowBuilder::build()` 与 NSWindow 的 resize/show 必须在主线程；
/// - Windows：主线程同步建窗会**死锁**（见 `windows::create_pin_window` 文档），
///   必须留在调用方的阻塞线程里做。
///
/// 用常量而不是 `#[cfg]` 分支的原因：`#[cfg]` 会把这段只在 mac 上运行的代码
/// 从 Windows 的编译里整块裁掉，从而**永远得不到编译检查**（API 改名、
/// 类型不匹配都要等 CI/mac 构建才炸）。运行时分支两边都会被编译。
#[cfg(target_os = "macos")]
const MENU_NEEDS_MAIN_THREAD: bool = true;
#[cfg(not(target_os = "macos"))]
const MENU_NEEDS_MAIN_THREAD: bool = false;

/// 取（必要时创建）菜单窗，处理平台建窗线程约束。
///
/// 已存在的窗口直接返回——`get_webview_window` 只是查句柄表，任意线程都安全。
/// 只有**真正建窗**那一步需要按平台分流：
///
/// - **macOS 必须主线程**：`WebviewWindowBuilder::build()` 要创建 NSWindow 与
///   WKWebView，在阻塞线程调用会失败（Tauri 明确要求主线程）。三个入口
///   （托盘点击 / 贴图右键命令 / 悬浮球）都在 `spawn_blocking` 里，
///   不投递的话**菜单一次都弹不出来**——表现为"点托盘毫无反应"。
/// - **Windows/其他平台留在调用线程**：主线程同步建窗会死锁
///   （见 `windows::create_pin_window` 的注释）。
///
/// 为什么只包建窗、不包整段 open_menu：`Window::set_size/set_position/show`
/// 这些方法在 Tauri 内部本来就会自己派发到主线程，外层再套一层投递会变成
/// "主线程等主线程"的嵌套等待，反而有死锁风险。
fn ensure_menu_window_threaded(app: &AppHandle) -> Result<tauri::WebviewWindow, String> {
    if let Some(w) = app.get_webview_window(MENU_LABEL) {
        return Ok(w);
    }
    if MENU_NEEDS_MAIN_THREAD {
        let (tx, rx) = std::sync::mpsc::channel::<Result<tauri::WebviewWindow, String>>();
        let app_cb = app.clone();
        app.run_on_main_thread(move || {
            let _ = tx.send(ensure_menu_window(&app_cb));
        })
        .map_err(|e| format!("menu window dispatch failed: {e}"))?;
        return rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap_or_else(|_| Err("menu window: main thread timeout".to_string()));
    }
    ensure_menu_window(app)
}

fn ensure_menu_window(app: &AppHandle) -> Result<tauri::WebviewWindow, String> {
    if let Some(w) = app.get_webview_window(MENU_LABEL) {
        return Ok(w);
    }

    let window = WebviewWindowBuilder::new(app, MENU_LABEL, WebviewUrl::App("menu.html".into()))
        .title("SnapPin Menu")
        .inner_size(INIT_W, INIT_H)
        .position(PARK_X as f64, PARK_Y as f64)
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .shadow(false)
        .focused(false)
        // 保持可见（见模块文档：隐藏会让 rAF 暂停），靠屏幕外坐标藏起来
        .visible(true)
        .initialization_script(WEBVIEW_GUARD)
        .build()
        .map_err(|e| format!("menu window build failed: {e}"))?;

    // 失焦即关（点外部）。判据用锚点而不是 is_visible——窗口一直"可见"。
    let handle = window.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::Focused(false) = event {
            let app = handle.app_handle();
            if menu_is_open(app) {
                park_menu(app);
                crate::logln!("menu closed (focus lost)");
            }
        }
    });

    let _ = window.set_position(PhysicalPosition::new(PARK_X, PARK_Y));
    Ok(window)
}

/// 把菜单窗移到屏幕外并清锚点（= 关闭）。
fn park_menu(app: &AppHandle) {
    if let Some(state) = app.try_state::<AppState>() {
        *state.menu_anchor.lock().unwrap() = None;
    }
    if let Some(w) = app.get_webview_window(MENU_LABEL) {
        let _ = w.set_position(PhysicalPosition::new(PARK_X, PARK_Y));
    }
}

/// `menu-open` 事件载荷：前端据此渲染对应种类的菜单。
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct MenuOpenPayload {
    /// `app` = 托盘/悬浮球菜单；`pin` = 贴图右键菜单
    kind: &'static str,
    /// 贴图菜单的目标窗口 label（kind = pin 时非空）
    pin_label: Option<String>,
}

/// 在鼠标光标处打开菜单（托盘点击、悬浮球右键）——应用级菜单。
pub fn open_menu_at_cursor(app: &AppHandle) -> Result<(), String> {
    let (px, py) = snappin_core_platform::cursor_position()
        .ok_or_else(|| "cursor position unavailable".to_string())?;
    open_menu(app, px as f64, py as f64, MenuKind::App, None)
}

/// 在鼠标光标处打开**贴图右键菜单**（作用于 label 指定的贴图窗）。
pub fn open_pin_menu_at_cursor(app: &AppHandle, label: &str) -> Result<(), String> {
    let (px, py) = snappin_core_platform::cursor_position()
        .ok_or_else(|| "cursor position unavailable".to_string())?;
    open_menu(app, px as f64, py as f64, MenuKind::Pin, Some(label.to_string()))
}

/// 在指定物理坐标打开菜单。尺寸待前端 `menu_fit` 到达后才移到锚点。
///
/// **线程约束（macOS）**：菜单窗是懒创建的，第一次点击就要
/// `WebviewWindowBuilder::build()`，而 macOS 要求建窗在主线程。三个入口
/// （托盘点击 / 贴图右键命令 / 悬浮球）都在 `spawn_blocking` 线程里调用本函数，
/// 直接用 build() 会失败 —— 表现就是**点托盘图标毫无反应**（菜单一次都弹不出来）。
/// 因此 macOS 上把整段逻辑投递到主线程并同步等待结果，与
/// `windows::create_pin_window` 同一套做法。
pub fn open_menu(
    app: &AppHandle,
    px: f64,
    py: f64,
    kind: MenuKind,
    pin_label: Option<String>,
) -> Result<(), String> {
    let window = ensure_menu_window_threaded(app)?;
    // 先离开屏幕：上一次的尺寸/位置不能露出来（也避免在旧位置闪一帧）
    park_menu(app);
    // 重新写锚点（park_menu 会清空）
    {
        let state = app.state::<AppState>();
        *state.menu_anchor.lock().unwrap() = Some(MenuAnchor {
            x: px,
            y: py,
            kind,
            pin_label: pin_label.clone(),
        });
    }
    let payload = MenuOpenPayload {
        kind: match kind {
            MenuKind::App => "app",
            MenuKind::Pin => "pin",
        },
        pin_label,
    };
    window
        .emit("menu-open", payload)
        .map_err(|e| format!("emit menu-open failed: {e}"))?;
    crate::logln!("menu open requested at ({px:.0}, {py:.0}) kind={kind:?}");
    Ok(())
}

/// 按前端量出的窗口尺寸定位并显示菜单。
///
/// 同样受 macOS 主线程约束（resize / move / show / set_focus 都在这里）。
pub fn fit_and_show(app: &AppHandle, w: f64, h: f64) -> Result<(), String> {
    let anchor = {
        let state = app.state::<AppState>();
        // 先取出值再释放 guard：直接让 `*guard` 作为块尾表达式会让临时
        // MutexGuard 比 state 活得久，借用检查器不认。
        let guard = state.menu_anchor.lock().unwrap();
        guard.as_ref().map(|a| (a.x, a.y))
    };
    // 锚点已清空说明菜单在测量期间被关掉了，不显示
    let Some((ax, ay)) = anchor else {
        return Ok(());
    };

    let window = app
        .get_webview_window(MENU_LABEL)
        .ok_or_else(|| "menu window missing".to_string())?;
    let (wx, wy) = compute_window_position(app, ax, ay, w, h)?;

    window
        .set_size(LogicalSize::new(w, h))
        .map_err(|e| format!("menu resize failed: {e}"))?;
    window
        .set_position(PhysicalPosition::new(wx, wy))
        .map_err(|e| format!("menu move failed: {e}"))?;
    window
        .show()
        .map_err(|e| format!("menu show failed: {e}"))?;
    // 必须拿到焦点：既让 Esc 生效，也让"点外部"能触发失焦关闭
    if let Err(e) = window.set_focus() {
        crate::logln!("menu set_focus failed: {e}");
    }
    crate::logln!("menu shown at ({wx}, {wy}) size={w:.0}x{h:.0}");
    Ok(())
}

/// 计算菜单窗的物理左上角：默认贴光标右下，越界则左/上翻转，最后夹紧到显示器内。
fn compute_window_position(
    app: &AppHandle,
    ax: f64,
    ay: f64,
    w: f64,
    h: f64,
) -> Result<(i32, i32), String> {
    let monitor = app
        .monitor_from_point(ax, ay)
        .ok()
        .flatten()
        .or_else(|| app.primary_monitor().ok().flatten())
        .ok_or_else(|| "no monitor for menu".to_string())?;

    let sf = monitor.scale_factor();
    let (mx, my) = (monitor.position().x as f64, monitor.position().y as f64);
    let (mw, mh) = (monitor.size().width as f64, monitor.size().height as f64);

    let pad = MENU_SHADOW_PAD * sf;
    let panel_w = w * sf - 2.0 * pad;
    let panel_h = h * sf - 2.0 * pad;
    // 光标与面板之间留一点点缝，避免菜单紧贴指针
    const GAP: f64 = 2.0;

    let mut panel_x = ax + GAP;
    let mut panel_y = ay + GAP;
    if panel_x + panel_w > mx + mw {
        panel_x = ax - GAP - panel_w;
    }
    if panel_y + panel_h > my + mh {
        panel_y = ay - GAP - panel_h;
    }
    // 极端情况（菜单比屏幕还高/宽）夹紧到左上角，保证可读
    panel_x = panel_x.max(mx);
    panel_y = panel_y.max(my);

    Ok((
        (panel_x - pad).round() as i32,
        (panel_y - pad).round() as i32,
    ))
}

/// 关闭菜单（Esc / 执行菜单项后）。
pub fn hide_menu(app: &AppHandle) {
    park_menu(app);
}

/// 菜单是否处于打开流程中（前端挂载后据此决定要不要马上测量显示）。
pub fn menu_is_open(app: &AppHandle) -> bool {
    app.try_state::<AppState>()
        .map(|s| s.menu_anchor.lock().unwrap().is_some())
        .unwrap_or(false)
}
