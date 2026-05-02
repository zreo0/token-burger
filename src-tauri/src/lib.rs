mod adapters;
mod commands;
mod db;
pub mod logger;
mod pricing;
mod types;
mod watcher;

use tauri::{
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, LogicalPosition, Manager, Rect, Runtime, WebviewUrl, WebviewWindow,
};
use tauri_plugin_positioner::{Position as TrayPosition, WindowExt};

const POPUP_WINDOW_LABEL: &str = "popup";
const POPUP_WINDOW_WIDTH: f64 = 390.0;
const POPUP_WINDOW_HEIGHT: f64 = 540.0;

fn attach_popup_auto_hide<R: Runtime>(_popup: &WebviewWindow<R>) {
    #[cfg(not(debug_assertions))]
    {
        let popup_clone = _popup.clone();
        _popup.on_window_event(move |event| {
            if let tauri::WindowEvent::Focused(false) = event {
                let _ = popup_clone.hide();
            }
        });
    }
}

fn get_popup_initial_position<R: Runtime>(app: &AppHandle<R>, rect: &Rect) -> (f64, f64) {
    let tray_position = rect.position.to_physical::<f64>(1.0);
    let tray_size = rect.size.to_physical::<f64>(1.0);
    let scale_factor = app
        .monitor_from_point(tray_position.x, tray_position.y)
        .ok()
        .flatten()
        .map(|monitor| monitor.scale_factor())
        .or_else(|| {
            app.primary_monitor()
                .ok()
                .flatten()
                .map(|monitor| monitor.scale_factor())
        })
        .unwrap_or(1.0);
    let tray_position = tray_position.to_logical::<f64>(scale_factor);
    let tray_size = tray_size.to_logical::<f64>(scale_factor);

    (
        tray_position.x + (tray_size.width / 2.0) - (POPUP_WINDOW_WIDTH / 2.0),
        tray_position.y,
    )
}

fn ensure_popup_window<R: Runtime>(
    app: &AppHandle<R>,
    rect: &Rect,
) -> tauri::Result<WebviewWindow<R>> {
    if let Some(window) = app.get_webview_window(POPUP_WINDOW_LABEL) {
        return Ok(window);
    }

    let (x, y) = get_popup_initial_position(app, rect);

    #[cfg(target_os = "windows")]
    let transparent = false;

    #[cfg(not(target_os = "windows"))]
    let transparent = true;

    let popup_builder = tauri::WebviewWindowBuilder::new(
        app,
        POPUP_WINDOW_LABEL,
        WebviewUrl::App("index.html".into()),
    )
    .title("TokenBurger")
    .inner_size(POPUP_WINDOW_WIDTH, POPUP_WINDOW_HEIGHT)
    .resizable(false)
    .decorations(false)
    .transparent(transparent);

    #[cfg(target_os = "windows")]
    let popup_builder = popup_builder.shadow(false);

    let popup = popup_builder
        .visible(false)
        .visible_on_all_workspaces(true)
        .focused(false)
        .position(x, y)
        .build()?;

    attach_popup_auto_hide(&popup);

    Ok(popup)
}

fn position_popup_window<R: Runtime>(app: &AppHandle<R>, popup: &WebviewWindow<R>, rect: &Rect) {
    let (x, y) = get_popup_initial_position(app, rect);
    let _ = popup.set_visible_on_all_workspaces(true);

    if popup
        .move_window_constrained(TrayPosition::TrayBottomCenter)
        .is_err()
    {
        let _ = popup.set_position(LogicalPosition::new(x, y));
    }
}

