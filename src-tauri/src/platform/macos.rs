//! macOS: CGEventTap keyboard hook, NSPasteboard with every format, and
//! Accessibility permission handling.

use std::ffi::c_void;
use std::process::Command;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicPtr, AtomicU16, Ordering};
use std::sync::mpsc::{self, Sender};
use std::time::{Duration, Instant};

use super::Permission;
use crate::engine::Msg;
use crate::filter::{Key, KeyEvent, KeyFilter, KeyKind, Mods};

pub const BUNDLE_ID: &str = "com.markuskoppe.slotboard";

// ---------------------------------------------------------------------------
// Raw CoreGraphics / CoreFoundation / ApplicationServices bindings
// ---------------------------------------------------------------------------

type CGEventRef = *mut c_void;
type CGEventSourceRef = *mut c_void;
type CGEventTapProxy = *mut c_void;
type CFMachPortRef = *mut c_void;
type CFRunLoopSourceRef = *mut c_void;
type CFRunLoopRef = *mut c_void;
type CFTypeRef = *const c_void;
type CFStringRef = *const c_void;
type CFDictionaryRef = *const c_void;
type CFAllocatorRef = *const c_void;

type TapCallback =
    extern "C" fn(proxy: CGEventTapProxy, etype: u32, event: CGEventRef, info: *mut c_void) -> CGEventRef;

#[repr(C)]
struct Opaque {
    _private: [u8; 0],
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        events_of_interest: u64,
        callback: TapCallback,
        user_info: *mut c_void,
    ) -> CFMachPortRef;
    fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
    fn CGEventTapIsEnabled(tap: CFMachPortRef) -> bool;
    fn CGEventGetIntegerValueField(event: CGEventRef, field: u32) -> i64;
    fn CGEventSetIntegerValueField(event: CGEventRef, field: u32, value: i64);
    fn CGEventGetFlags(event: CGEventRef) -> u64;
    fn CGEventSetFlags(event: CGEventRef, flags: u64);
    fn CGEventKeyboardGetUnicodeString(
        event: CGEventRef,
        max_len: usize,
        actual_len: *mut usize,
        buffer: *mut u16,
    );
    fn CGEventCreateKeyboardEvent(source: CGEventSourceRef, keycode: u16, keydown: bool) -> CGEventRef;
    fn CGEventPost(tap: u32, event: CGEventRef);
    fn CGEventSourceCreate(state: i32) -> CGEventSourceRef;
    fn CGEventSourceGetSourceStateID(source: CGEventSourceRef) -> i32;
    fn CGRequestListenEventAccess() -> bool;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFMachPortCreateRunLoopSource(alloc: CFAllocatorRef, port: CFMachPortRef, order: isize) -> CFRunLoopSourceRef;
    fn CFRunLoopGetCurrent() -> CFRunLoopRef;
    fn CFRunLoopAddSource(rl: CFRunLoopRef, source: CFRunLoopSourceRef, mode: CFStringRef);
    fn CFRunLoopRun();
    fn CFRelease(cf: CFTypeRef);
    fn CFDictionaryCreate(
        alloc: CFAllocatorRef,
        keys: *const *const c_void,
        values: *const *const c_void,
        num_values: isize,
        key_callbacks: *const c_void,
        value_callbacks: *const c_void,
    ) -> CFDictionaryRef;
    static kCFRunLoopCommonModes: CFStringRef;
    static kCFBooleanTrue: CFTypeRef;
    static kCFTypeDictionaryKeyCallBacks: Opaque;
    static kCFTypeDictionaryValueCallBacks: Opaque;
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> bool;
    static kAXTrustedCheckOptionPrompt: CFStringRef;
}

const SESSION_TAP: u32 = 1; // kCGSessionEventTap
const HID_TAP: u32 = 0; // kCGHIDEventTap
const HEAD_INSERT: u32 = 0; // kCGHeadInsertEventTap
const TAP_OPTION_DEFAULT: u32 = 0; // active filter, may drop events

