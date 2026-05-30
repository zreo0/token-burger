mod account_usage;
mod adapters;
mod commands;
mod db;
pub mod logger;
mod pricing;
mod tray_usage;
mod types;
mod watcher;

use std::sync::{atomic::AtomicBool, Arc};

use tauri::{
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Rect, Runtime, WebviewUrl,
    WebviewWindow,
};

const POPUP_WINDOW_LABEL: &str = "popup";
const POPUP_WINDOW_WIDTH: f64 = 390.0;
const POPUP_WINDOW_HEIGHT: f64 = 540.0;
const POPUP_WINDOW_MAX_HEIGHT: f64 = 680.0;
const POPUP_OFFSCREEN_POSITION: f64 = -10_000.0;

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
    rect: Option<&Rect>,
) -> tauri::Result<WebviewWindow<R>> {
    if let Some(window) = app.get_webview_window(POPUP_WINDOW_LABEL) {
        return Ok(window);
    }

    let (x, y) = rect
        .map(|rect| get_popup_initial_position(app, rect))
        .unwrap_or((POPUP_OFFSCREEN_POSITION, POPUP_OFFSCREEN_POSITION));

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

fn prewarm_popup_window<R: Runtime>(app: &AppHandle<R>) {
    if let Err(error) = ensure_popup_window(app, None) {
        log::warn!("预热弹窗失败: {}", error);
    }
}

fn position_popup_window<R: Runtime>(app: &AppHandle<R>, popup: &WebviewWindow<R>, rect: &Rect) {
    let (x, y) = get_popup_initial_position(app, rect);
    let _ = popup.set_visible_on_all_workspaces(true);
    let _ = popup.set_position(LogicalPosition::new(x, y));
}

pub(crate) fn toggle_popup_window<R: Runtime>(app: &AppHandle<R>, rect: &Rect) {
    if let Some(window) = app.get_webview_window(POPUP_WINDOW_LABEL) {
        if window.is_visible().unwrap_or(false) {
            let _ = window.hide();
        } else {
            position_popup_window(app, &window, rect);
            show_popup(&window);
        }
    } else if let Ok(window) = ensure_popup_window(app, Some(rect)) {
        position_popup_window(app, &window, rect);
        show_popup(&window);
    }
}

fn show_popup<R: Runtime>(window: &WebviewWindow<R>) {
    let _ = window.show();
    let _ = window.set_focus();
    let _ = window.emit("popup-shown", ());
}

#[tauri::command]
fn restart_app(app: AppHandle) {
    app.request_restart();
}

#[tauri::command]
fn resize_popup_window(app: AppHandle, height: f64) -> Result<(), String> {
    let popup = app
        .get_webview_window(POPUP_WINDOW_LABEL)
        .ok_or_else(|| "popup window not found".to_string())?;
    let target_height = if height.is_finite() {
        height.clamp(POPUP_WINDOW_HEIGHT, POPUP_WINDOW_MAX_HEIGHT)
    } else {
        POPUP_WINDOW_HEIGHT
    };

    popup
        .set_size(LogicalSize::new(POPUP_WINDOW_WIDTH, target_height))
        .map_err(|error| error.to_string())
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
            let account_usage_manager =
                account_usage::manager::AccountUsageManager::new(db_path.clone());
            account_usage_manager
                .initialize_provider_states()
                .expect("账号用量 Provider 状态初始化失败");

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
            let cold_start_complete = Arc::new(AtomicBool::new(false));

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
                cold_start_complete.clone(),
            );

            app.manage(commands::AppState {
                db_path: db_path_str,
                pricing: pricing_table,
                watcher: std::sync::Mutex::new(Some(watcher_engine)),
                write_tx: std::sync::Mutex::new(write_tx_for_state),
                account_usage: account_usage_manager,
                cold_start_complete: cold_start_complete.clone(),
            });

            // 使用编译时嵌入的图标，Windows 使用白色版本以适配深色任务栏
            #[cfg(target_os = "windows")]
            let icon = tauri::include_image!("icons/icon-windows.png");
            #[cfg(not(target_os = "windows"))]
            let icon = tauri::include_image!("icons/tray-icon.png");

            // 构建右键上下文菜单
            let menu = commands::build_tray_menu(app.handle(), &language)?;

            // 创建 tray icon
            let _tray = TrayIconBuilder::with_id("main")
                .icon(icon)
                .icon_as_template(true)
                .title(commands::main_tray_scanning_title(&language))
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
                        if !commands::is_cold_start_complete(app) {
                            return;
                        }
                        toggle_popup_window(app, &rect);
                    }
                })
                .build(app)?;

            commands::sync_account_usage_tray_items(app.handle());
            prewarm_popup_window(app.handle());

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
            commands::list_account_usage_providers,
            commands::get_account_usage_snapshots,
            commands::refresh_account_usage_all,
            commands::refresh_account_usage_provider,
            commands::save_account_usage_credential,
            commands::clear_account_usage_credential,
            commands::get_account_usage_provider_state,
            commands::set_account_usage_provider_enabled,
            commands::set_account_usage_provider_menu_bar_visible,
            resize_popup_window,
            restart_app,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