#[tauri::command]
fn restart_app(app: AppHandle) {
    app.request_restart();
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_positioner::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // macOS: 隐藏 Dock 图标，仅在菜单栏显示
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            // 初始化数据库
            let db_path = db::get_db_path(app.handle());
            db::init_db(&db_path).expect("数据库初始化失败");

            // 加载定价表
            let resource_dir = app
                .path()
                .resource_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("."));
            let pricing_table = pricing::load_pricing_table(&resource_dir);

            // 从数据库读取设置
            let settings = {
                let conn = db::open_readonly(&db_path).ok();
                let defaults = types::AppSettings::default();
                match conn {
                    Some(c) => {
                        let enabled_str =
                            db::queries::get_setting(&c, "enabled_agents").unwrap_or(None);
                        let enabled_agents: Vec<String> = match enabled_str {
                            Some(s) => {
                                serde_json::from_str(&s).unwrap_or(defaults.enabled_agents.clone())
                            }
                            None => defaults.enabled_agents.clone(),
                        };
                        let keep_days = db::queries::get_setting(&c, "keep_days")
                            .unwrap_or(None)
                            .and_then(|v| v.parse().ok())
                            .unwrap_or(defaults.keep_days);
                        let watch_mode = db::queries::get_setting(&c, "watch_mode")
                            .unwrap_or(None)
                            .unwrap_or(defaults.watch_mode.clone());
                        let polling_interval_secs =
                            db::queries::get_setting(&c, "polling_interval_secs")
                                .unwrap_or(None)
                                .and_then(|v| v.parse().ok())
                                .unwrap_or(defaults.polling_interval_secs);
                        let language = db::queries::get_setting(&c, "language")
                            .unwrap_or(None)
                            .unwrap_or(defaults.language.clone());
                        (
                            enabled_agents,
                            keep_days,
                            watch_mode,
                            polling_interval_secs,
                            language,
                        )
                    }
                    None => (
                        defaults.enabled_agents,
                        defaults.keep_days,
                        defaults.watch_mode,
                        defaults.polling_interval_secs,
                        defaults.language,
                    ),
                }
            };
            let (enabled_agents, keep_days, watch_mode, polling_interval_secs, language) = settings;

            // 注册共享状态
            let db_path_str = db_path.to_string_lossy().to_string();

            // 启动写入线程
            let db_manager = db::DbManager::start(db_path.clone(), app.handle().clone());

            // 克隆 write_tx 供 AppState 和 Watcher 分别使用
            let write_tx_for_state = db_manager.write_tx.clone();

            // 启动 Watcher 引擎（只启动已启用的 adapter）
            let all = adapters::all_adapters();
            let active_adapters: Vec<Box<dyn adapters::AgentAdapter>> = all
                .into_iter()
                .filter(|a| enabled_agents.contains(&a.agent_name().to_string()))
                .collect();
            let watcher_config = watcher::WatcherConfig {
                watch_mode,
                polling_interval_secs,
                keep_days,
            };
            let watcher_engine = watcher::WatcherEngine::start(
                active_adapters,
                db_manager.write_tx,
                app.handle().clone(),
                watcher_config,
                db_path.clone(),
            );

            app.manage(commands::AppState {
                db_path: db_path_str,
                pricing: pricing_table,
                watcher: std::sync::Mutex::new(Some(watcher_engine)),
                write_tx: std::sync::Mutex::new(write_tx_for_state),
            });

            // 使用编译时嵌入的图标，Windows 使用白色版本以适配深色任务栏
            #[cfg(target_os = "windows")]
            let icon = tauri::include_image!("icons/icon-windows.png");
            #[cfg(not(target_os = "windows"))]
            let icon = tauri::include_image!("icons/icon.png");

            // 构建右键上下文菜单
            let menu = commands::build_tray_menu(app.handle(), &language)?;

            // 创建 tray icon
            let _tray = TrayIconBuilder::with_id("main")
                .icon(icon)
                .icon_as_template(true)
                .title("0")
                .tooltip("TokenBurger")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| {
                    match event.id().as_ref() {
                        "settings" => {
                            if let Some(window) = app.get_webview_window("settings") {
                                let _ = window.show();
                                let _ = window.set_focus();
                            } else {
                                // 窗口被销毁时重建
                                let _ = tauri::WebviewWindowBuilder::new(
                                    app,
                                    "settings",
                                    WebviewUrl::App("index.html#/settings".into()),
                                )
                                .title("TokenBurger Settings")
                                .inner_size(640.0, 520.0)
                                .resizable(true)
                                .build();
                            }
                        }
                        "quit" => {
                            std::process::exit(0);
                        }
                        _ => {}
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    // 通知 positioner 插件处理 tray 事件（用于窗口定位）
                    tauri_plugin_positioner::on_tray_event(tray.app_handle(), &event);

                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        rect,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window(POPUP_WINDOW_LABEL) {
                            if window.is_visible().unwrap_or(false) {
                                let _ = window.hide();
                            } else {
                                position_popup_window(app, &window, &rect);
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        } else if let Ok(window) = ensure_popup_window(app, &rect) {
                            position_popup_window(app, &window, &rect);
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                })
                .build(app)?;

            // settings 窗口关闭时改为隐藏，避免被销毁后无法重新打开
            if let Some(settings_win) = app.get_webview_window("settings") {
                let settings_clone = settings_win.clone();
                settings_win.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = settings_clone.hide();
                    }
                });
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_token_summary,
            commands::get_agent_list,
            commands::toggle_agent,
            commands::clear_data,
            commands::get_settings,
            commands::update_settings,
            commands::get_pricing,
            commands::get_platform_info,
            restart_app,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
