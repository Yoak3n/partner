use std::sync::OnceLock;
use tokio::sync::{mpsc, Mutex};

/// Tray events that map into iced messages.
#[derive(Debug, Clone)]
pub enum TrayEvent {
    Show,
    Quit,
}

static TRAY_TX: OnceLock<mpsc::UnboundedSender<TrayEvent>> = OnceLock::new();
static TRAY_RX: OnceLock<Mutex<mpsc::UnboundedReceiver<TrayEvent>>> = OnceLock::new();

/// Initialize the system tray with icon and context menu.
pub fn init() {
    let (tx, rx) = mpsc::unbounded_channel();
    TRAY_RX.get_or_init(|| Mutex::new(rx));
    let _ = TRAY_TX.set(tx.clone());

    std::thread::spawn(move || {
        setup_tray(tx);
    });
}

/// Receive a tray event (for iced subscription).
pub async fn recv_event() -> TrayEvent {
    let rx = TRAY_RX.get().expect("tray not initialized");
    let mut guard = rx.lock().await;
    match guard.recv().await {
        Some(evt) => evt,
        None => std::future::pending().await,
    }
}

/// Get the current executable path for re-launching
fn get_exe_path() -> Option<std::path::PathBuf> {
    std::env::current_exe().ok()
}

/// Re-launch the application
fn relaunch_app() {
    if let Some(exe) = get_exe_path() {
        eprintln!("[tray] Relaunching app from: {:?}", exe);
        match std::process::Command::new(&exe).spawn() {
            Ok(_) => eprintln!("[tray] App relaunched successfully"),
            Err(e) => eprintln!("[tray] Failed to relaunch app: {}", e),
        }
    } else {
        eprintln!("[tray] Could not determine executable path");
    }
}

fn setup_tray(tx: mpsc::UnboundedSender<TrayEvent>) {
    // Initialize GTK for Linux/Wayland tray support
    #[cfg(not(windows))]
    {
        if !gtk::is_initialized() {
            gtk::init().expect("Failed to initialize GTK");
        }
    }
    
    let icon = load_icon();

    let menu = muda::Menu::new();
    let show_item = muda::MenuItem::with_id("show", "Show", true, None);
    let quit_item = muda::MenuItem::with_id("quit", "Quit", true, None);
    menu.append(&show_item).expect("failed to append show item");
    menu.append(&muda::PredefinedMenuItem::separator()).expect("failed to append separator");
    menu.append(&quit_item).expect("failed to append quit item");

    let tray = tray_icon::TrayIconBuilder::new()
        .with_icon(icon)
        .with_tooltip("AI Partner")
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_menu_on_right_click(true)
        .build()
        .expect("failed to create tray icon");

    // Handle tray icon events (double-click to show)
    tray_icon::TrayIconEvent::set_event_handler(Some(move |event| {
        if let tray_icon::TrayIconEvent::DoubleClick { .. } = event {
            // Try to send event to running app, if that fails, relaunch
            if send_event(TrayEvent::Show).is_err() {
                relaunch_app();
            }
        }
    }));

    // Handle menu events
    muda::MenuEvent::set_event_handler(Some(move |event: muda::MenuEvent| {
        match event.id().0.as_str() {
            "show" => {
                // Try to send event to running app, if that fails, relaunch
                if tx.send(TrayEvent::Show).is_err() {
                    relaunch_app();
                }
            }
            "quit" => {
                let _ = tx.send(TrayEvent::Quit);
                // Give some time for cleanup, then exit
                std::thread::sleep(std::time::Duration::from_millis(100));
                std::process::exit(0);
            }
            _ => {}
        }
    }));

    // Keep tray alive and pump messages
    let _keep = tray;
    #[cfg(windows)]
    {
        use windows::Win32::UI::WindowsAndMessaging::{GetMessageW, TranslateMessage, DispatchMessageW, MSG};
        let mut msg: MSG = unsafe { std::mem::zeroed() };
        unsafe {
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
    #[cfg(not(windows))]
    {
        // Run GTK main loop for tray icon to work on Linux/Wayland
        gtk::main();
    }
}

fn load_icon() -> tray_icon::Icon {
    #[cfg(windows)]
    {
        tray_icon::Icon::from_path(
            concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/icons/icon.ico"),
            None,
        ).expect("failed to load tray icon from ico")
    }
    #[cfg(not(windows))]
    {
        let icon_path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/icons/icon.png");
        let image = image::open(icon_path)
            .expect("failed to open tray icon")
            .into_rgba8();
        let (width, height) = image.dimensions();
        let rgba = image.into_raw();
        tray_icon::Icon::from_rgba(rgba, width, height)
            .expect("failed to create tray icon from RGBA")
    }
}

fn send_event(event: TrayEvent) -> Result<(), mpsc::error::SendError<TrayEvent>> {
    if let Some(tx) = TRAY_TX.get() {
        tx.send(event)
    } else {
        Ok(())
    }
}