const EV_KEY_DOWN: u32 = 10;
const EV_KEY_UP: u32 = 11;
const EV_FLAGS_CHANGED: u32 = 12;
const EV_TAP_DISABLED_BY_TIMEOUT: u32 = 0xFFFF_FFFE;
const EV_TAP_DISABLED_BY_USER: u32 = 0xFFFF_FFFF;

const FIELD_AUTOREPEAT: u32 = 8; // kCGKeyboardEventAutorepeat
const FIELD_KEYCODE: u32 = 9; // kCGKeyboardEventKeycode
const FIELD_USER_DATA: u32 = 42; // kCGEventSourceUserData
const FIELD_SOURCE_STATE_ID: u32 = 45; // kCGEventSourceStateID

const FLAG_SHIFT: u64 = 0x0002_0000;
const FLAG_CONTROL: u64 = 0x0004_0000;
const FLAG_OPTION: u64 = 0x0008_0000;
const FLAG_COMMAND: u64 = 0x0010_0000;

const SOURCE_STATE_PRIVATE: i32 = -1;

/// Marks events Slotboard sends itself so the hook lets them through.
const MAGIC: i64 = 0x534C_4F54; // "SLOT"

const KC_C: u16 = 8;
const KC_V: u16 = 9;

// ---------------------------------------------------------------------------
// Keyboard hook
// ---------------------------------------------------------------------------

static TAP_PORT: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
static PAUSED: AtomicBool = AtomicBool::new(false);
static RESET_FILTER: AtomicBool = AtomicBool::new(false);
/// Physical key that produced "v" the last time – reused for the paste shortcut,
/// so it also works with layouts like Dvorak.
static V_KEYCODE: AtomicU16 = AtomicU16::new(KC_V);
static OUR_SOURCE_STATE: AtomicI32 = AtomicI32::new(i32::MIN);
static OUR_SOURCE: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());

struct TapState {
    filter: KeyFilter,
    tx: Sender<Msg>,
}

fn digit_for_keycode(kc: u16) -> Option<u8> {
    Some(match kc {
        29 | 82 => 0,
        18 | 83 => 1,
        19 | 84 => 2,
        20 | 85 => 3,
        21 | 86 => 4,
        23 | 87 => 5,
        22 | 88 => 6,
        26 | 89 => 7,
        28 | 91 => 8,
        25 | 92 => 9,
        _ => return None,
    })
}

unsafe fn classify(event: CGEventRef) -> (Key, u16) {
    let kc = CGEventGetIntegerValueField(event, FIELD_KEYCODE) as u16;
    if let Some(d) = digit_for_keycode(kc) {
        return (Key::Digit(d), kc);
    }
    // Letters follow the keyboard layout (QWERTZ, AZERTY, Dvorak …).
    let mut buf = [0u16; 4];
    let mut len: usize = 0;
    CGEventKeyboardGetUnicodeString(event, buf.len(), &mut len, buf.as_mut_ptr());
    if len == 1 {
        match buf[0] {
            0x63 | 0x43 => return (Key::C, kc), // c C
            0x76 | 0x56 => return (Key::V, kc), // v V
            _ => {}
        }
        if char::from_u32(buf[0] as u32).is_some_and(|c| c.is_alphabetic()) {
            return (Key::Other, kc);
        }
    }
    let key = match kc {
        KC_C => Key::C,
        KC_V => Key::V,
        _ => Key::Other,
    };
    (key, kc)
}

