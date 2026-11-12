// Windows plumbing: single instance + show/exit events, tray icon on its own thread (keeps working while the
// egui window is hidden), autostart Run key, dark title bar.
use crate::util::log;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::OnceLock;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};
use windows_sys::Win32::System::Registry::*;
use windows_sys::Win32::System::Threading::*;
use windows_sys::Win32::UI::Shell::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

pub fn w(s: &str) -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() }

const DEV: bool = cfg!(feature = "dev");
fn obj(name: &str) -> String { format!(r"Local\T3Presence{}.{}", if DEV { ".Dev" } else { "" }, name) }

// ---------- single instance ----------
static MUTEX: AtomicIsize = AtomicIsize::new(0);
/// true when this is the first copy.
pub fn claim_singleton() -> bool {
    unsafe {
        let h = CreateMutexW(std::ptr::null(), 1, w(&obj("Singleton")).as_ptr());
        let first = GetLastError() != ERROR_ALREADY_EXISTS;
        MUTEX.store(h as isize, Ordering::SeqCst);
        first
    }
}
pub fn release_singleton() {
    let h = MUTEX.swap(0, Ordering::SeqCst);
    if h != 0 { unsafe { ReleaseMutex(h as HANDLE); CloseHandle(h as HANDLE); } }
}
fn event(name: &str) -> HANDLE { unsafe { CreateEventW(std::ptr::null(), 0, 0, w(&obj(name)).as_ptr()) } }
pub fn signal(name: &str) { unsafe { let e = event(name); SetEvent(e); CloseHandle(e); } }
/// Calls f every time another launch signals the named event.
pub fn on_event(name: &'static str, f: impl Fn() + Send + 'static) {
    let e = event(name) as isize;
    std::thread::spawn(move || loop {
        if unsafe { WaitForSingleObject(e as HANDLE, INFINITE) } != WAIT_OBJECT_0 { return; }
        f();
    });
}

// ---------- window ----------
pub static MAIN_HWND: AtomicIsize = AtomicIsize::new(0);
pub fn show_main() {
    let h = MAIN_HWND.load(Ordering::SeqCst) as HWND;
    if h.is_null() { return; }
    unsafe {
        ShowWindow(h, if IsIconic(h) != 0 { SW_RESTORE } else { SW_SHOW });
        SetForegroundWindow(h);
    }
}
pub fn hide_main() {
    let h = MAIN_HWND.load(Ordering::SeqCst) as HWND;
    if !h.is_null() { unsafe { ShowWindow(h, SW_HIDE); } }
}
pub fn dark_title(hwnd: isize) {
    unsafe {
        let on: i32 = 1;
        DwmSetWindowAttribute(hwnd as HWND, 20, &on as *const _ as _, 4); // dark title bar
        let caption: u32 = 0x0022_1F1E; // COLORREF is BGR: #1E1F22
        DwmSetWindowAttribute(hwnd as HWND, 35, &caption as *const _ as _, 4);
        let border: u32 = 0x0030_2D2B;
        DwmSetWindowAttribute(hwnd as HWND, 34, &border as *const _ as _, 4);
    }
}
/// Undocumented uxtheme SetPreferredAppMode(ForceDark) + FlushMenuThemes: dark tray menu like the C# one.
fn dark_menus() {
    unsafe {
        let m = LoadLibraryW(w("uxtheme.dll").as_ptr());
        if m.is_null() { return; }
        if let Some(f) = GetProcAddress(m, 135 as _) { let f: extern "system" fn(i32) -> i32 = std::mem::transmute(f); f(2); }
        if let Some(f) = GetProcAddress(m, 136 as _) { let f: extern "system" fn() = std::mem::transmute(f); f(); }
    }
}

