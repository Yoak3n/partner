mod app;
mod components;
mod platform;
mod runtime_bridge;
mod tray;

use app::App;

const ICON_PNG: &[u8] = include_bytes!("../../../assets/icons/icon.png");

fn main() -> iced::Result {
    let icon = iced::window::icon::from_file_data(ICON_PNG, None)
        .expect("Failed to load window icon");

    tray::init();

    let font_data = load_or_cache_chinese_font();

    let mut app = iced::application(App::new, App::update, App::view)
        .theme(App::theme)
        .title("AI Partner")
        .subscription(App::subscription)
        .window(iced::window::Settings {
            decorations: false,
            icon: Some(icon),
            platform_specific: iced::window::settings::PlatformSpecific {
                #[cfg(target_os = "linux")]
                application_id: "ai-partner".to_string(),
                ..Default::default()
            },
            ..Default::default()
        });

    if let Some(bytes) = font_data {
        app = app.font(bytes);
    }

    app.run()
}

/// 启动时获取中文字体：优先读缓存，未命中则从系统提取并缓存
fn load_or_cache_chinese_font() -> Option<&'static [u8]> {
    let cache_dir = dirs::cache_dir()?.join("ai-partner").join("fonts");
    let cached_path = cache_dir.join("chinese-font.ttf");

    // 1. 缓存命中，直接加载
    if cached_path.exists() {
        if let Ok(data) = std::fs::read(&cached_path) {
            log::info!("loaded Chinese font from cache: {}", cached_path.display());
            return Some(leak(data));
        }
    }

    // 2. 缓存未命中，从系统查找中文字体
    let system_font = find_system_chinese_font()?;
    log::info!("found system Chinese font: {}", system_font.display());

    // 复制到缓存目录
    if let Err(e) = std::fs::create_dir_all(&cache_dir) {
        log::warn!("failed to create font cache dir: {e}");
    } else if let Err(e) = std::fs::copy(&system_font, &cached_path) {
        log::warn!("failed to cache font: {e}");
    } else {
        log::info!("cached Chinese font to: {}", cached_path.display());
    }

    // 加载字体数据
    let data = std::fs::read(system_font).ok()?;
    Some(leak(data))
}

fn leak(data: Vec<u8>) -> &'static [u8] {
    Box::leak(data.into_boxed_slice())
}

/// 在系统字体目录中查找可用的中文字体文件
fn find_system_chinese_font() -> Option<std::path::PathBuf> {
    #[cfg(windows)]
    {
        let windir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".to_string());
        let font_dir = std::path::PathBuf::from(windir).join("Fonts");
        let candidates = [
            "msyh.ttc",     // 微软雅黑
            "msyhbd.ttc",   // 微软雅黑 粗体
            "simhei.ttf",   // 黑体
            "simsun.ttc",   // 宋体
            "SIMYOU.TTF",   // 幼圆
        ];
        for name in &candidates {
            let path = font_dir.join(name);
            if path.exists() {
                return Some(path);
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        let font_dirs = [
            "/usr/share/fonts",
            "/usr/local/share/fonts",
        ];
        let candidates = [
            "NotoSansCJK-Regular.ttc",
            "NotoSansSC-Regular.otf",
            "wqy-zenhei.ttc",
            "wqy-microhei.ttc",
            "DroidSansFallbackFull.ttf",
        ];
        for dir in &font_dirs {
            for name in &candidates {
                // 递归搜索（字体文件可能在子目录中）
                if let Some(path) = find_file_recursive(std::path::Path::new(dir), name) {
                    return Some(path);
                }
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        let font_dirs = [
            "/System/Library/Fonts",
            "/Library/Fonts",
            dirs::home_dir().map(|h| h.join("Library/Fonts")).unwrap_or_default(),
        ];
        let candidates = [
            "PingFang.ttc",
            "STHeiti Medium.ttc",
            "Hiragino Sans GB.ttc",
        ];
        for dir in &font_dirs {
            for name in &candidates {
                let path = dir.join(name);
                if path.exists() {
                    return Some(path);
                }
            }
        }
    }

    None
}

#[cfg(target_os = "linux")]
fn find_file_recursive(dir: &std::path::Path, name: &str) -> Option<std::path::PathBuf> {
    for entry in std::fs::read_dir(dir).ok()? {
        let entry = entry.ok()?;
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_file_recursive(&path, name) {
                return Some(found);
            }
        } else if path.file_name().and_then(|n| n.to_str()) == Some(name) {
            return Some(path);
        }
    }
    None
}

/// 启动后设置窗口圆角（Windows DWM）
pub(crate) fn apply_rounded_corners() {
    platform::set_rounded_corners_for_title("AI Partner");
}