extern "C" fn tap_callback(_proxy: CGEventTapProxy, etype: u32, event: CGEventRef, info: *mut c_void) -> CGEventRef {
    if info.is_null() {
        return event;
    }
    // SAFETY: `info` is the `TapState` leaked in `start_hook`; this callback
    // only ever runs on the hook thread.
    let state = unsafe { &mut *(info as *mut TapState) };

    if etype == EV_TAP_DISABLED_BY_TIMEOUT || etype == EV_TAP_DISABLED_BY_USER {
        let port = TAP_PORT.load(Ordering::SeqCst);
        if !port.is_null() {
            unsafe { CGEventTapEnable(port, true) };
        }
        state.filter.reset();
        return event;
    }
    if RESET_FILTER.swap(false, Ordering::SeqCst) {
        state.filter.reset();
    }
    if event.is_null() || PAUSED.load(Ordering::Relaxed) {
        return event;
    }

    unsafe {
        if CGEventGetIntegerValueField(event, FIELD_USER_DATA) == MAGIC {
            return event;
        }
        let our_state = OUR_SOURCE_STATE.load(Ordering::Relaxed);
        if our_state != i32::MIN
            && CGEventGetIntegerValueField(event, FIELD_SOURCE_STATE_ID) == our_state as i64
        {
            return event;
        }

        let flags = CGEventGetFlags(event);
        let mods = Mods {
            primary: flags & FLAG_COMMAND != 0,
            alt: flags & FLAG_OPTION != 0,
            other: flags & FLAG_CONTROL != 0,
            shift: flags & FLAG_SHIFT != 0,
        };
        let (kind, key) = match etype {
            EV_KEY_DOWN => {
                let repeat = CGEventGetIntegerValueField(event, FIELD_AUTOREPEAT) != 0;
                let (key, kc) = classify(event);
                if key == Key::V && mods.only_primary() {
                    V_KEYCODE.store(kc, Ordering::Relaxed);
                }
                (KeyKind::Down { repeat }, key)
            }
            EV_KEY_UP => (KeyKind::Up, classify(event).0),
            EV_FLAGS_CHANGED => (KeyKind::ModifiersChanged, Key::Other),
            _ => return event,
        };

        let decision = state.filter.handle(&KeyEvent { kind, key, mods }, Instant::now());
        if let Some(action) = decision.action {
            let _ = state.tx.send(Msg::Key(action));
        }
        if decision.swallow {
            ptr::null_mut()
        } else {
            event
        }
    }
}

/// Starts the keyboard hook on its own thread. Fails when macOS refuses to
/// create the event tap (permission missing).
pub fn start_hook(tx: Sender<Msg>) -> Result<(), String> {
    if hook_running() {
        return Ok(());
    }
    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
    std::thread::Builder::new()
        .name("slotboard-keys".into())
        .spawn(move || unsafe {
            let state = Box::into_raw(Box::new(TapState { filter: KeyFilter::new(), tx }));
            let mask = (1u64 << EV_KEY_DOWN) | (1u64 << EV_KEY_UP) | (1u64 << EV_FLAGS_CHANGED);
            let port = CGEventTapCreate(
                SESSION_TAP,
                HEAD_INSERT,
                TAP_OPTION_DEFAULT,
                mask,
                tap_callback,
                state as *mut c_void,
            );
            if port.is_null() {
                drop(Box::from_raw(state));
                let _ = ready_tx.send(Err("event tap could not be created".into()));
                return;
            }
            TAP_PORT.store(port, Ordering::SeqCst);
            let source = CFMachPortCreateRunLoopSource(ptr::null(), port, 0);
            CFRunLoopAddSource(CFRunLoopGetCurrent(), source, kCFRunLoopCommonModes);
            CGEventTapEnable(port, true);
            let _ = ready_tx.send(Ok(()));
            CFRunLoopRun();
        })
        .map_err(|e| e.to_string())?;
    ready_rx
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| "event tap did not start".to_string())?
}

pub fn hook_running() -> bool {
    !TAP_PORT.load(Ordering::SeqCst).is_null()
}

/// Re-enables the tap, e.g. after the permission was revoked and granted again.
pub fn ensure_hook_enabled() {
    let port = TAP_PORT.load(Ordering::SeqCst);
    if !port.is_null() {
        unsafe {
            if !CGEventTapIsEnabled(port) {
                CGEventTapEnable(port, true);
            }
        }
    }
}

pub fn set_paused(paused: bool) {
    PAUSED.store(paused, Ordering::SeqCst);
    RESET_FILTER.store(true, Ordering::SeqCst);
}

fn our_source() -> CGEventSourceRef {
    let existing = OUR_SOURCE.load(Ordering::SeqCst);
    if !existing.is_null() {
        return existing;
    }
    unsafe {
        let source = CGEventSourceCreate(SOURCE_STATE_PRIVATE);
        if source.is_null() {
            return ptr::null_mut();
        }
        match OUR_SOURCE.compare_exchange(ptr::null_mut(), source, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => {
                OUR_SOURCE_STATE.store(CGEventSourceGetSourceStateID(source), Ordering::SeqCst);
                source
            }
            Err(other) => {
                CFRelease(source as CFTypeRef);
                other
            }
        }
    }
}

