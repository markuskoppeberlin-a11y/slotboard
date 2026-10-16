//! Linux (X11): XRecord watches the keyboard, XGrabKey hides the shortcut
//! keys from apps, XTest sends the paste shortcut. Wayland does not allow
//! global shortcuts like these, so Slotboard reports `Unsupported` there.

#![allow(non_upper_case_globals, non_camel_case_types, non_snake_case)]

use std::ffi::{c_char, c_int, c_long, c_uchar, c_uint, c_ulong, c_ushort, c_void};
use std::process::Command;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU8, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use super::Permission;
use crate::engine::Msg;
use crate::filter::{Key, KeyEvent, KeyFilter, KeyKind, Mods};

// ---------------------------------------------------------------------------
// Raw Xlib / XTest / XRecord bindings
// ---------------------------------------------------------------------------

type Display = c_void;
type Window = c_ulong;
type KeySym = c_ulong;
type KeyCode = c_uchar;
type Bool = c_int;
type XRecordContext = c_ulong;
type XRecordClientSpec = c_ulong;

#[repr(C)]
struct XErrorEvent {
    type_: c_int,
    display: *mut Display,
    resourceid: c_ulong,
    serial: c_ulong,
    error_code: c_uchar,
    request_code: c_uchar,
    minor_code: c_uchar,
}

