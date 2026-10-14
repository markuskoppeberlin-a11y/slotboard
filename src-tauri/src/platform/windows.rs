//! Windows: low-level keyboard hook, clipboard with every format, SendInput.
//! No special permission is required.

#![allow(non_snake_case, non_camel_case_types, clippy::upper_case_acronyms)]

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::mpsc::{self, Sender};
use std::time::{Duration, Instant};

use super::Permission;
use crate::engine::Msg;
use crate::filter::{Key, KeyEvent, KeyFilter, KeyKind, Mods};

type HANDLE = *mut c_void;
type HWND = *mut c_void;
type HHOOK = *mut c_void;
type HINSTANCE = *mut c_void;
type HGLOBAL = *mut c_void;
type BOOL = i32;
type WPARAM = usize;
type LPARAM = isize;
type LRESULT = isize;
type HOOKPROC = Option<unsafe extern "system" fn(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT>;

#[repr(C)]
struct POINT {
    x: i32,
    y: i32,
}

#[repr(C)]
struct MSG {
    hwnd: HWND,
    message: u32,
    wParam: WPARAM,
    lParam: LPARAM,
    time: u32,
    pt: POINT,
}

#[repr(C)]
struct KBDLLHOOKSTRUCT {
    vkCode: u32,
    scanCode: u32,
    flags: u32,
    time: u32,
    dwExtraInfo: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct KEYBDINPUT {
    wVk: u16,
    wScan: u16,
    dwFlags: u32,
    time: u32,
    dwExtraInfo: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct MOUSEINPUT {
    dx: i32,
    dy: i32,
    mouseData: u32,
    dwFlags: u32,
    time: u32,
    dwExtraInfo: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct HARDWAREINPUT {
    uMsg: u32,
    wParamL: u16,
    wParamH: u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
union INPUT_UNION {
    mi: MOUSEINPUT,
    ki: KEYBDINPUT,
    hi: HARDWAREINPUT,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct INPUT {
    r#type: u32,
    u: INPUT_UNION,
}

#[link(name = "user32")]
extern "system" {
    fn SetWindowsHookExW(idHook: i32, lpfn: HOOKPROC, hmod: HINSTANCE, dwThreadId: u32) -> HHOOK;
    fn CallNextHookEx(hhk: HHOOK, nCode: i32, wParam: WPARAM, lParam: LPARAM) -> LRESULT;
    fn GetMessageW(lpMsg: *mut MSG, hWnd: HWND, wMsgFilterMin: u32, wMsgFilterMax: u32) -> BOOL;
    fn GetAsyncKeyState(vKey: i32) -> i16;
    fn SendInput(cInputs: u32, pInputs: *const INPUT, cbSize: i32) -> u32;
    fn OpenClipboard(hWndNewOwner: HWND) -> BOOL;
    fn CloseClipboard() -> BOOL;
    fn EmptyClipboard() -> BOOL;
    fn EnumClipboardFormats(format: u32) -> u32;
    fn GetClipboardData(uFormat: u32) -> HANDLE;
    fn SetClipboardData(uFormat: u32, hMem: HANDLE) -> HANDLE;
    fn GetClipboardSequenceNumber() -> u32;
    fn RegisterClipboardFormatW(lpszFormat: *const u16) -> u32;
    fn GetClipboardFormatNameW(format: u32, lpszFormatName: *mut u16, cchMaxCount: i32) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleW(lpModuleName: *const u16) -> HINSTANCE;
    fn GlobalAlloc(uFlags: u32, dwBytes: usize) -> HGLOBAL;
    fn GlobalLock(hMem: HGLOBAL) -> *mut c_void;
    fn GlobalUnlock(hMem: HGLOBAL) -> BOOL;
    fn GlobalSize(hMem: HGLOBAL) -> usize;
    fn GlobalFree(hMem: HGLOBAL) -> HGLOBAL;
    fn GetUserDefaultUILanguage() -> u16;
}

#[link(name = "shell32")]
extern "system" {
    fn ShellExecuteW(
        hwnd: HWND,
        lpOperation: *const u16,
        lpFile: *const u16,
        lpParameters: *const u16,
        lpDirectory: *const u16,
        nShowCmd: i32,
    ) -> HINSTANCE;
}

const WH_KEYBOARD_LL: i32 = 13;
const HC_ACTION: i32 = 0;
const WM_KEYDOWN: usize = 0x0100;
const WM_KEYUP: usize = 0x0101;
const WM_SYSKEYDOWN: usize = 0x0104;
const WM_SYSKEYUP: usize = 0x0105;
const LLKHF_INJECTED: u32 = 0x10;

const INPUT_KEYBOARD: u32 = 1;
const KEYEVENTF_KEYUP: u32 = 0x0002;

const VK_SHIFT: u32 = 0x10;
const VK_CONTROL: u32 = 0x11;
const VK_MENU: u32 = 0x12;
const VK_LWIN: u32 = 0x5B;
const VK_RWIN: u32 = 0x5C;
const VK_LSHIFT: u32 = 0xA0;
const VK_RSHIFT: u32 = 0xA1;
const VK_LCONTROL: u32 = 0xA2;
const VK_RCONTROL: u32 = 0xA3;
const VK_LMENU: u32 = 0xA4;
const VK_RMENU: u32 = 0xA5;
const VK_C: u32 = 0x43;
const VK_V: u32 = 0x56;

const GMEM_MOVEABLE: u32 = 0x0002;

/// Marks input Slotboard sends itself.
const MAGIC: usize = 0x534C_4F54;

// ---------------------------------------------------------------------------
// Keyboard hook
// ---------------------------------------------------------------------------

static HOOK: AtomicIsize = AtomicIsize::new(0);
static PAUSED: AtomicBool = AtomicBool::new(false);
static RESET_FILTER: AtomicBool = AtomicBool::new(false);

thread_local! {
    static FILTER: RefCell<KeyFilter> = RefCell::new(KeyFilter::new());
    static TX: RefCell<Option<Sender<Msg>>> = const { RefCell::new(None) };
    /// Keys currently held (to recognise auto-repeat).
    static DOWN: RefCell<[bool; 256]> = const { RefCell::new([false; 256]) };
    static MODS: Cell<u8> = const { Cell::new(0) };
}

const M_LCTRL: u8 = 1;
const M_RCTRL: u8 = 2;
const M_LALT: u8 = 4;
const M_RALT: u8 = 8;
const M_SHIFT_L: u8 = 16;
const M_SHIFT_R: u8 = 32;
const M_LWIN: u8 = 64;
const M_RWIN: u8 = 128;

fn modifier_bit(vk: u32) -> Option<u8> {
    Some(match vk {
        VK_LCONTROL | VK_CONTROL => M_LCTRL,
        VK_RCONTROL => M_RCTRL,
        VK_LMENU | VK_MENU => M_LALT,
        VK_RMENU => M_RALT,
        VK_LSHIFT | VK_SHIFT => M_SHIFT_L,
        VK_RSHIFT => M_SHIFT_R,
        VK_LWIN => M_LWIN,
        VK_RWIN => M_RWIN,
        _ => return None,
    })
}

fn key_down(vk: u32) -> bool {
    unsafe { GetAsyncKeyState(vk as i32) < 0 }
}

/// Reads the real modifier state (used for non-modifier keys, which never
/// change it, so this also heals a state that got out of sync).
fn sync_mods() -> u8 {
    let mut m = 0u8;
    for (vk, bit) in [
        (VK_LCONTROL, M_LCTRL),
        (VK_RCONTROL, M_RCTRL),
        (VK_LMENU, M_LALT),
        (VK_RMENU, M_RALT),
        (VK_LSHIFT, M_SHIFT_L),
        (VK_RSHIFT, M_SHIFT_R),
        (VK_LWIN, M_LWIN),
        (VK_RWIN, M_RWIN),
    ] {
        if key_down(vk) {
            m |= bit;
        }
    }
    m
}

fn mods_from_bits(m: u8) -> Mods {
    let altgr = m & M_RALT != 0;
    Mods {
        // AltGr sends a fake left Ctrl; it must not count as Ctrl.
        primary: (m & (M_LCTRL | M_RCTRL) != 0) && !altgr,
        alt: m & M_LALT != 0,
        other: altgr || m & (M_LWIN | M_RWIN) != 0,
        shift: m & (M_SHIFT_L | M_SHIFT_R) != 0,
    }
}

fn classify(vk: u32) -> Key {
    match vk {
        0x30..=0x39 => Key::Digit((vk - 0x30) as u8),
        0x60..=0x69 => Key::Digit((vk - 0x60) as u8),
        VK_C => Key::C,
        VK_V => Key::V,
        _ => Key::Other,
    }
}

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let next = || CallNextHookEx(ptr::null_mut(), code, wparam, lparam);
    if code != HC_ACTION || lparam == 0 {
        return next();
    }
    let info = &*(lparam as *const KBDLLHOOKSTRUCT);
    if info.dwExtraInfo == MAGIC {
        return next();
    }
    let is_down = matches!(wparam, WM_KEYDOWN | WM_SYSKEYDOWN);
    let is_up = matches!(wparam, WM_KEYUP | WM_SYSKEYUP);
    if !is_down && !is_up {
        return next();
    }
    let vk = info.vkCode;
    let injected = info.flags & LLKHF_INJECTED != 0;

    let repeat = DOWN.with(|d| {
        let mut d = d.borrow_mut();
        let idx = (vk & 0xFF) as usize;
        let was = d[idx];
        d[idx] = is_down;
        is_down && was
    });

    if RESET_FILTER.swap(false, Ordering::SeqCst) {
        FILTER.with(|f| f.borrow_mut().reset());
    }
    if PAUSED.load(Ordering::Relaxed) {
        return next();
    }

    let event = if let Some(bit) = modifier_bit(vk) {
        let mut m = MODS.with(|c| c.get());
        if is_down {
            m |= bit;
        } else {
            m &= !bit;
        }
        MODS.with(|c| c.set(m));
        KeyEvent { kind: KeyKind::ModifiersChanged, key: Key::Other, mods: mods_from_bits(m) }
    } else {
        let m = if injected { MODS.with(|c| c.get()) } else { sync_mods() };
        MODS.with(|c| c.set(m));
        let kind = if is_down { KeyKind::Down { repeat } } else { KeyKind::Up };
        KeyEvent { kind, key: classify(vk), mods: mods_from_bits(m) }
    };

    let decision = FILTER.with(|f| f.borrow_mut().handle(&event, Instant::now()));
    if let Some(action) = decision.action {
        TX.with(|tx| {
            if let Some(tx) = tx.borrow().as_ref() {
                let _ = tx.send(Msg::Key(action));
            }
        });
    }
    if decision.swallow {
        1
    } else {
        next()
    }
}

pub fn start_hook(tx: Sender<Msg>) -> Result<(), String> {
    if hook_running() {
        return Ok(());
    }
    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
    std::thread::Builder::new()
        .name("slotboard-keys".into())
        .spawn(move || unsafe {
            TX.with(|t| *t.borrow_mut() = Some(tx));
            let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), GetModuleHandleW(ptr::null()), 0);
            if hook.is_null() {
                let _ = ready_tx.send(Err("keyboard hook could not be installed".into()));
                return;
            }
            HOOK.store(hook as isize, Ordering::SeqCst);
            let _ = ready_tx.send(Ok(()));
            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, ptr::null_mut(), 0, 0) > 0 {}
        })
        .map_err(|e| e.to_string())?;
    ready_rx
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| "keyboard hook did not start".to_string())?
}

pub fn hook_running() -> bool {
    HOOK.load(Ordering::SeqCst) != 0
}

pub fn ensure_hook_enabled() {}

pub fn note_hook_failure() {}

pub fn set_paused(paused: bool) {
    PAUSED.store(paused, Ordering::SeqCst);
    RESET_FILTER.store(true, Ordering::SeqCst);
}

fn key_input(vk: u32, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        u: INPUT_UNION {
            ki: KEYBDINPUT {
                wVk: vk as u16,
                wScan: 0,
                dwFlags: if up { KEYEVENTF_KEYUP } else { 0 },
                time: 0,
                dwExtraInfo: MAGIC,
            },
        },
    }
}