/// Sends ⌘V to the frontmost app.
pub fn send_paste() {
    let source = our_source();
    let keycode = V_KEYCODE.load(Ordering::Relaxed);
    unsafe {
        for down in [true, false] {
            let event = CGEventCreateKeyboardEvent(source, keycode, down);
            if event.is_null() {
                continue;
            }
            CGEventSetFlags(event, FLAG_COMMAND);
            CGEventSetIntegerValueField(event, FIELD_USER_DATA, MAGIC);
            CGEventPost(HID_TAP, event);
            CFRelease(event as CFTypeRef);
        }
    }
}

// ---------------------------------------------------------------------------
// Permission
// ---------------------------------------------------------------------------

static TAP_FAILED_WHILE_TRUSTED: AtomicBool = AtomicBool::new(false);
static TCC_RESET_DONE: AtomicBool = AtomicBool::new(false);

pub fn permission() -> Permission {
    let trusted = unsafe { AXIsProcessTrusted() };
    if !trusted {
        return Permission::Missing;
    }
    if !hook_running() && TAP_FAILED_WHILE_TRUSTED.load(Ordering::SeqCst) {
        return Permission::InputMonitoring;
    }
    Permission::Granted
}

/// Called by the engine when the tap could not be created although
/// Accessibility is granted.
pub fn note_hook_failure() {
    if unsafe { AXIsProcessTrusted() } {
        if !TAP_FAILED_WHILE_TRUSTED.swap(true, Ordering::SeqCst) {
            unsafe {
                CGRequestListenEventAccess();
            }
        }
    }
}

/// Shows the system prompt that adds Slotboard to the Accessibility list.
pub fn request_permission() {
    let trusted = unsafe { AXIsProcessTrusted() };
    if trusted {
        if TAP_FAILED_WHILE_TRUSTED.load(Ordering::SeqCst) {
            unsafe {
                CGRequestListenEventAccess();
            }
            open_url("x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent");
        }
        return;
    }
    // After an update the old (now invalid) entry stays in the list and looks
    // switched on. Remove it once so the user only sees the current app.
    if !TCC_RESET_DONE.swap(true, Ordering::SeqCst) {
        let _ = Command::new("/usr/bin/tccutil").args(["reset", "Accessibility", BUNDLE_ID]).output();
    }
    unsafe {
        let keys = [kAXTrustedCheckOptionPrompt as *const c_void];
        let values = [kCFBooleanTrue];
        let dict = CFDictionaryCreate(
            ptr::null(),
            keys.as_ptr(),
            values.as_ptr(),
            1,
            &kCFTypeDictionaryKeyCallBacks as *const Opaque as *const c_void,
            &kCFTypeDictionaryValueCallBacks as *const Opaque as *const c_void,
        );
        AXIsProcessTrustedWithOptions(dict);
        if !dict.is_null() {
            CFRelease(dict);
        }
    }
}

pub fn open_permission_settings() {
    if permission() == Permission::InputMonitoring {
        open_url("x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent");
    } else {
        open_url("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility");
    }
}

pub fn open_url(url: &str) {
    let _ = Command::new("/usr/bin/open").arg(url).spawn();
}

pub fn notice() -> Option<&'static str> {
    None
}

pub fn system_language() -> String {
    use objc2_foundation::NSLocale;
    objc2::rc::autoreleasepool(|_| {
        NSLocale::preferredLanguages()
            .to_vec()
            .first()
            .map(|l| l.to_string())
            .unwrap_or_default()
    })
}

// ---------------------------------------------------------------------------
// Clipboard
// ---------------------------------------------------------------------------

pub mod clipboard {
    use objc2::rc::{autoreleasepool, Retained};
    use objc2::runtime::ProtocolObject;
    use objc2_app_kit::{NSPasteboard, NSPasteboardItem, NSPasteboardWriting};
    use objc2_foundation::{NSArray, NSData, NSString};