type XErrorHandler = Option<unsafe extern "C" fn(*mut Display, *mut XErrorEvent) -> c_int>;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct XRecordRange8 {
    first: c_uchar,
    last: c_uchar,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct XRecordRange16 {
    first: c_ushort,
    last: c_ushort,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct XRecordExtRange {
    ext_major: XRecordRange8,
    ext_minor: XRecordRange16,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct XRecordRange {
    core_requests: XRecordRange8,
    core_replies: XRecordRange8,
    ext_requests: XRecordExtRange,
    ext_replies: XRecordExtRange,
    delivered_events: XRecordRange8,
    device_events: XRecordRange8,
    errors: XRecordRange8,
    client_started: Bool,
    client_died: Bool,
}

#[repr(C)]
struct XRecordInterceptData {
    id_base: c_ulong,
    server_time: c_ulong,
    client_seq: c_ulong,
    category: c_int,
    client_swapped: Bool,
    data: *mut c_uchar,
    data_len: c_ulong,
}

type XRecordInterceptProc = Option<unsafe extern "C" fn(closure: *mut c_char, data: *mut XRecordInterceptData)>;

#[link(name = "X11")]
extern "C" {
    fn XOpenDisplay(name: *const c_char) -> *mut Display;
    fn XDefaultRootWindow(dpy: *mut Display) -> Window;
    fn XKeysymToKeycode(dpy: *mut Display, keysym: KeySym) -> KeyCode;
    fn XGrabKey(
        dpy: *mut Display,
        keycode: c_int,
        modifiers: c_uint,
        grab_window: Window,
        owner_events: Bool,
        pointer_mode: c_int,
        keyboard_mode: c_int,
    ) -> c_int;
    fn XUngrabKey(dpy: *mut Display, keycode: c_int, modifiers: c_uint, grab_window: Window) -> c_int;
    fn XUngrabKeyboard(dpy: *mut Display, time: c_ulong) -> c_int;
    fn XSync(dpy: *mut Display, discard: Bool) -> c_int;
    fn XPending(dpy: *mut Display) -> c_int;
    fn XNextEvent(dpy: *mut Display, event: *mut c_long) -> c_int;
    fn XSetErrorHandler(handler: XErrorHandler) -> XErrorHandler;
    fn XFree(data: *mut c_void) -> c_int;
}

#[link(name = "Xtst")]
extern "C" {
    fn XTestFakeKeyEvent(dpy: *mut Display, keycode: c_uint, is_press: Bool, delay: c_ulong) -> c_int;
    fn XRecordAllocRange() -> *mut XRecordRange;
    fn XRecordCreateContext(
        dpy: *mut Display,
        datum_flags: c_int,
        clients: *mut XRecordClientSpec,
        nclients: c_int,
        ranges: *mut *mut XRecordRange,
        nranges: c_int,
    ) -> XRecordContext;
    fn XRecordEnableContext(
        dpy: *mut Display,
        context: XRecordContext,
        callback: XRecordInterceptProc,
        closure: *mut c_char,
    ) -> c_int;
    fn XRecordFreeData(data: *mut XRecordInterceptData);
}

const KeyPress: c_uchar = 2;
const KeyRelease: c_uchar = 3;
const XRecordFromServer: c_int = 0;
const XRecordAllClients: XRecordClientSpec = 3;
const GrabModeAsync: c_int = 1;
const CurrentTime: c_ulong = 0;

const LockMask: c_uint = 1 << 1;
const ControlMask: c_uint = 1 << 2;
const Mod2Mask: c_uint = 1 << 4; // NumLock
const LOCK_VARIANTS: [c_uint; 4] = [0, LockMask, Mod2Mask, LockMask | Mod2Mask];

// Keysyms
const XK_c: KeySym = 0x63;
const XK_v: KeySym = 0x76;
const XK_Shift_L: KeySym = 0xffe1;
const XK_Shift_R: KeySym = 0xffe2;
const XK_Control_L: KeySym = 0xffe3;
const XK_Control_R: KeySym = 0xffe4;
const XK_Alt_L: KeySym = 0xffe9;
const XK_Alt_R: KeySym = 0xffea;
const XK_Super_L: KeySym = 0xffeb;
const XK_Super_R: KeySym = 0xffec;
const XK_ISO_Level3_Shift: KeySym = 0xfe03;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum KClass {
    None,
    Digit(u8),
    C,
    V,
    CtrlL,
    CtrlR,
    AltL,
    AltGr,
    ShiftL,
    ShiftR,
    Super,
}

// Shared modifier state (bits) so the paste sender knows what is held.
const B_CTRL_L: u8 = 1;
const B_CTRL_R: u8 = 2;
const B_ALT_L: u8 = 4;
const B_ALTGR: u8 = 8;
const B_SHIFT_L: u8 = 16;
const B_SHIFT_R: u8 = 32;
const B_SUPER: u8 = 64;

static MODS: AtomicU8 = AtomicU8::new(0);
static PAUSED: AtomicBool = AtomicBool::new(false);
static RESET_FILTER: AtomicBool = AtomicBool::new(false);
static RUNNING: AtomicBool = AtomicBool::new(false);
/// V press/release events we sent ourselves and still expect to see.
static SYNTHETIC_V: AtomicUsize = AtomicUsize::new(0);
static CONTROL_DPY: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
static DATA_DPY: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
static PREV_HANDLER: AtomicUsize = AtomicUsize::new(0);
static CTL_TX: std::sync::Mutex<Option<Sender<Ctl>>> = std::sync::Mutex::new(None);

enum Ctl {
    DigitGrab(bool),
    Paste,
}

unsafe extern "C" fn error_handler(dpy: *mut Display, ev: *mut XErrorEvent) -> c_int {
    let ours = dpy == CONTROL_DPY.load(Ordering::SeqCst) || dpy == DATA_DPY.load(Ordering::SeqCst);
    if ours {
        // e.g. BadAccess when another app already grabbed a key – ignore.
        return 0;
    }
    let prev = PREV_HANDLER.load(Ordering::SeqCst);
    if prev != 0 {
        let f: unsafe extern "C" fn(*mut Display, *mut XErrorEvent) -> c_int = std::mem::transmute(prev);
        return f(dpy, ev);
    }
    0
}

struct Keys {
    table: [KClass; 256],
    digits: Vec<KeyCode>,
    v: KeyCode,
    ctrl_l: KeyCode,
    alt_l: KeyCode,
    altgr: KeyCode,
    shift_l: KeyCode,
    shift_r: KeyCode,
    super_l: KeyCode,
}

unsafe fn build_keys(dpy: *mut Display) -> Keys {
    let mut table = [KClass::None; 256];
    let mut digits = Vec::new();
    let kc = |ks: KeySym| XKeysymToKeycode(dpy, ks);
    for d in 0..10u8 {
        for ks in [0x30 + d as KeySym, 0xffb0 + d as KeySym] {
            let code = kc(ks);
            if code != 0 {
                table[code as usize] = KClass::Digit(d);
                digits.push(code);
            }
        }
    }
    let mut set = |ks: KeySym, class: KClass| -> KeyCode {
        let code = kc(ks);
        if code != 0 {
            table[code as usize] = class;
        }
        code
    };
    set(XK_c, KClass::C);
    let v = set(XK_v, KClass::V);
    let ctrl_l = set(XK_Control_L, KClass::CtrlL);
    set(XK_Control_R, KClass::CtrlR);
    let shift_l = set(XK_Shift_L, KClass::ShiftL);
    let shift_r = set(XK_Shift_R, KClass::ShiftR);
    let super_l = set(XK_Super_L, KClass::Super);
    set(XK_Super_R, KClass::Super);
    let mut altgr = set(XK_Alt_R, KClass::AltGr);
    let l3 = set(XK_ISO_Level3_Shift, KClass::AltGr);
    if l3 != 0 {
        altgr = l3;
    }
    let alt_l = set(XK_Alt_L, KClass::AltL);
    Keys { table, digits, v, ctrl_l, alt_l, altgr, shift_l, shift_r, super_l }
}

fn mods_from_bits(m: u8) -> Mods {
    Mods {
        primary: m & (B_CTRL_L | B_CTRL_R) != 0,
        alt: m & B_ALT_L != 0,
        other: m & (B_ALTGR | B_SUPER) != 0,
        shift: m & (B_SHIFT_L | B_SHIFT_R) != 0,
    }
}

struct RecordState {
    filter: KeyFilter,
    tx: Sender<Msg>,
    ctl: Sender<Ctl>,
    table: [KClass; 256],
    down: [bool; 256],
    digit_grab: bool,
}

unsafe extern "C" fn record_callback(closure: *mut c_char, data: *mut XRecordInterceptData) {
    if data.is_null() {
        return;
    }
    if !closure.is_null() && (*data).category == XRecordFromServer && !(*data).data.is_null() {
        let state = &mut *(closure as *mut RecordState);
        let bytes = (*data).data;
        let ev_type = *bytes & 0x7f;
        let keycode = *bytes.add(1);
        if ev_type == KeyPress || ev_type == KeyRelease {
            handle_key(state, ev_type == KeyPress, keycode);
        }
    }
    XRecordFreeData(data);
}

fn handle_key(state: &mut RecordState, press: bool, keycode: u8) {
    let idx = keycode as usize;
    let class = state.table[idx];
    let repeat = press && state.down[idx];
    state.down[idx] = press;

    if class == KClass::V {
        let pending = SYNTHETIC_V.load(Ordering::SeqCst);
        if pending > 0 {
            SYNTHETIC_V.store(pending - 1, Ordering::SeqCst);
            return;
        }
    }
    if RESET_FILTER.swap(false, Ordering::SeqCst) {
        state.filter.reset();
    }

    let bit = match class {
        KClass::CtrlL => Some(B_CTRL_L),
        KClass::CtrlR => Some(B_CTRL_R),
        KClass::AltL => Some(B_ALT_L),
        KClass::AltGr => Some(B_ALTGR),
        KClass::ShiftL => Some(B_SHIFT_L),
        KClass::ShiftR => Some(B_SHIFT_R),
        KClass::Super => Some(B_SUPER),
        _ => None,
    };
    let mut m = MODS.load(Ordering::SeqCst);
    if let Some(b) = bit {
        if press {
            m |= b;
        } else {
            m &= !b;
        }
        MODS.store(m, Ordering::SeqCst);
    }
    if PAUSED.load(Ordering::Relaxed) {
        update_digit_grab(state, false);
        return;
    }

    let mods = mods_from_bits(m);
    let event = if bit.is_some() {
        KeyEvent { kind: KeyKind::ModifiersChanged, key: Key::Other, mods }
    } else {
        let key = match class {
            KClass::Digit(d) => Key::Digit(d),
            KClass::C => Key::C,
            KClass::V => Key::V,
            _ => Key::Other,
        };
        let kind = if press { KeyKind::Down { repeat } } else { KeyKind::Up };
        KeyEvent { kind, key, mods }
    };
    let decision = state.filter.handle(&event, Instant::now());
    if let Some(action) = decision.action {
        let _ = state.tx.send(Msg::Key(action));
    }
    let want = state.filter.is_copy_armed() || state.filter.is_paste_armed();
    update_digit_grab(state, want);
}

fn update_digit_grab(state: &mut RecordState, want: bool) {
    if want != state.digit_grab {
        state.digit_grab = want;
        let _ = state.ctl.send(Ctl::DigitGrab(want));
    }
}

unsafe fn grab_ctrl(dpy: *mut Display, root: Window, code: KeyCode, grab: bool) {
    if code == 0 {
        return;
    }
    for extra in LOCK_VARIANTS {
        if grab {
            XGrabKey(dpy, code as c_int, ControlMask | extra, root, 0, GrabModeAsync, GrabModeAsync);
        } else {
            XUngrabKey(dpy, code as c_int, ControlMask | extra, root);
        }
    }
}

fn control_loop(dpy: *mut Display, keys: Keys, rx: Receiver<Ctl>) {
    unsafe {
        let root = XDefaultRootWindow(dpy);
        grab_ctrl(dpy, root, keys.v, true);
        XSync(dpy, 0);
        let mut digits_grabbed = false;
        let mut grabbed_at = Instant::now();
        let mut event = [0 as c_long; 24];
        loop {
            while XPending(dpy) > 0 {
                XNextEvent(dpy, event.as_mut_ptr());
            }
            // Safety net: never keep Ctrl+digit grabbed longer than needed.
            if digits_grabbed && grabbed_at.elapsed() > Duration::from_secs(30) {
                for code in &keys.digits {
                    grab_ctrl(dpy, root, *code, false);
                }
                XSync(dpy, 0);
                digits_grabbed = false;
            }
            match rx.recv_timeout(Duration::from_millis(5)) {
                Ok(Ctl::DigitGrab(want)) => {
                    if want != digits_grabbed {
                        for code in &keys.digits {
                            grab_ctrl(dpy, root, *code, want);
                        }
                        XSync(dpy, 0);
                        digits_grabbed = want;
                        grabbed_at = Instant::now();
                    }
                }
                Ok(Ctl::Paste) => {
                    XUngrabKeyboard(dpy, CurrentTime);
                    grab_ctrl(dpy, root, keys.v, false);
                    XSync(dpy, 0);
                    let m = MODS.load(Ordering::SeqCst);
                    for (bit, code) in [
                        (B_ALT_L, keys.alt_l),
                        (B_ALTGR, keys.altgr),
                        (B_SHIFT_L, keys.shift_l),
                        (B_SHIFT_R, keys.shift_r),
                        (B_SUPER, keys.super_l),
                    ] {
                        if m & bit != 0 && code != 0 {
                            XTestFakeKeyEvent(dpy, code as c_uint, 0, 0);
                        }
                    }
                    let ctrl_held = m & (B_CTRL_L | B_CTRL_R) != 0;
                    if !ctrl_held {
                        XTestFakeKeyEvent(dpy, keys.ctrl_l as c_uint, 1, 0);
                    }
                    SYNTHETIC_V.fetch_add(2, Ordering::SeqCst);
                    XTestFakeKeyEvent(dpy, keys.v as c_uint, 1, 0);
                    XTestFakeKeyEvent(dpy, keys.v as c_uint, 0, 0);
                    if !ctrl_held {
                        XTestFakeKeyEvent(dpy, keys.ctrl_l as c_uint, 0, 0);
                    }
                    XSync(dpy, 0);
                    grab_ctrl(dpy, root, keys.v, true);
                    XSync(dpy, 0);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }
    }
}

fn is_wayland() -> bool {
    std::env::var("XDG_SESSION_TYPE").map(|v| v.eq_ignore_ascii_case("wayland")).unwrap_or(false)
        || std::env::var_os("WAYLAND_DISPLAY").is_some()
}

pub fn start_hook(tx: Sender<Msg>) -> Result<(), String> {
    if hook_running() {
        return Ok(());
    }
    if is_wayland() {
        return Err("Wayland does not allow global shortcuts".into());
    }
    unsafe {
        let control = XOpenDisplay(ptr::null());
        let data = XOpenDisplay(ptr::null());
        if control.is_null() || data.is_null() {
            return Err("cannot connect to the X server".into());
        }
        CONTROL_DPY.store(control, Ordering::SeqCst);
        DATA_DPY.store(data, Ordering::SeqCst);
        let prev = XSetErrorHandler(Some(error_handler));
        if let Some(p) = prev {
            if p as usize != error_handler as *const () as usize {
                PREV_HANDLER.store(p as usize, Ordering::SeqCst);
            }
        }

        let keys = build_keys(control);
        let table = keys.table;

        let range = XRecordAllocRange();
        if range.is_null() {
            return Err("XRecord is not available".into());
        }
        (*range).device_events = XRecordRange8 { first: KeyPress, last: KeyRelease };
        let mut clients: XRecordClientSpec = XRecordAllClients;
        let mut ranges = range;
        let context = XRecordCreateContext(control, 0, &mut clients, 1, &mut ranges, 1);
        XFree(range as *mut c_void);
        if context == 0 {
            return Err("XRecord context could not be created".into());
        }
        XSync(control, 0);

        let (ctl_tx, ctl_rx) = mpsc::channel::<Ctl>();
        *CTL_TX.lock().map_err(|_| "lock")? = Some(ctl_tx.clone());

        let control_addr = control as usize;
        std::thread::Builder::new()
            .name("slotboard-x11".into())
            .spawn(move || control_loop(control_addr as *mut Display, keys, ctl_rx))
            .map_err(|e| e.to_string())?;

        let state = Box::into_raw(Box::new(RecordState {
            filter: KeyFilter::new(),
            tx,
            ctl: ctl_tx,
            table,
            down: [false; 256],
            digit_grab: false,
        }));
        let data_addr = data as usize;
        let state_addr = state as usize;
        std::thread::Builder::new()
            .name("slotboard-keys".into())
            .spawn(move || {
                RUNNING.store(true, Ordering::SeqCst);
                XRecordEnableContext(
                    data_addr as *mut Display,
                    context,
                    Some(record_callback),
                    state_addr as *mut c_char,
                );
                RUNNING.store(false, Ordering::SeqCst);
            })
            .map_err(|e| e.to_string())?;
    }
    std::thread::sleep(Duration::from_millis(50));
    Ok(())
}

pub fn hook_running() -> bool {
    RUNNING.load(Ordering::SeqCst)
}

pub fn ensure_hook_enabled() {}

pub fn note_hook_failure() {}

pub fn set_paused(paused: bool) {
    PAUSED.store(paused, Ordering::SeqCst);
    RESET_FILTER.store(true, Ordering::SeqCst);
}

pub fn send_paste() {
    if let Ok(guard) = CTL_TX.lock() {
        if let Some(tx) = guard.as_ref() {
            let _ = tx.send(Ctl::Paste);
            return;
        }
    }
}

pub fn permission() -> Permission {
    if is_wayland() {
        Permission::Unsupported
    } else {
        Permission::Granted
    }
}

pub fn request_permission() {}

pub fn open_permission_settings() {}

pub fn open_url(url: &str) {
    let _ = Command::new("xdg-open").arg(url).spawn();
}

pub fn notice() -> Option<&'static str> {
    if is_wayland() {
        Some("wayland")
    } else {
        None
    }
}

pub fn system_language() -> String {
    for var in ["LC_ALL", "LC_MESSAGES", "LANG", "LANGUAGE"] {
        if let Ok(v) = std::env::var(var) {
            if !v.is_empty() {
                return v;
            }
        }
    }
    String::new()
}

// ---------------------------------------------------------------------------
// Clipboard (arboard keeps serving the X11 selection in the background)
// ---------------------------------------------------------------------------

pub mod clipboard {
    use std::borrow::Cow;
    use std::cell::RefCell;
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use std::path::PathBuf;

    use crate::platform::{file_name_from_path, ContentKind};

    thread_local! {
        static CLIPBOARD: RefCell<Option<arboard::Clipboard>> = const { RefCell::new(None) };
    }

    fn with<R>(f: impl FnOnce(&mut arboard::Clipboard) -> R) -> Option<R> {
        CLIPBOARD.with(|cell| {
            let mut slot = cell.borrow_mut();
            if slot.is_none() {
                *slot = arboard::Clipboard::new().ok();
            }
            slot.as_mut().map(f)
        })
    }

    #[derive(Clone, Debug)]
    pub struct Snapshot {
        text: Option<String>,
        html: Option<String>,
        image: Option<(usize, usize, Vec<u8>)>,
        files: Vec<PathBuf>,
    }

    impl Snapshot {
        pub fn is_concealed(&self) -> bool {
            false
        }

        pub fn kind(&self) -> ContentKind {
            if !self.files.is_empty() {
                return ContentKind::Files(
                    self.files.iter().map(|p| file_name_from_path(&p.to_string_lossy())).collect(),
                );
            }
            if let Some(t) = &self.text {
                if !t.trim().is_empty() {
                    return ContentKind::Text(t.clone());
                }
            }
            if self.image.is_some() {
                return ContentKind::Image;
            }
            ContentKind::Other
        }
    }

    pub fn change_count() -> u64 {
        with(|cb| {
            let mut h = DefaultHasher::new();
            cb.get_text().ok().hash(&mut h);
            h.finish()
        })
        .unwrap_or(0)
    }

    pub fn read() -> Option<Snapshot> {
        with(|cb| {
            let files = cb.get().file_list().unwrap_or_default();
            let text = cb.get_text().ok();
            let html = cb.get().html().ok();
            let image = if text.is_none() && files.is_empty() {
                cb.get_image().ok().map(|img| (img.width, img.height, img.bytes.into_owned()))
            } else {
                None
            };
            if text.is_none() && html.is_none() && image.is_none() && files.is_empty() {
                None
            } else {
                Some(Snapshot { text, html, image, files })
            }
        })
        .flatten()
    }

    pub fn write(snapshot: &Snapshot, _transient: bool) -> bool {
        with(|cb| {
            if !snapshot.files.is_empty() {
                return cb.set().file_list(&snapshot.files).is_ok();
            }
            if let Some((width, height, bytes)) = &snapshot.image {
                return cb
                    .set_image(arboard::ImageData { width: *width, height: *height, bytes: Cow::Borrowed(bytes) })
                    .is_ok();
            }
            if let Some(html) = &snapshot.html {
                return cb.set_html(html.as_str(), snapshot.text.as_deref()).is_ok();
            }
            if let Some(text) = &snapshot.text {
                return cb.set_text(text.as_str()).is_ok();
            }
            false
        })
        .unwrap_or(false)
    }

    pub fn clear() {
        let _ = with(|cb| cb.clear());
    }
}
