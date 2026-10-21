mod commands;
mod config;
mod logfile;
mod menuwin;
mod state;
mod windows;

use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::GlobalShortcutExt;
use tauri_plugin_global_shortcut::{Code, Modifiers, Shortcut, ShortcutState};

use crate::state::AppState;

/// 构造长截图专用的 Esc 快捷键（无修饰键）。
/// 仅在长截图进行期间动态注册，结束即注销（见 commands::register_long_esc）。
pub fn long_esc_shortcut() -> Shortcut {
    Shortcut::new(None, Code::Escape)
}

/// 内置默认快捷键：截图（Ctrl+1）。配置里可改，这里只做默认值。
pub fn default_capture_shortcut() -> Shortcut {
    Shortcut::new(Some(Modifiers::CONTROL), Code::Digit1)
}

/// 内置默认快捷键：剪贴板贴图（Ctrl+2）。
pub fn default_pin_shortcut() -> Shortcut {
    Shortcut::new(Some(Modifiers::CONTROL), Code::Digit2)
}

pub fn run() {
    // 最先初始化日志：release 无控制台，所有 eprintln! 靠它落盘才有取证价值
    logfile::init();

    // 截图/贴图快捷键由配置驱动（默认 Ctrl+1 / Ctrl+2，见 default_*_shortcut）；
    // Esc 只在长截图期间动态注册。
    let esc_sc = long_esc_shortcut();

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // 第二个实例启动时唤出悬浮球，而不是开新进程
            if let Some(w) = app.get_webview_window(windows::BALL_LABEL) {
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler({
                    let esc_sc = esc_sc;
                    move |app, shortcut, event| {
                        // 插件对**按下**和**抬起**各回调一次，只认 Pressed。
                        // 否则一次按键会执行两遍：截图侧表现为「start 紧接 busy」，
                        // 其它动作也会被重复执行两次。
                        if event.state != ShortcutState::Pressed {
                            return;
                        }
                        // 埋点：确认插件回调真的进来了（排障用，频率低）
                        crate::logln!("hotkey pressed: {:?}", shortcut);
                        // 与设置窗共用同一份"当前生效键"：用户在设置里改键后
                        // update_config 会更新它，这里立即按新键分发。
                        let state = app.state::<AppState>();
                        let (capture_sc, pin_sc) = {
                            let g = state.hotkeys.lock().unwrap();
                            (g.0.clone(), g.1.clone())
                        };
                        if shortcut == &capture_sc {
                            handle_capture(app, "hotkey");
                        } else if shortcut == &pin_sc {
                            handle_pin_clipboard(app);
                        } else if shortcut == &esc_sc {
                            // Esc 只在长截图期间注册：命中即"停止滚动"。
                            // 用 long_active 快照判断（不取 long 锁——
                            // 主线程不能阻塞在驱动循环持的锁上）。
                            let state = app.state::<AppState>();
                            if state.long_active.load(std::sync::atomic::Ordering::SeqCst) {
                                state
                                    .long_stop
                                    .store(true, std::sync::atomic::Ordering::SeqCst);
                                eprintln!("[snappin] long capture stop requested (Esc)");
                            }
                        }
                    }
                })
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::new())
        .setup(move |app| {
            // macOS 专属启动配置（必须放在 `app.handle()` 之前：
            // set_activation_policy 需要 &mut App，handle 会先借走不可变借用）
            // 屏幕录制权限（仅 macOS 有意义）：未授权时系统不会报错，只是把
            // 其他应用的窗口从采集结果里过滤掉 —— 用户看到的是"截图后所有窗口都不见了"。
            // 这里只做一次状态探测（必要时触发系统授权框），真正的拦截在截图入口
            // （commands::mac_capture_gate），探测结果同时带进托盘 tooltip。
            #[cfg(target_os = "macos")]
            let mac_perm_ok = {
                // 纯后台常驻：不占 Dock 图标（截图工具的常规形态，同 PixPin/Shottr）
                app.set_activation_policy(tauri::ActivationPolicy::Accessory);
                let ok = snappin_core_platform::ensure_screen_capture_permission();
                // 无论成败都记一行：这是判断"截出空桌面"到底是权限问题还是采集问题的
                // 唯一依据（菜单 → 打开运行日志）。
                crate::logln!("[snappin] 屏幕录制权限 preflight = {ok}");
                if !ok {
                    crate::logln!(
                        "[snappin] 未获得「屏幕录制」权限：系统设置 → 隐私与安全性 → \
                         屏幕录制 → 勾选 SnapPin，然后重启应用（macOS 要求重启才生效）"
                    );
                }
                ok
            };
            #[cfg(not(target_os = "macos"))]
            let mac_perm_ok = true;

            let handle = app.handle();

            // 关掉 Tauri 自动创建的默认主窗口（延后 300ms：setup 执行时它还没建）。
            //
            // 本项目是多入口 SPA（ball/pin/menu/main/ocr/trans 六个 HTML），
            // 没有 index.html。Tauri 的默认行为是建一个 label="main" 的窗口并
            // 加载 index.html，加载失败就表现为**黑框 +「无法访问此页面」**，
            // 每次启动都弹，用户还以为那是悬浮球。
            // 注意：`tauri.conf.json` 的 `app.windows = []` 挡不住它（空数组被当作
            // "未配置"），所以在这里按 label 白名单清理。
            {
                let handle_bg = handle.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(300));
                    for (label, win) in handle_bg.webview_windows() {
                        let ours = label == windows::BALL_LABEL
                            || label.starts_with(windows::OVERLAY_PREFIX)
                            || label.starts_with(windows::PIN_PREFIX)
                            || label == "menu"
                            || label == windows::OCR_LABEL
                            || label == windows::TRANS_LABEL
                            || label == windows::SETTINGS_LABEL;
                        if !ours {
                            crate::logln!("closing unexpected default window: {label}");
                            let _ = win.close();
                        }
                    }
                });
            }

            // 加载（并落一次）配置
            let config_dir = handle.path().app_config_dir().map_err(|e| e.to_string())?;
            let cfg = config::load(&config_dir);
            let _ = config::save(&config_dir, &cfg);
            {
                let state = handle.state::<AppState>();
                *state.config.lock().unwrap() = cfg.clone();
            }

            // 注册全局快捷键（须在主线程；setup 即在主线程）。
            // 注册失败（如组合键已被其他程序 RegisterHotKey 占用）**不阻断启动**，
            // 但必须让用户看得见：否则表现为"按 Ctrl+1 完全没反应"，
            // 用户只会以为什么都没发生。收集失败列表 → 写进托盘 tooltip。
            // 配置里的快捷键优先；解析不了（手改 config.json 改错了）回退默认值，
            // 并记进日志——不能静默用默认值，否则用户以为自己的键生效了。
            let capture_sc = commands::parse_shortcut(&cfg.shortcut_capture)
                .unwrap_or_else(|| {
                    crate::logln!(
                        "[snappin] shortcut_capture “{}” 无法解析，回退 Ctrl+1",
                        cfg.shortcut_capture
                    );
                    default_capture_shortcut()
                });
            let pin_sc = commands::parse_shortcut(&cfg.shortcut_pin_clipboard)
                .unwrap_or_else(|| {
                    crate::logln!(
                        "[snappin] shortcut_pin_clipboard “{}” 无法解析，回退 Ctrl+2",
                        cfg.shortcut_pin_clipboard
                    );
                    default_pin_shortcut()
                });
            {
                let state = handle.state::<AppState>();
                *state.hotkeys.lock().unwrap() = (capture_sc.clone(), pin_sc.clone());
            }
            let mut sc_failed: Vec<&'static str> = Vec::new();
            if let Err(e) = handle.global_shortcut().register(capture_sc.clone()) {
                crate::logln!("[snappin] register {} FAILED: {e}", cfg.shortcut_capture);
                sc_failed.push("截图快捷键");
            } else {
                crate::logln!("[snappin] {} registered", cfg.shortcut_capture);
            }
            if let Err(e) = handle.global_shortcut().register(pin_sc.clone()) {
                crate::logln!("[snappin] register {} FAILED: {e}", cfg.shortcut_pin_clipboard);
                sc_failed.push("贴图快捷键");
            } else {
                crate::logln!("[snappin] {} registered", cfg.shortcut_pin_clipboard);
            }

            // 悬浮球窗口
            windows::ensure_ball_window(&handle, &cfg)?;

            // 菜单事件统一由托盘 on_menu_event 分发（见 setup_tray）。
            // 注意：**不要**再注册 ball.on_menu_event——Tauri 2 的菜单事件
            // （EventLoopMessage::MenuEvent）是全网广播：先遍历 App 级
            // global_event_listeners（托盘 on_menu_event 在此注册），
            // 再遍历各窗口 event_listeners。两处都注册会导致每次菜单点击
            // 触发两次 handle_menu_event（曾致"关于"打开两个网页）。

            // 悬浮球移动后持久化位置（节流：位移超过 1px 才写盘）
            // 保存**球心**（窗口左上角 + 半边长）：窗口留白随阴影需求变化，
            // 若存左上角，改留白后球会在屏幕上整体平移。
            if let Some(ball) = handle.get_webview_window(windows::BALL_LABEL) {
                // Windows 会给窗口加最小宽度（实测 136x39），宽高可能不等，
                // 所以半宽半高要分别取，不能假设正方形。
                let (half_w, half_h) = ball
                    .inner_size()
                    .map(|s| (s.width as f64 / 2.0, s.height as f64 / 2.0))
                    .unwrap_or((0.0, 0.0));
                let persist_handle = handle.clone();
                ball.on_window_event(move |event| {
                    if let tauri::WindowEvent::Moved(pos) = event {
                        let state = persist_handle.state::<AppState>();
                        let mut guard = state.config.lock().unwrap();
                        let center = config::BallPosition {
                            x: pos.x as f64 + half_w,
                            y: pos.y as f64 + half_h,
                        };
                        // ball_pos_center=false 说明是旧配置（存的是左上角），必须改写一次
                        let changed = !guard.ball_pos_center
                            || match guard.ball_position {
                                Some(p) => {
                                    (p.x - center.x).abs() > 1.0 || (p.y - center.y).abs() > 1.0
                                }
                                None => true,
                            };
                        if changed {
                            guard.ball_position = Some(center);
                            guard.ball_pos_center = true;
                            let snapshot = guard.clone();
                            drop(guard);
                            if let Ok(dir) = persist_handle.path().app_config_dir() {
                                let _ = config::save(&dir, &snapshot);
                            }
                        }
                    }
                });
            }

            // 失败清单同时存进状态：自绘菜单顶部据此显示告警行
            {
                let state = handle.state::<AppState>();
                *state.shortcut_failed.lock().unwrap() =
                    sc_failed.iter().map(|s| s.to_string()).collect();
            }
            setup_tray(&handle, &sc_failed, mac_perm_ok)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::start_capture,
            commands::overlay_context,
            commands::confirm_selection,
            commands::cancel_capture,
            commands::ui_log,
            commands::copy_text,
            commands::open_url,
            commands::decode_qr,
            commands::ocr_models_status,
            commands::ocr_recognize,
            commands::long_capture_start,
            commands::long_capture_stop,
            commands::long_capture_commit,
            commands::pin_from_clipboard,
            commands::pin_context,
            commands::pin_close,
            commands::pin_set_topmost,
            commands::pin_set_ignore_cursor_events,
            commands::pin_copy,
            commands::pin_save,
            commands::pin_apply_edit,
            commands::ball_command,
            commands::ball_menu,
            commands::pin_menu_open,
            commands::read_image_bytes,
            commands::pin_publish_flags,
            commands::pin_flags,
            commands::pin_set_opacity,
            commands::menu_fit,
            commands::menu_close,
            commands::menu_state,
            commands::menu_action,
            commands::drag_window,
            commands::get_config,
            commands::update_config,
            commands::app_info,
            commands::ocr_show,
            commands::ocr_result,
            commands::ocr_close,
            commands::trans_start,
            commands::trans_test,
            commands::trans_state,
            commands::trans_close,
            commands::open_settings_window,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run SnapPin");
}