/// Sends Ctrl+V to the focused app.
pub fn send_paste() {
    let mut inputs = Vec::with_capacity(8);
    // A held Alt/Shift/Win would turn Ctrl+V into a different shortcut.
    let held: Vec<u32> = [VK_LMENU, VK_RMENU, VK_LSHIFT, VK_RSHIFT, VK_LWIN, VK_RWIN]
        .into_iter()
        .filter(|vk| key_down(*vk))
        .collect();
    for vk in &held {
        inputs.push(key_input(*vk, true));
    }
    let ctrl_held = key_down(VK_LCONTROL) || key_down(VK_RCONTROL);
    if !ctrl_held {
        inputs.push(key_input(VK_LCONTROL, false));
    }
    inputs.push(key_input(VK_V, false));
    inputs.push(key_input(VK_V, true));
    if !ctrl_held {
        inputs.push(key_input(VK_LCONTROL, true));
    }
    unsafe {
        SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32);
    }
}

// ---------------------------------------------------------------------------
// Permission & misc
// ---------------------------------------------------------------------------

pub fn permission() -> Permission {
    Permission::Granted
}

pub fn request_permission() {}

pub fn open_permission_settings() {}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn open_url(url: &str) {
    let op = wide("open");
    let file = wide(url);
    unsafe {
        ShellExecuteW(ptr::null_mut(), op.as_ptr(), file.as_ptr(), ptr::null(), ptr::null(), 1);
    }
}

