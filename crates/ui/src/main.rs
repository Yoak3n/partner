mod app;
mod components;
mod platform;
mod runtime_bridge;
mod tray;

use app::App;

const ICON_PNG: &[u8] = include_bytes!("../icons/icon.png");

fn main() -> iced::Result {
    let icon = iced::window::icon::from_file_data(ICON_PNG, None)
        .expect("Failed to load window icon");

    // Initialize system tray before iced event loop
    tray::init();

    iced::application(App::new, App::update, App::view)
        .theme(App::theme)
        .title("AI Partner")
        .subscription(App::subscription)
        .window(iced::window::Settings {
            decorations: false,
            icon: Some(icon),
            ..Default::default()
        })
        .run()
}

/// 启动后设置窗口圆角（Windows DWM）
pub(crate) fn apply_rounded_corners() {
    platform::set_rounded_corners_for_title("AI Partner");
}