// ---------- autostart ----------
const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_NAME: &str = "T3Presence";
fn command() -> String { format!("\"{}\" --minimized", std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default()) }
pub fn autostart_enabled() -> bool {
    unsafe {
        let mut k: HKEY = std::ptr::null_mut();
        if RegOpenKeyExW(HKEY_CURRENT_USER, w(RUN).as_ptr(), 0, KEY_READ, &mut k) != 0 { return false; }
        let r = RegQueryValueExW(k, w(RUN_NAME).as_ptr(), std::ptr::null(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut());
        RegCloseKey(k);
        r == 0
    }
}
pub fn set_autostart(on: bool) {
    if DEV { log("dev build: autostart left alone"); return; }
    unsafe {
        let mut k: HKEY = std::ptr::null_mut();
        if RegCreateKeyExW(HKEY_CURRENT_USER, w(RUN).as_ptr(), 0, std::ptr::null(), 0, KEY_WRITE | KEY_READ, std::ptr::null(), &mut k, std::ptr::null_mut()) != 0 { return; }
        if on {
            let v = w(&command());
            RegSetValueExW(k, w(RUN_NAME).as_ptr(), 0, REG_SZ, v.as_ptr() as *const u8, (v.len() * 2) as u32);
        } else {
            RegDeleteValueW(k, w(RUN_NAME).as_ptr());
        }
        RegCloseKey(k);
    }
}
/// Follows the exe if it moved.
pub fn refresh_autostart() { if !DEV && autostart_enabled() { set_autostart(true); } }

// ---------- tray ----------
#[derive(Clone, Copy, Debug)]
pub enum TrayCmd { Show, Pause, Autostart, Restart, Exit }
pub struct TrayHooks { pub on: Box<dyn Fn(TrayCmd) + Send + Sync>, pub paused: Box<dyn Fn() -> bool + Send + Sync> }
static HOOKS: OnceLock<TrayHooks> = OnceLock::new();
static TRAY_HWND: AtomicIsize = AtomicIsize::new(0);
const WM_TRAY: u32 = WM_APP + 1;
static TASKBAR_CREATED: OnceLock<u32> = OnceLock::new();

fn nid(hwnd: HWND) -> NOTIFYICONDATAW {
    let mut n: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    n.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    n.hWnd = hwnd; n.uID = 1;
    n
}
fn add_icon(hwnd: HWND) {
    unsafe {
        let mut n = nid(hwnd);
        n.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        n.uCallbackMessage = WM_TRAY;
        let (cx, cy) = (GetSystemMetrics(SM_CXSMICON), GetSystemMetrics(SM_CYSMICON));
        let mut ico = LoadImageW(GetModuleHandleW(std::ptr::null()), 1 as _, IMAGE_ICON, cx, cy, 0);
        if ico.is_null() { log("embedded icon missing, using the stock one"); ico = LoadIconW(std::ptr::null_mut(), IDI_APPLICATION) as _; }
        n.hIcon = ico as HICON;
        let tip = w(if DEV { "T3 Presence (dev)" } else { "T3 Presence" });
        n.szTip[..tip.len()].copy_from_slice(&tip);
        if Shell_NotifyIconW(NIM_ADD, &n) == 0 { log("tray icon add failed"); }
    }
}
pub fn remove_tray() {
    let h = TRAY_HWND.swap(0, Ordering::SeqCst);
    if h != 0 { unsafe { let n = nid(h as HWND); Shell_NotifyIconW(NIM_DELETE, &n); } }
}

const ID_SHOW: usize = 1; const ID_PAUSE: usize = 2; const ID_AUTO: usize = 3; const ID_RESTART: usize = 4; const ID_EXIT: usize = 5;
fn menu(hwnd: HWND) {
    let Some(h) = HOOKS.get() else { return };
    unsafe {
        let m = CreatePopupMenu();
        let item = |id: usize, text: &str, checked: bool| { AppendMenuW(m, MF_STRING | if checked { MF_CHECKED } else { 0 }, id, w(text).as_ptr()); };
        item(ID_SHOW, "Show", false);
        item(ID_PAUSE, "Pause presence", (h.paused)());
        item(ID_AUTO, "Start with Windows", autostart_enabled());
        AppendMenuW(m, MF_SEPARATOR, 0, std::ptr::null());
        item(ID_RESTART, "Restart", false);
        item(ID_EXIT, "Exit", false);
        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        SetForegroundWindow(hwnd);
        let id = TrackPopupMenu(m, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN, pt.x, pt.y, 0, hwnd, std::ptr::null()) as usize;
        PostMessageW(hwnd, WM_NULL, 0, 0);
        DestroyMenu(m);
        let cmd = match id { ID_SHOW => TrayCmd::Show, ID_PAUSE => TrayCmd::Pause, ID_AUTO => TrayCmd::Autostart, ID_RESTART => TrayCmd::Restart, ID_EXIT => TrayCmd::Exit, _ => return };
        (h.on)(cmd);
    }
}
unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_TRAY {
        match (lp as u32) & 0xFFFF {
            WM_RBUTTONUP | WM_CONTEXTMENU => menu(hwnd),
            WM_LBUTTONDBLCLK => if let Some(h) = HOOKS.get() { (h.on)(TrayCmd::Show) },
            _ => {}
        }
        return 0;
    }
    if Some(&msg) == TASKBAR_CREATED.get() { add_icon(hwnd); return 0; } // Explorer restarted
    DefWindowProcW(hwnd, msg, wp, lp)
}
pub fn start_tray(hooks: TrayHooks) {
    let _ = HOOKS.set(hooks);
    std::thread::Builder::new().name("tray".into()).spawn(|| unsafe {
        dark_menus();
        let _ = TASKBAR_CREATED.set(RegisterWindowMessageW(w("TaskbarCreated").as_ptr()));
        let cls = w("T3PresenceTray");
        let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: GetModuleHandleW(std::ptr::null()), lpszClassName: cls.as_ptr(), ..std::mem::zeroed() };
        RegisterClassW(&wc);
        let hwnd = CreateWindowExW(0, cls.as_ptr(), cls.as_ptr(), WS_OVERLAPPED, 0, 0, 0, 0, std::ptr::null_mut(), std::ptr::null_mut(), wc.hInstance, std::ptr::null());
        TRAY_HWND.store(hwnd as isize, Ordering::SeqCst);
        add_icon(hwnd);
        let mut m: MSG = std::mem::zeroed();
        while GetMessageW(&mut m, std::ptr::null_mut(), 0, 0) > 0 { TranslateMessage(&m); DispatchMessageW(&m); }
    }).ok();
}