pub fn notice() -> Option<&'static str> {
    None
}

pub fn system_language() -> String {
    let lang = unsafe { GetUserDefaultUILanguage() } & 0x3FF;
    if lang == 0x07 {
        "de".into()
    } else {
        "en".into()
    }
}

// ---------------------------------------------------------------------------
// Clipboard
// ---------------------------------------------------------------------------

pub mod clipboard {
    use super::*;
    use crate::platform::{file_name_from_path, ContentKind};

    const CF_TEXT: u32 = 1;
    const CF_BITMAP: u32 = 2;
    const CF_METAFILEPICT: u32 = 3;
    const CF_DIB: u32 = 8;
    const CF_PALETTE: u32 = 9;
    const CF_UNICODETEXT: u32 = 13;
    const CF_ENHMETAFILE: u32 = 14;
    const CF_HDROP: u32 = 15;
    const CF_DIBV5: u32 = 17;
    const CF_OWNERDISPLAY: u32 = 0x0080;
    const CF_DSPBITMAP: u32 = 0x0082;
    const CF_DSPMETAFILEPICT: u32 = 0x0083;
    const CF_DSPENHMETAFILE: u32 = 0x008E;

    const MARKER: &str = "Slotboard.Internal";
    const EXCLUDE: &str = "ExcludeClipboardContentFromMonitorProcessing";
    const NO_HISTORY: &str = "CanIncludeInClipboardHistory";
    const NO_CLOUD: &str = "CanUploadToCloudClipboard";
    const VIEWER_IGNORE: &str = "Clipboard Viewer Ignore";

