mod commands;
mod config;
mod state;
mod windows;

use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::GlobalShortcutExt;
use tauri_plugin_global_shortcut::{Code, Modifiers, Shortcut, ShortcutState};
use std::time::{Duration, Instant};

use crate::state::AppState;

/// Ctrl+V 武装有效期：截图"复制"成功后在此时间内按 Ctrl+V 弹贴图。
/// 到期自动注销热键，恢复正常粘贴（RegisterHotKey 注册期间会系统级
/// 拦截该组合键，必须按时注销，不能常驻）。
pub const PASTE_ARM_TTL: Duration = Duration::from_secs(20);

/// 构造 Ctrl+V 快捷键。
pub fn paste_shortcut() -> Shortcut {
    Shortcut::new(Some(Modifiers::CONTROL), Code::KeyV)
}

pub fn run() {
    // 内置快捷键（自定义快捷键为 ROADMAP M7 项）
    let capture_sc = Shortcut::new(Some(Modifiers::CONTROL), Code::Digit1);
    let pin_sc = Shortcut::new(Some(Modifiers::CONTROL), Code::Digit2);
    let paste_sc = paste_shortcut();

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
                    let capture_sc = capture_sc;
                    let pin_sc = pin_sc;
                    let paste_sc = paste_sc;
                    move |app, shortcut, event| {
                        // 插件对**按下**和**抬起**各回调一次，只认 Pressed。
                        // 否则一次按键会执行两遍：截图侧表现为「start 紧接 busy」，
                        // Ctrl+V 侧会真的粘贴两次。
                        if event.state != ShortcutState::Pressed {
                            return;
                        }
                        if shortcut == &capture_sc {
                            handle_capture(app, "hotkey");
                        } else if shortcut == &pin_sc {
                            handle_pin_clipboard(app);
                        } else if shortcut == &paste_sc {
                            handle_paste(app);
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
            #[cfg(target_os = "macos")]
            {
                // 纯后台常驻：不占 Dock 图标（截图工具的常规形态，同 PixPin/Shottr）
                app.set_activation_policy(tauri::ActivationPolicy::Accessory);
                // 屏幕录制权限：macOS 10.15+ 未授权时采集到的是空白/全黑图，
                // 表现为"截图全黑 / 采集失败"，必须明确提示（首次会弹系统授权框）
                if !snappin_core_platform::ensure_screen_capture_permission() {
                    eprintln!(
                        "[snappin] 未获得屏幕录制权限：系统设置 → 隐私与安全性 → 屏幕录制 → \
                         勾选 SnapPin，然后**重启应用**（macOS 要求重启才生效）"
                    );
                }
            }

            let handle = app.handle();

            // 加载（并落一次）配置
            let config_dir = handle.path().app_config_dir().map_err(|e| e.to_string())?;
            let cfg = config::load(&config_dir);
            let _ = config::save(&config_dir, &cfg);
            {
                let state = handle.state::<AppState>();
                *state.config.lock().unwrap() = cfg.clone();
            }

            // 注册全局快捷键（须在主线程；setup 即在主线程）。
            // 注册失败（如被其他程序占用）时降级为日志，不阻断启动。
            if let Err(e) = handle.global_shortcut().register(capture_sc) {
                eprintln!("[snappin] register Ctrl+1 failed: {e}");
            }
            if let Err(e) = handle.global_shortcut().register(pin_sc) {
                eprintln!("[snappin] register Ctrl+2 failed: {e}");
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

            setup_tray(&handle)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::start_capture,
            commands::overlay_context,
            commands::confirm_selection,
            commands::cancel_capture,
            commands::ui_log,
            commands::pin_from_clipboard,
            commands::pin_context,
            commands::pin_close,
            commands::pin_set_topmost,
            commands::pin_set_ignore_cursor_events,
            commands::pin_copy,
            commands::pin_save,
            commands::ball_command,
            commands::ball_menu,
            commands::drag_window,
            commands::get_config,
            commands::update_config,
            commands::app_info,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run snappin");
}

fn handle_capture(app: &AppHandle, src: &str) {
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
        if let Err(e) = commands::do_pin_from_clipboard(&app, &state, None) {
            eprintln!("[snappin] pin from clipboard failed: {e}");
        }
    });
}

/// 武装 Ctrl+V（复制成功后调用）：注册全局 Ctrl+V 热键并启动到期解除任务。
/// 已注册时 register 返回错误，忽略（幂等）。
pub fn arm_paste(app: &AppHandle, state: &AppState) {
    let gen = state
        .paste_gen
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        + 1;
    {
        let mut guard = state.paste_armed.lock().unwrap();
        *guard = Some((gen, Instant::now() + PASTE_ARM_TTL));
    }
    // **必须在主线程注册**：RegisterHotKey 把 WM_HOTKEY 投递到"注册该热键
    // 的那个线程"的消息队列。confirm_selection 跑在 async 工作线程上，
    // 在那里注册虽然返回成功，但该线程没有消息循环 → 永远收不到按键
    // （表现为"复制成功但 Ctrl+V 贴图没反应"）。
    let app_reg = app.clone();
    crate::windows::post_to_main(app, move |_| {
        if let Err(e) = app_reg.global_shortcut().register(paste_shortcut()) {
            // 常见原因：Ctrl+V 被其他程序占用。降级为不可用（Ctrl+2 仍可贴图）。
            eprintln!("[snappin] arm Ctrl+V failed: {e}");
        } else {
            eprintln!("[snappin] Ctrl+V armed (gen {gen})");
        }
    });
    // 到期自动解除（幂等：仅当代号未变时执行，重复 arm 会废弃旧任务）
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        std::thread::sleep(PASTE_ARM_TTL);
        disarm_if_current(&app2, gen);
    });
}

