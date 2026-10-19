mod commands;
mod config;
mod state;
mod windows;

use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::GlobalShortcutExt;
use tauri_plugin_global_shortcut::{Code, Modifiers, Shortcut};

use crate::state::AppState;

pub fn run() {
    // 内置快捷键（自定义快捷键为 ROADMAP M7 项）
    let capture_sc = Shortcut::new(Some(Modifiers::CONTROL), Code::Digit1);
    let pin_sc = Shortcut::new(Some(Modifiers::CONTROL), Code::Digit2);

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
                    move |app, shortcut, _event| {
                        if shortcut == &capture_sc {
                            handle_capture(app);
                        } else if shortcut == &pin_sc {
                            handle_pin_clipboard(app);
                        }
                    }
                })
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::new())
        .setup(move |app| {
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

            // 悬浮球移动后持久化位置（节流：位移超过 1px 才写盘）
            if let Some(ball) = handle.get_webview_window(windows::BALL_LABEL) {
                let persist_handle = handle.clone();
                ball.on_window_event(move |event| {
                    if let tauri::WindowEvent::Moved(pos) = event {
                        let state = persist_handle.state::<AppState>();
                        let mut guard = state.config.lock().unwrap();
                        let changed = match guard.ball_position {
                            Some(p) => {
                                (p.x - pos.x as f64).abs() > 1.0 || (p.y - pos.y as f64).abs() > 1.0
                            }
                            None => true,
                        };
                        if changed {
                            guard.ball_position = Some(config::BallPosition {
                                x: pos.x as f64,
                                y: pos.y as f64,
                            });
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
            commands::pin_from_clipboard,
            commands::pin_context,
            commands::pin_close,
            commands::pin_copy,
            commands::pin_save,
            commands::ball_command,
            commands::drag_window,
            commands::get_config,
            commands::update_config,
            commands::app_info,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run snappin");
}

fn handle_capture(app: &AppHandle) {
    // 防重入：已有截图浮层时忽略（含长按重复触发）
    if app
        .webview_windows()
        .keys()
        .any(|l| l.starts_with(windows::OVERLAY_PREFIX))
    {
        return;
    }
    // 事件处理器在主线程执行：创建 webview 必须放阻塞线程（Windows 已知问题）
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        if let Err(e) = commands::do_capture(&app, &state) {
            eprintln!("[snappin] capture failed: {e}");
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

fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::TrayIconBuilder;

    let i_capture = MenuItem::with_id(app, "capture", "截图 (Ctrl+1)", true, None::<&str>)?;
    let i_pin =
        MenuItem::with_id(app, "pin-clipboard", "剪贴板贴图 (Ctrl+2)", true, None::<&str>)?;
    let i_toggle =
        MenuItem::with_id(app, "toggle-ball", "显示/隐藏悬浮球", true, None::<&str>)?;
    let i_quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&i_capture, &i_pin, &i_toggle, &i_quit])?;

    let tray = TrayIconBuilder::new()
        .icon(tauri::include_image!("icons/32x32.png"))
        .tooltip("SnapPin")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "capture" => handle_capture(app),
            "pin-clipboard" => handle_pin_clipboard(app),
            "toggle-ball" => windows::toggle_ball(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;

    // 托盘图标须与应用同生命周期：故意泄漏句柄，避免 drop 后图标消失
    std::mem::forget(tray);
    Ok(())
}
