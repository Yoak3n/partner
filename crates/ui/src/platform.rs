#[cfg(target_os = "windows")]
use std::ffi::OsStr;
#[cfg(target_os = "windows")]
use std::os::windows::ffi::OsStrExt;
#[cfg(target_os = "windows")]
use windows::Win32::Foundation::HWND;
#[cfg(target_os = "windows")]
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
};
#[cfg(target_os = "windows")]
use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SetForegroundWindow, SW_HIDE, SW_RESTORE};

#[cfg(target_os = "windows")]
unsafe extern "system" {
    pub fn FindWindowW(
        class_name: *const u16,
        window_name: *const u16,
    ) -> HWND;
}

#[cfg(target_os = "windows")]
fn find_hwnd(title: &str) -> Option<HWND> {
    let wide: Vec<u16> = OsStr::new(title)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let hwnd = unsafe { FindWindowW(std::ptr::null(), wide.as_ptr()) };
    if hwnd.0.is_null() { None } else { Some(hwnd) }
}

/// Windows-specific: 通过 DWM API 设置窗口圆角
#[cfg(target_os = "windows")]
pub fn set_rounded_corners_for_title(title: &str) {
    if let Some(hwnd) = find_hwnd(title) {
        unsafe {
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                &DWMWCP_ROUND as *const _ as _,
                std::mem::size_of::<u32>() as u32,
            );
        }
    }
}

#[cfg(target_os = "windows")]
pub fn hide_window(title: &str) {
    if let Some(hwnd) = find_hwnd(title) {
        unsafe { let _ = ShowWindow(hwnd, SW_HIDE); }
    }
}

#[cfg(target_os = "windows")]
pub fn bring_to_foreground(title: &str) {
    if let Some(hwnd) = find_hwnd(title) {
        unsafe {
            let _ = ShowWindow(hwnd, SW_RESTORE);
            let _ = SetForegroundWindow(hwnd);
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub fn set_rounded_corners_for_title(_title: &str) {}

#[cfg(not(target_os = "windows"))]
pub fn hide_window(_title: &str) {}

#[cfg(not(target_os = "windows"))]
pub fn bring_to_foreground(_title: &str) {}
