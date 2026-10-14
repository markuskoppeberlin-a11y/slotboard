//! Operating-system specific parts: keyboard hook, clipboard, sending the
//! paste shortcut and permissions.
//!
//! Every platform module exposes the same functions:
//!
//! * `permission()`, `request_permission()`, `open_permission_settings()`
//! * `start_hook(tx)`, `hook_running()`, `ensure_hook_enabled()`, `set_paused(bool)`
//! * `send_paste()`
//! * `system_language()`, `notice()`
//! * `clipboard::{change_count, read, write, clear}` and `clipboard::Snapshot`

use serde::Serialize;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::*;

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::*;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::*;

/// Whether Slotboard may watch the keyboard and send the paste shortcut.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    Granted,
    #[default]
    Missing,
    /// macOS: Accessibility is granted, but the keyboard tap still could not
    /// be created – "Input Monitoring" is needed as well.
    InputMonitoring,
    /// The system does not allow global shortcuts (e.g. Linux on Wayland).
    Unsupported,
}

/// A human-friendly description of what a slot holds (for the menu).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContentKind {
    Text(String),
    Image,
    Files(Vec<String>),
    Other,
}

/// Turns a `file:///Users/me/My%20File.pdf` URL into `My File.pdf`.
#[allow(dead_code)]
pub fn file_name_from_url(url: &str) -> String {
    let path = url.trim_end_matches('\0').trim_end_matches('/');
    let last = path.rsplit('/').next().unwrap_or(path);
    percent_decode(last)
}

#[allow(dead_code)]
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(v) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Takes the last path component of a Windows or Unix path.
#[allow(dead_code)]
pub fn file_name_from_path(path: &str) -> String {
    let p = path.trim_end_matches(['\\', '/']);
    p.rsplit(['\\', '/']).next().unwrap_or(p).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names() {
        assert_eq!(file_name_from_url("file:///Users/me/My%20File.pdf"), "My File.pdf");
        assert_eq!(file_name_from_url("file:///Users/me/Folder/"), "Folder");
        assert_eq!(file_name_from_url("file:///tmp/%C3%BCber.txt"), "über.txt");
        assert_eq!(file_name_from_url("file:///tmp/100%"), "100%");
        assert_eq!(file_name_from_path("C:\\Users\\me\\Bild.png"), "Bild.png");
        assert_eq!(file_name_from_path("/home/me/doc.txt"), "doc.txt");
    }
}