fn handle_capture(app: &AppHandle, src: &str) {
    crate::logln!("handle_capture: src={src}");
    // 防重入：已有截图浮层时忽略（含长按重复触发）
    if app
        .webview_windows()
        .keys()
        .any(|l| l.starts_with(windows::OVERLAY_PREFIX))
    {
        eprintln!("[snappin] handle_capture ignored (src={src}, overlay exists)");
        return;
    }
    // 事件处理器在主线程执行：创建 webview 必须放阻塞线程（Windows 已知问题）
    let app = app.clone();
    let src = src.to_string();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        if let Err(e) = commands::do_capture(&app, &state, &src) {
            eprintln!("[snappin] capture failed (src={src}): {e}");
        }
    });
}

fn handle_pin_clipboard(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        if let Err(e) = commands::do_pin_from_clipboard(&app, &state) {
            eprintln!("[snappin] pin from clipboard failed: {e}");
        }
    });
}

/// 建托盘图标。
///
/// **不挂原生菜单**：点击托盘图标改为弹出自绘菜单窗（menuwin.rs），
/// 这样才能做出与设计稿一致的深色圆角面板。注意托盘一旦没有 menu，
/// Windows 就不会再自己弹任何菜单，左右键都走 `on_tray_icon_event`。
///
/// `sc_failed`：注册失败的内置快捷键名（如 `["Ctrl+1"]`）。非空时 tooltip
/// 改告警文案，并在自绘菜单顶部显示一条提示——热键注册失败只靠这个让用户
/// 看见，否则表现就是"按键毫无反应"。
fn setup_tray(
    app: &AppHandle,
    sc_failed: &[&'static str],
    mac_perm_ok: bool,
) -> tauri::Result<()> {
    use tauri::tray::{MouseButtonState, TrayIconBuilder, TrayIconEvent};

    // tooltip 是用户唯一常驻可见的状态位：热键被占用、以及（macOS）缺屏幕录制
    // 权限都必须写在这里，否则用户只会觉得"这软件坏了"。
    let mut tips: Vec<String> = Vec::new();
    if !sc_failed.is_empty() {
        tips.push(format!(
            "全局快捷键 {} 注册失败（已被其他程序占用）",
            sc_failed.join(" / ")
        ));
    }
    if !mac_perm_ok {
        tips.push("未获得「屏幕录制」权限：只能截到桌面壁纸".to_string());
    }
    let tooltip = if tips.is_empty() {
        "SnapPin".to_string()
    } else {
        format!("SnapPin — {}", tips.join("；"))
    };

    #[allow(unused_mut)]
    let mut builder = TrayIconBuilder::new()
        .icon(tauri::include_image!("icons/32x32.png"))
        .tooltip(tooltip)
        // 点击（左键或右键）抬起时弹自绘菜单。
        // 抬起而非按下：与系统托盘交互惯例一致，也避免按住拖动误触发。
        .on_tray_icon_event(|tray, event| match event {
            TrayIconEvent::Click {
                button,
                button_state,
                ..
            } => {
                crate::logln!("tray click: {button:?} / {button_state:?}");
                if button_state == MouseButtonState::Up {
                    let app = tray.app_handle().clone();
                    // 建 webview 不能在事件处理器（主线程）里同步做
                    tauri::async_runtime::spawn_blocking(move || {
                        if let Err(e) = crate::menuwin::open_menu_at_cursor(&app) {
                            crate::logln!("tray menu open failed: {e}");
                        }
                    });
                }
            }
            TrayIconEvent::DoubleClick { button, .. } => {
                crate::logln!("tray double-click: {button:?}")
            }
            _ => {}
        });

    // macOS 兜底：**右键**走系统原生 NSMenu，左键仍然弹自绘菜单
    // （`show_menu_on_left_click(false)` 把左键让给上面的点击事件）。
    //
    // 为什么必须留这条路：本应用是 Accessory（无 Dock 图标、无 ⌘Q），自绘菜单一旦
    // 建不出来（WKWebView 建窗失败、主线程投递超时……），用户就**没有任何退出入口**，
    // 只能去命令行 kill —— 曾经真实发生过（见 docs/BUILD-macos.md 的"无法退出"条目）。
    // 原生菜单由系统自己画，不依赖我们的 webview / 主线程 / 前端，因此永远点得动。
    #[cfg(target_os = "macos")]
    {
        let menu = mac_native_tray_menu(app)?;
        // 菜单须与应用同生命周期：与托盘句柄一样故意泄漏
        let menu: &'static tauri::menu::Menu<tauri::Wry> = Box::leak(Box::new(menu));
        builder = builder.menu(menu).show_menu_on_left_click(false);
    }

    let tray = builder.build(app)?;

    // 托盘图标须与应用同生命周期：故意泄漏句柄，避免 drop 后图标消失
    std::mem::forget(tray);
    Ok(())
}

/// macOS 右键兜底菜单（系统原生绘制）。
///
/// 只放**不可缺失**的动作：截图、贴图、退出、排障入口。视觉上不与自绘菜单
/// 保持一致是刻意的取舍——这一份的职责是"永远能用"，不是"好看"。
#[cfg(target_os = "macos")]
fn mac_native_tray_menu(app: &AppHandle) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};

    let capture = MenuItem::with_id(app, "capture", "截图", true, None::<&str>)?;
    let pin = MenuItem::with_id(app, "pin-clipboard", "剪贴板贴图", true, None::<&str>)?;
    let ball = MenuItem::with_id(app, "toggle-ball", "显示 / 隐藏悬浮球", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "设置…", true, None::<&str>)?;
    let perm = MenuItem::with_id(app, "open-perm", "屏幕录制权限…", true, None::<&str>)?;
    let log = MenuItem::with_id(app, "open-log", "打开运行日志", true, None::<&str>)?;
    let about = MenuItem::with_id(app, "about", "关于 SnapPin", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出 SnapPin", true, None::<&str>)?;
    let s1 = PredefinedMenuItem::separator(app)?;
    let s2 = PredefinedMenuItem::separator(app)?;

    Menu::with_items(
        app,
        &[
            &capture,
            &pin,
            &ball,
            &perm,
            &s1,
            &log,
            &settings,
            &about,
            &s2,
            &quit,
        ],
    )
}

