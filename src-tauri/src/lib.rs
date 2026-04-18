mod adapters;
mod db;
mod watcher;

use tauri::{
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager,
};

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_positioner::init())
        .setup(|app| {
            // macOS: 隐藏 Dock 图标，仅在菜单栏显示
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            // 使用编译时嵌入的图标
            let icon = tauri::include_image!("icons/icon.png");

            // 创建 tray icon
            let _tray = TrayIconBuilder::new()
                .icon(icon)
                .icon_as_template(true)
                .title("0")
                .tooltip("TokenBurger")
                .show_menu_on_left_click(false)
                .on_tray_icon_event(|tray, event| {
                    // 通知 positioner 插件处理 tray 事件（用于窗口定位）
                    tauri_plugin_positioner::on_tray_event(tray.app_handle(), &event);

                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("popup") {
                            if window.is_visible().unwrap_or(false) {
                                let _ = window.hide();
                            } else {
                                // 将窗口定位到 tray icon 下方
                                use tauri_plugin_positioner::{Position, WindowExt};
                                let _ = window
                                    .as_ref()
                                    .window()
                                    .move_window(Position::TrayBottomCenter);
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                    }
                })
                .build(app)?;

            // popup 窗口失焦自动隐藏
            if let Some(popup) = app.get_webview_window("popup") {
                let popup_clone = popup.clone();
                popup.on_window_event(move |event| {
                    if let tauri::WindowEvent::Focused(false) = event {
                        let _ = popup_clone.hide();
                    }
                });
            }

            Ok(())
        })
        // TODO: 注册 Tauri commands
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