    use crate::platform::{file_name_from_url, ContentKind};

    const MARKER: &str = "com.markuskoppe.slotboard.internal";
    const TRANSIENT: &str = "org.nspasteboard.TransientType";
    const CONCEALED: &str = "org.nspasteboard.ConcealedType";
    const TEXT: &str = "public.utf8-plain-text";
    const FILE_URL: &str = "public.file-url";
    const IMAGE_TYPES: [&str; 5] = ["public.png", "public.tiff", "public.jpeg", "public.heic", "com.adobe.pdf"];

    /// Every item on the pasteboard with every type and its raw data.
    #[derive(Clone, Debug)]
    pub struct Snapshot {
        items: Vec<Vec<(String, Vec<u8>)>>,
    }

    impl Snapshot {
        fn find(&self, ty: &str) -> Option<&[u8]> {
            self.items
                .iter()
                .flat_map(|item| item.iter())
                .find(|(t, _)| t == ty)
                .map(|(_, d)| d.as_slice())
        }

        fn has(&self, ty: &str) -> bool {
            self.find(ty).is_some()
        }

        /// Password managers mark passwords as concealed – those stay out.
        pub fn is_concealed(&self) -> bool {
            self.has(CONCEALED)
        }

        pub fn kind(&self) -> ContentKind {
            let files: Vec<String> = self
                .items
                .iter()
                .filter_map(|item| item.iter().find(|(t, _)| t == FILE_URL))
                .map(|(_, d)| file_name_from_url(&String::from_utf8_lossy(d)))
                .collect();
            if !files.is_empty() {
                return ContentKind::Files(files);
            }
            if IMAGE_TYPES.iter().any(|t| self.has(t)) && !self.has(TEXT) {
                return ContentKind::Image;
            }
            if let Some(text) = self.find(TEXT) {
                return ContentKind::Text(String::from_utf8_lossy(text).into_owned());
            }
            if IMAGE_TYPES.iter().any(|t| self.has(t)) {
                return ContentKind::Image;
            }
            ContentKind::Other
        }
    }

    pub fn change_count() -> u64 {
        autoreleasepool(|_| NSPasteboard::generalPasteboard().changeCount() as u64)
    }

    pub fn read() -> Option<Snapshot> {
        autoreleasepool(|_| {
            let pasteboard = NSPasteboard::generalPasteboard();
            let items = pasteboard.pasteboardItems()?;
            let mut out = Vec::new();
            for item in items.to_vec() {
                let mut entries = Vec::new();
                for ty in item.types().to_vec() {
                    let name = ty.to_string();
                    if name == MARKER || name == TRANSIENT {
                        continue;
                    }
                    if let Some(data) = item.dataForType(&ty) {
                        entries.push((name, data.to_vec()));
                    }
                }
                if !entries.is_empty() {
                    out.push(entries);
                }
            }
            if out.is_empty() {
                None
            } else {
                Some(Snapshot { items: out })
            }
        })
    }

    /// Puts a snapshot on the pasteboard. `transient` tells other clipboard
    /// managers to ignore this write.
    pub fn write(snapshot: &Snapshot, transient: bool) -> bool {
        autoreleasepool(|_| {
            let pasteboard = NSPasteboard::generalPasteboard();
            let mut objects: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> = Vec::new();
            for (index, entries) in snapshot.items.iter().enumerate() {
                let item = NSPasteboardItem::new();
                for (ty, data) in entries {
                    let ty = NSString::from_str(ty);
                    let data = NSData::with_bytes(data);
                    item.setData_forType(&data, &ty);
                }
                if transient && index == 0 {
                    let empty = NSData::with_bytes(&[]);
                    item.setData_forType(&empty, &NSString::from_str(TRANSIENT));
                    item.setData_forType(&empty, &NSString::from_str(MARKER));
                }
                objects.push(ProtocolObject::from_retained(item));
            }
            let array = NSArray::from_retained_slice(&objects);
            pasteboard.clearContents();
            pasteboard.writeObjects(&array)
        })
    }

    pub fn clear() {
        autoreleasepool(|_| {
            NSPasteboard::generalPasteboard().clearContents();
        });
    }
}