/// 解除武装并注销 Ctrl+V 热键（幂等，可重复调用）。
pub fn disarm_paste(app: &AppHandle) {
    let state = app.state::<AppState>();
    {
        let mut guard = state.paste_armed.lock().unwrap();
        *guard = None;
    }
    // 与注册对称：注销同样在主线程执行
    let app_unreg = app.clone();
    crate::windows::post_to_main(app, move |_| {
        let _ = app_unreg.global_shortcut().unregister(paste_shortcut());
    });
    eprintln!("[snappin] Ctrl+V disarmed");
}

/// 仅当武装代号仍为 `gen` 时解除（用于到期任务，避免误杀新一轮武装）。
fn disarm_if_current(app: &AppHandle, gen: u64) {
    let state = app.state::<AppState>();
    let current = {
        let guard = state.paste_armed.lock().unwrap();
        guard.map(|(g, _)| g)
    };
    if current == Some(gen) {
        disarm_paste(app);
    }
}

/// Ctrl+V 分发：武装中 → 立即解除并跟随鼠标贴图；未武装 → 忽略
/// （热键只在武装期注册，正常不会收到）。
fn handle_paste(app: &AppHandle) {
    let state = app.state::<AppState>();
    let armed = {
        let guard = state.paste_armed.lock().unwrap();
        matches!(*guard, Some((_, dl)) if Instant::now() < dl)
    };
    if !armed {
        return;
    }
    // 先解除再贴图：防止连按弹出多个贴图窗，也避免贴图期间按键被吞
    disarm_paste(app);
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let at = snappin_core_platform::cursor_position().map(|(px, py)| (px as f64, py as f64));
        if let Err(e) = commands::do_pin_from_clipboard(&app, &state, at) {
            eprintln!("[snappin] Ctrl+V pin failed: {e}");
        }
    });
}

fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::TrayIconBuilder;

    let i_capture = MenuItem::with_id(app, "capture", "截图 (Ctrl+1)", true, None::<&str>)?;
    let i_pin =
        MenuItem::with_id(app, "pin-clipboard", "剪贴板贴图 (Ctrl+2)", true, None::<&str>)?;
    let i_toggle =
        MenuItem::with_id(app, "toggle-ball", "显示/隐藏悬浮球", true, None::<&str>)?;
    let i_about = MenuItem::with_id(app, "about", "关于v0.1", true, None::<&str>)?;
    let i_quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&i_capture, &i_pin, &i_toggle, &i_about, &i_quit])?;

    let tray = TrayIconBuilder::new()
        .icon(tauri::include_image!("icons/32x32.png"))
        .tooltip("SnapPin")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| handle_menu_event(app, event.id().as_ref()))
        .build(app)?;

    // 托盘图标须与应用同生命周期：故意泄漏句柄，避免 drop 后图标消失
    std::mem::forget(tray);
    Ok(())
}

    // 托盘菜单与悬浮球右键菜单共用的事件分发（单一事实来源）。
    // 挂在托盘 on_menu_event 上：Tauri 2 的菜单事件全网广播，托盘这一处
    // 即可收到全部菜单（托盘菜单 + 悬浮球 popup_menu + 未来窗口菜单）的点击。
    // 切莫再在窗口上注册 on_menu_event，否则每次点击会触发两次。
    fn handle_menu_event(app: &AppHandle, id: &str) {
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
        "quit" => app.exit(0),
        _ => {}
    }
}

/// "关于..." 跳转的项目仓库地址。
pub const ABOUT_URL: &str = "https://gitee.com/dskcvip/snappin";

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