    #[derive(Clone, Debug)]
    struct Format {
        id: u32,
        /// Registered formats are stored by name.
        name: Option<String>,
        data: Vec<u8>,
    }

    #[derive(Clone, Debug)]
    pub struct Snapshot {
        formats: Vec<Format>,
        concealed: bool,
    }

    impl Snapshot {
        fn data(&self, id: u32) -> Option<&[u8]> {
            self.formats.iter().find(|f| f.id == id && f.name.is_none()).map(|f| f.data.as_slice())
        }

        fn named(&self, name: &str) -> bool {
            self.formats.iter().any(|f| f.name.as_deref() == Some(name))
        }

        pub fn is_concealed(&self) -> bool {
            self.concealed
        }

        pub fn kind(&self) -> ContentKind {
            if let Some(drop) = self.data(CF_HDROP) {
                let files = parse_hdrop(drop);
                if !files.is_empty() {
                    return ContentKind::Files(files);
                }
            }
            let has_image =
                self.data(CF_DIB).is_some() || self.data(CF_DIBV5).is_some() || self.named("PNG");
            let text = self.data(CF_UNICODETEXT).map(utf16_to_string);
            match (text, has_image) {
                (Some(t), _) if !t.trim().is_empty() => ContentKind::Text(t),
                (_, true) => ContentKind::Image,
                _ => {
                    if let Some(t) = self.data(CF_TEXT) {
                        let s = String::from_utf8_lossy(t.split(|b| *b == 0).next().unwrap_or(&[])).into_owned();
                        if !s.trim().is_empty() {
                            return ContentKind::Text(s);
                        }
                    }
                    ContentKind::Other
                }
            }
        }
    }

    fn utf16_to_string(bytes: &[u8]) -> String {
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .take_while(|u| *u != 0)
            .collect();
        String::from_utf16_lossy(&units)
    }