/// 菜单动作分发（单一事实来源）。
///
/// 托盘图标、悬浮球右键、自绘菜单窗的点击最终都汇到这里，
/// 保证"同一个菜单项不管从哪进来，行为一致"。
pub(crate) fn handle_menu_event(app: &AppHandle, id: &str) {
    crate::logln!("menu item activated: id={id}");
    match id {
        "capture" => handle_capture(app, "menu"),
        "pin-clipboard" => handle_pin_clipboard(app),
        "hide-ball" => {
            if let Some(w) = app.get_webview_window(windows::BALL_LABEL) {
                let _ = w.hide();
            }
        }
        "toggle-ball" => windows::toggle_ball(app),
        "about" => open_external(ABOUT_URL),
        // 排障入口：直接打开落盘日志（release 无控制台，这是唯一的现场）
        "open-log" => open_external(&logfile::log_path().to_string_lossy()),
        // 设置窗（翻译 Key、快捷键、外观都在这）
        "settings" => {
            if let Err(e) = windows::show_main_window(app) {
                crate::logln!("open settings window failed: {e}");
            }
        }
        // 换包后授权失效时，用户唯一的自助恢复入口（macOS）
        "open-perm" => open_external(SCREEN_RECORDING_SETTINGS_URL),
        "quit" => app.exit(0),
        _ => {}
    }
}

/// "关于..." 跳转的项目仓库地址。
pub const ABOUT_URL: &str = "https://gitee.com/dskcvip/snappin";

/// macOS「隐私与安全性 → 屏幕录制」面板的深链（`open` 可直接识别）。
pub const SCREEN_RECORDING_SETTINGS_URL: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture";

/// 用系统默认浏览器打开外部链接。
///
/// 不引 tauri-plugin-shell：本项目要求"全本地、最小依赖"，
/// 直接用各平台原生命令即可（Windows cmd start / macOS open / Linux xdg-open）。
pub fn open_external(url: &str) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // start 是 cmd 内建命令，且 URL 含 & 等字符必须整体引号传参；
        // CREATE_NO_WINDOW 避免弹出黑色控制台窗口
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let _ = std::process::Command::new("cmd")
            .arg("/c")
            .arg("start")
            .arg("")
            .arg(url)
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(url).spawn();
    }
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    }
}