    /// DROPFILES header followed by a double-NUL-terminated list of paths.
    fn parse_hdrop(bytes: &[u8]) -> Vec<String> {
        if bytes.len() < 20 {
            return Vec::new();
        }
        let offset = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
        let wide = u32::from_le_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]) != 0;
        if offset >= bytes.len() {
            return Vec::new();
        }
        let list = &bytes[offset..];
        let joined = if wide {
            let units: Vec<u16> = list.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            String::from_utf16_lossy(&units)
        } else {
            String::from_utf8_lossy(list).into_owned()
        };
        joined
            .split('\0')
            .take_while(|s| !s.is_empty())
            .map(file_name_from_path)
            .collect()
    }

    fn format_name(id: u32) -> Option<String> {
        if id < 0xC000 {
            return None;
        }
        let mut buf = [0u16; 256];
        let len = unsafe { GetClipboardFormatNameW(id, buf.as_mut_ptr(), buf.len() as i32) };
        if len <= 0 {
            return None;
        }
        Some(String::from_utf16_lossy(&buf[..len as usize]))
    }

    fn skip(id: u32) -> bool {
        matches!(
            id,
            CF_BITMAP
                | CF_METAFILEPICT
                | CF_PALETTE
                | CF_ENHMETAFILE
                | CF_OWNERDISPLAY
                | CF_DSPBITMAP
                | CF_DSPMETAFILEPICT
                | CF_DSPENHMETAFILE
        )
    }

    fn open() -> bool {
        for _ in 0..20 {
            if unsafe { OpenClipboard(ptr::null_mut()) } != 0 {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    pub fn change_count() -> u64 {
        unsafe { GetClipboardSequenceNumber() as u64 }
    }

    pub fn read() -> Option<Snapshot> {
        if !open() {
            return None;
        }
        let mut formats = Vec::new();
        unsafe {
            let mut id = 0u32;
            loop {
                id = EnumClipboardFormats(id);
                if id == 0 {
                    break;
                }
                if skip(id) {
                    continue;
                }
                let handle = GetClipboardData(id);
                if handle.is_null() {
                    continue;
                }
                let size = GlobalSize(handle);
                let locked = GlobalLock(handle);
                if locked.is_null() {
                    continue;
                }
                let data = std::slice::from_raw_parts(locked as *const u8, size).to_vec();
                GlobalUnlock(handle);
                formats.push(Format { id, name: format_name(id), data });
            }
            CloseClipboard();
        }
        let ours = formats.iter().any(|f| f.name.as_deref() == Some(MARKER));
        let is_flag = |f: &Format| {
            matches!(f.name.as_deref(), Some(MARKER) | Some(EXCLUDE) | Some(NO_HISTORY) | Some(NO_CLOUD))
        };
        let concealed =
            !ours && formats.iter().any(|f| matches!(f.name.as_deref(), Some(EXCLUDE) | Some(VIEWER_IGNORE)));
        if ours {
            formats.retain(|f| !is_flag(f));
        }
        if formats.is_empty() {
            None
        } else {
            Some(Snapshot { formats, concealed })
        }
    }

    unsafe fn put(id: u32, data: &[u8]) {
        let len = data.len().max(1);
        let mem = GlobalAlloc(GMEM_MOVEABLE, len);
        if mem.is_null() {
            return;
        }
        let dst = GlobalLock(mem);
        if dst.is_null() {
            GlobalFree(mem);
            return;
        }
        ptr::write_bytes(dst as *mut u8, 0, len);
        ptr::copy_nonoverlapping(data.as_ptr(), dst as *mut u8, data.len());
        GlobalUnlock(mem);
        if SetClipboardData(id, mem).is_null() {
            GlobalFree(mem);
        }
    }

    fn register(name: &str) -> u32 {
        let w = wide(name);
        unsafe { RegisterClipboardFormatW(w.as_ptr()) }
    }

    pub fn write(snapshot: &Snapshot, transient: bool) -> bool {
        if !open() {
            return false;
        }
        unsafe {
            EmptyClipboard();
            for f in &snapshot.formats {
                let id = match &f.name {
                    Some(name) => register(name),
                    None => f.id,
                };
                if id != 0 {
                    put(id, &f.data);
                }
            }
            if transient {
                // Keep Slotboard's short-lived writes out of Win+V history.
                put(register(MARKER), &[1]);
                put(register(EXCLUDE), &[0, 0, 0, 0]);
                put(register(NO_HISTORY), &[0, 0, 0, 0]);
                put(register(NO_CLOUD), &[0, 0, 0, 0]);
            }
            CloseClipboard();
        }
        true
    }

    pub fn clear() {
        if open() {
            unsafe {
                EmptyClipboard();
                CloseClipboard();
            }
        }
    }
}
