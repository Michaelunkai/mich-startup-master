//! The main window: a native dark control center. Owner-drawn rows show every
//! startup route with its real schedule ("Weekly · Mon, Fri · 09:00"), a
//! one-click toggle per row, live search, a working Tools menu (add / edit /
//! repair / protect / verify), and a tray icon with new-item balloons.
//! Pure Win32 — no external GUI framework.

use msm_core::model::StartupItem;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

const WINDOW_TITLE: &str = "Mich Startup Master \u{2014} Startup Control";

// ---------------------------------------------------------------------------
// Design tokens — see DESIGN.md at the repo root. COLORREF is 0x00BBGGRR,
// so every color is built through rgb() to keep channels in the right order.
// ---------------------------------------------------------------------------
const fn rgb(red: u32, green: u32, blue: u32) -> u32 { blue << 16 | green << 8 | red }

const BG: u32 = rgb(0x16, 0x16, 0x1A); // #16161A canvas
const SURFACE: u32 = rgb(0x1D, 0x1D, 0x23); // #1D1D23 rows (odd)
const SURFACE_2: u32 = rgb(0x26, 0x26, 0x2E); // #26262E rows (even), inputs
const BORDER: u32 = rgb(0x2F, 0x2F, 0x38); // #2F2F38 hairlines
const ACCENT: u32 = rgb(0xFF, 0xB4, 0x54); // #FFB454 amber — schedule + primary
const TEXT_MAIN: u32 = rgb(0xED, 0xED, 0xF2); // #EDEDF2
const TEXT_MUTED: u32 = rgb(0x8B, 0x8B, 0x98); // #8B8B98
const GOOD: u32 = rgb(0x3F, 0xB9, 0x50); // #3FB950 enabled
const DANGER: u32 = rgb(0xF8, 0x51, 0x49); // #F85149 disabled
const SELECTION: u32 = rgb(0x33, 0x2D, 0x24); // amber-tinted selected row

fn blend(base: u32, tint: u32, amount: f32) -> u32 {
    let channel = |b: u32, t: u32| -> u32 {
        let value = b as f32 + (t as f32 - b as f32) * amount;
        value.round().clamp(0.0, 255.0) as u32
    };
    let (br, bg_, bb) = (base & 0xFF, (base >> 8) & 0xFF, (base >> 16) & 0xFF);
    let (tr, tg, tb) = (tint & 0xFF, (tint >> 8) & 0xFF, (tint >> 16) & 0xFF);
    rgb(channel(br, tr), channel(bg_, tg), channel(bb, tb))
}

const ID_TAB_APPS: i32 = 2001;
const ID_TAB_ENABLED: i32 = 2002;
const ID_TAB_DISABLED: i32 = 2003;
const ID_SEARCH: i32 = 2004;
const ID_LIST: i32 = 2005;
const ID_ACTION_BUTTON: i32 = 2006;
const ID_TOOLS_BUTTON: i32 = 2008;
const WM_REFRESHED: u32 = 0x8000 + 0x42;
const WM_APP_TRAY: u32 = 0x8000 + 0x100; // WM_APP + 0x100

const MENU_ADD: i32 = 3001;
const MENU_EDIT: i32 = 3002;
const MENU_DISABLE: i32 = 3003;
const MENU_ENABLE: i32 = 3004;
const MENU_QUIET: i32 = 3005;
const MENU_WINDOW: i32 = 3006;
const MENU_RUN_NOW: i32 = 3007;
const MENU_OPEN_LOCATION: i32 = 3008;
const MENU_COPY_COMMAND: i32 = 3009;
const MENU_COPY_SELECTED: i32 = 3010;
const MENU_COPY_VISIBLE: i32 = 3011;
const MENU_COPY_ALL: i32 = 3012;
const MENU_SELECT_ALL: i32 = 3013;
const MENU_PROTECT: i32 = 3014;
const MENU_REPAIR: i32 = 3015;
const MENU_VERIFY: i32 = 3016;
const MENU_OPEN_FOLDERS: i32 = 3017;
const MENU_DELETE_MANAGED: i32 = 3018;
const MENU_TRAY_SHOW: i32 = 3101;
const MENU_TRAY_REFRESH: i32 = 3102;
const MENU_TRAY_QUIT: i32 = 3103;
const DLG_ID_OK: i32 = 3201;
const DLG_ID_CANCEL: i32 = 3202;
const DLG_ID_BROWSE: i32 = 3203;
const DLG_ID_MODE: i32 = 3204;
const ID_FOCUS_SEARCH: i32 = 3020;

static MAIN_HWND: AtomicIsize = AtomicIsize::new(0);
static CURRENT_DPI: AtomicU32 = AtomicU32::new(96);

pub struct AppSnapshot {
    pub items: Vec<StartupItem>,
    pub filter: String,
    pub query: String,
    pub rows: Vec<msm_ui::view::InventoryRow>,
    pub selected: usize,
}

pub struct AppState {
    pub snapshot: Mutex<AppSnapshot>,
    pub scanning: AtomicBool,
    pub last_status: Mutex<String>,
    pub open_requested: AtomicBool,
}

thread_local! {
    static STATE: std::cell::RefCell<Option<Arc<AppState>>> = const { std::cell::RefCell::new(None) };
}

fn wstr(text: &str) -> Vec<u16> { msm_core::paths::to_wide(text) }


// ---------------------------------------------------------------------------
// Fonts (scaled per DPI; recreated on WM_DPICHANGED)
// ---------------------------------------------------------------------------
#[derive(Clone, Copy)]
struct Fonts {
    body: windows::Win32::Graphics::Gdi::HFONT,
    semibold: windows::Win32::Graphics::Gdi::HFONT,
    mono: windows::Win32::Graphics::Gdi::HFONT,
    small: windows::Win32::Graphics::Gdi::HFONT,
}

thread_local! {
    // GDI handles are not Send; the cache stays on the UI thread that paints.
    static FONT_CACHE: std::cell::RefCell<Option<(u32, Fonts)>> = const { std::cell::RefCell::new(None) };
}

fn create_font_dpi(height_units: i32, weight: u32, face: &str, dpi: u32) -> windows::Win32::Graphics::Gdi::HFONT {
    use windows::Win32::Graphics::Gdi::HFONT;
    unsafe {
        #[allow(non_snake_case)]
        let CreateFontW: unsafe extern "system" fn(i32, i32, i32, i32, u32, u32, u32, u32, u32, u32, u32, u32, u32, windows::core::PCWSTR) -> HFONT = {
            extern "system" {
                #[link_name = "CreateFontW"]
                fn imp(height: i32, width: i32, escapement: i32, orientation: i32, weight: u32, italic: u32, underline: u32, strikeout: u32, charset: u32, output: u32, clipping: u32, quality: u32, pitch: u32, face: windows::core::PCWSTR) -> HFONT;
            }
            imp
        };
        let pixels = -(height_units * dpi as i32 / 96);
        let wide = msm_core::paths::to_wide(face);
        CreateFontW(pixels, 0, 0, 0, weight, 0, 0, 0, 1, 0, 0, 5, 0, windows::core::PCWSTR(wide.as_ptr()))
    }
}

fn font_cache_take() {
    FONT_CACHE.with(|cache| *cache.borrow_mut() = None);
}

fn fonts_for(dpi: u32) -> Fonts {
    if let Some((cached_dpi, fonts)) = FONT_CACHE.with(|cache| *cache.borrow()) {
        if cached_dpi == dpi { return fonts; }
    }
    // Fonts from a previous DPI are replaced; a few leaked GDI handles per DPI
    // change are bounded by the number of monitors.
    let fonts = Fonts {
        body: create_font_dpi(15, 400, "Segoe UI", dpi),
        semibold: create_font_dpi(15, 600, "Segoe UI", dpi),
        mono: create_font_dpi(13, 400, "Cascadia Mono", dpi),
        small: create_font_dpi(12, 600, "Segoe UI", dpi),
    };
    FONT_CACHE.with(|cache| *cache.borrow_mut() = Some((dpi, fonts)));
    fonts
}

// ---------------------------------------------------------------------------
// Entry
// ---------------------------------------------------------------------------
pub fn run(start_in_tray: bool) -> i32 {
    let state = Arc::new(AppState {
        snapshot: Mutex::new(AppSnapshot {
            items: Vec::new(),
            filter: "Apps".into(),
            query: String::new(),
            rows: Vec::new(),
            selected: 0,
        }),
        scanning: AtomicBool::new(false),
        last_status: Mutex::new("Scanning every startup surface\u{2026}".into()),
        open_requested: AtomicBool::new(!start_in_tray),
    });
    STATE.with(|s| *s.borrow_mut() = Some(state.clone()));
    // First inventory scan off the UI thread so the window appears instantly.
    run_inventory_refresh(state.clone());
    match run_message_loop(state) {
        Ok(code) => code,
        Err(e) => { eprintln!("window error: {}", e); 1 }
    }
}

fn pending_balloon() -> &'static Mutex<Option<(String, String)>> {
    static BALLOON: OnceLock<Mutex<Option<(String, String)>>> = OnceLock::new();
    BALLOON.get_or_init(|| Mutex::new(None))
}

/// One full refresh pass: scan, record first-seen, detect new arrivals, update
/// the visible snapshot, then post WM_REFRESHED so the UI re-renders. Shared
/// by launch, tools actions, tray refresh, and the real-time change monitor.
fn run_inventory_refresh(state: Arc<AppState>) {
    if state.scanning.swap(true, Ordering::SeqCst) {
        return; // a refresh is already in flight
    }
    std::thread::Builder::new().name("inventory-refresh".into()).spawn(move || {
        let items = msm_core::scan::scan_all();
        msm_state::first_seen::observe(&items);
        let fresh = msm_state::known::detect_new_with(&items);
        {
            let mut snapshot = state.snapshot.lock().unwrap();
            snapshot.items = items;
            snapshot.rows = msm_ui::view::build_visible_rows(&snapshot.items, &snapshot.filter, &snapshot.query);
            snapshot.selected = snapshot.selected.min(snapshot.rows.len().saturating_sub(1));
            let status = if fresh.is_empty() {
                format!("{} startup routes \u{00B7} real-time monitoring active", snapshot.items.len())
            } else {
                format!("New startup item detected: {}", fresh[0].human_name())
            };
            *state.last_status.lock().unwrap() = status;
        }
        state.scanning.store(false, Ordering::SeqCst);
        if !fresh.is_empty() {
            // Balloons are shown by the UI thread when WM_REFRESHED lands.
            *pending_balloon().lock().unwrap() = Some((
                "New startup item".into(),
                format!("{} just registered to run.", fresh[0].human_name()),
            ));
        }
        post_refresh_to_windows();
        start_change_monitor(state.clone());
    }).ok();
}

fn start_change_monitor(state: Arc<AppState>) {
    use std::sync::Once;
    static STARTED: Once = Once::new();
    STARTED.call_once(|| {
        let monitor_state = state.clone();
        std::thread::Builder::new().name("change-monitor".into()).spawn(move || {
            let monitor = Arc::new(msm_watch::StartupChangeMonitor::new(move || {
                // Registry / folder / task-catalog change → full live re-scan.
                run_inventory_refresh(monitor_state.clone());
            }));
            monitor.start();
            loop {
                std::thread::sleep(std::time::Duration::from_secs(3600));
            }
        }).ok();
    });
}

fn post_refresh_to_windows() {
    unsafe {
        #[allow(non_snake_case)]
        let PostMessageW: unsafe extern "system" fn(windows::Win32::Foundation::HWND, u32, usize, isize) -> windows::core::BOOL = {
            extern "system" {
                #[link_name = "PostMessageW"]
                fn imp(hwnd: windows::Win32::Foundation::HWND, message: u32, wparam: usize, lparam: isize) -> windows::core::BOOL;
            }
            imp
        };
        let hwnd = windows::Win32::Foundation::HWND(MAIN_HWND.load(Ordering::SeqCst) as *mut _);
        let _ = PostMessageW(hwnd, WM_REFRESHED, 0, 0);
    }
}

fn set_status(text: &str) {
    STATE.with(|s| {
        if let Some(state) = s.borrow().as_ref() {
            *state.last_status.lock().unwrap() = text.to_string();
        }
    });
    if std::env::var_os("MSM_UI_LOG").is_some() {
        use std::io::Write;
        let path = std::env::temp_dir().join("msm-status.log");
        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
            let _ = writeln!(file, "{}", text);
        }
    }
    invalidate_main();
}

fn invalidate_main() {
    unsafe {
        let hwnd = windows::Win32::Foundation::HWND(MAIN_HWND.load(Ordering::SeqCst) as *mut _);
        let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, true);
    }
}

// ---------------------------------------------------------------------------
// Window + message loop
// ---------------------------------------------------------------------------
unsafe extern "system" fn window_proc(hwnd: windows::Win32::Foundation::HWND, message: u32, wparam: windows::Win32::Foundation::WPARAM, lparam: windows::Win32::Foundation::LPARAM) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::Foundation::LRESULT;
    use windows::Win32::UI::WindowsAndMessaging::*;
    match message {
        WM_CREATE => {
            MAIN_HWND.store(hwnd.0 as isize, Ordering::SeqCst);
            CURRENT_DPI.store(windows::Win32::UI::HiDpi::GetDpiForWindow(hwnd), Ordering::SeqCst);
            enable_dark_title_bar(hwnd);
            set_window_icons(hwnd);
            LRESULT(1)
        }
        WM_SIZE => { layout_controls(hwnd); LRESULT(0) }
        WM_PAINT => { paint_window(hwnd); LRESULT(0) }
        WM_ERASEBKGND => LRESULT(1),
        WM_COMMAND => {
            let id = (wparam.0 & 0xFFFF) as i32;
            let notification = ((wparam.0 >> 16) & 0xFFFF) as u32;
            let handled = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if id == ID_SEARCH && notification == EN_CHANGE {
                    apply_search();
                } else {
                    handle_command(hwnd, id);
                }
            }));
            if handled.is_err() {
                set_status("Internal error while running that action - everything was left unchanged.");
            }
            LRESULT(0)
        }
        WM_NOTIFY => {
            let notification = &*(lparam.0 as *const windows::Win32::UI::Controls::NMHDR);
            if notification.idFrom == ID_LIST as usize {
                handle_list_notify(notification.code);
            }
            LRESULT(0)
        }
        WM_DRAWITEM => { on_draw_item(wparam, lparam); LRESULT(1) }
        WM_MEASUREITEM => { on_measure_item(lparam); LRESULT(1) }
        WM_CTLCOLORLISTBOX => { dark_control_color(wparam, SURFACE); LRESULT(dark_brush(SURFACE).0 as isize) }
        WM_CTLCOLOREDIT | WM_CTLCOLORSTATIC => { dark_control_color(wparam, SURFACE_2); LRESULT(dark_brush(SURFACE_2).0 as isize) }
        WM_GETMINMAXINFO => {
            let info = &mut *(lparam.0 as *mut MINMAXINFO);
            let scale = dpi_scale();
            info.ptMinTrackSize = windows::Win32::Foundation::POINT { x: (900.0 * scale) as i32, y: (660.0 * scale) as i32 };
            LRESULT(0)
        }
        WM_DPICHANGED => {
            let dpi = ((wparam.0 >> 16) & 0xFFFF) as u32;
            CURRENT_DPI.store(dpi, Ordering::SeqCst);
            font_cache_take();
            let suggested = &*(lparam.0 as *const windows::Win32::Foundation::RECT);
            let _ = SetWindowPos(hwnd, None, suggested.left, suggested.top,
                suggested.right - suggested.left, suggested.bottom - suggested.top,
                SWP_NOZORDER | SWP_NOACTIVATE);
            relayout_and_repaint_fonts();
            LRESULT(0)
        }
        WM_APP_TRAY => on_tray_message(hwnd, lparam.0 as u32),
        WM_REFRESHED => {
            if let Some((title, body)) = pending_balloon().lock().unwrap().take() {
                tray_balloon(&title, &body);
            }
            refresh_list();
            LRESULT(0)
        }
        WM_CLOSE => {
            // Closing keeps the resident agent + live monitor in the tray
            // (contract: hiddenTrayInventoryStaysCurrent). Quit is on the tray menu.
            let _ = ShowWindow(hwnd, SW_HIDE);
            LRESULT(0)
        }
        WM_DESTROY => {
            remove_tray_icon();
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

fn dpi_scale() -> f32 { CURRENT_DPI.load(Ordering::SeqCst) as f32 / 96.0 }

fn run_message_loop(state: Arc<AppState>) -> Result<i32, String> {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::*;

    unsafe {
        let instance = GetModuleHandleW(None).map_err(|e| e.to_string())?;
        CURRENT_DPI.store(96, Ordering::SeqCst);

        let class_name = wstr("MichStartupMasterWindow");
        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            lpszClassName: windows::core::PCWSTR(class_name.as_ptr()),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hbrBackground: windows::Win32::Graphics::Gdi::HBRUSH::default(),
            ..Default::default()
        };
        let atom = RegisterClassW(&wc);
        if atom == 0 {
            return Err("RegisterClassW failed".into());
        }
        let (width, height) = (1360, 860);
        let mut rect = RECT { left: 0, top: 0, right: width, bottom: height };
        let _ = AdjustWindowRect(&mut rect, WS_OVERLAPPEDWINDOW, false);
        let window_width = rect.right - rect.left;
        let window_height = rect.bottom - rect.top;
        let screen = windows::Win32::UI::WindowsAndMessaging::GetSystemMetrics(windows::Win32::UI::WindowsAndMessaging::SM_CXSCREEN);
        let screen_h = windows::Win32::UI::WindowsAndMessaging::GetSystemMetrics(windows::Win32::UI::WindowsAndMessaging::SM_CYSCREEN);
        let x = (screen - window_width).max(0) / 2;
        let y = (screen_h - window_height).max(0) / 3;
        let title_buffer = wstr(WINDOW_TITLE);
        let hwnd = CreateWindowExW(
            Default::default(),
            windows::core::PCWSTR(class_name.as_ptr()),
            windows::core::PCWSTR(title_buffer.as_ptr()),
            WS_OVERLAPPEDWINDOW,
            x, y, window_width, window_height,
            None, None, Some(instance.into()), None)
            .map_err(|_| "CreateWindowExW failed")?;
        build_controls(hwnd, instance.into());
        let show_now = state.open_requested.load(Ordering::SeqCst);
        if show_now {
            let _ = ShowWindow(hwnd, SW_SHOW);
        } else {
            *state.last_status.lock().unwrap() = "Running quietly in the system tray.".into();
        }
        let _ = windows::Win32::Graphics::Gdi::UpdateWindow(hwnd);
        create_tray_icon(hwnd);
        let mut message = MSG::default();
        loop {
            let result = GetMessageW(&mut message, None, 0, 0);
            if result.0 <= 0 {
                if std::env::var_os("MSM_UI_LOG").is_some() {
                    use std::io::Write;
                    let path = std::env::temp_dir().join("msm-status.log");
                    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
                        let _ = writeln!(file, "message loop ended: ret={} msg=0x{:X} wParam={}", result.0, message.message, message.wParam.0);
                    }
                }
                break;
            }
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        Ok(message.wParam.0 as i32)
    }
}

fn enable_dark_title_bar(hwnd: windows::Win32::Foundation::HWND) {
    use windows::core::BOOL;
    const DWMWA_USE_IMMERSIVE_DARK_MODE: u32 = 20;
    #[allow(non_snake_case)]
    unsafe {
        let DwmSetWindowAttribute: unsafe extern "system" fn(windows::Win32::Foundation::HWND, u32, *const core::ffi::c_void, u32) -> windows::core::HRESULT = {
            extern "system" {
                #[link_name = "DwmSetWindowAttribute"]
                fn imp(hwnd: windows::Win32::Foundation::HWND, attribute: u32, value: *const core::ffi::c_void, length: u32) -> windows::core::HRESULT;
            }
            imp
        };
        let dark: BOOL = BOOL(1);
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark as *const _ as *const _, std::mem::size_of::<BOOL>() as u32);
    }
}

fn set_window_icons(hwnd: windows::Win32::Foundation::HWND) {
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::*;
    unsafe {
        let instance = windows::Win32::System::LibraryLoader::GetModuleHandleW(None).unwrap_or_default();
        let small = LoadImageW(Some(instance.into()), windows::core::PCWSTR(1usize as *const u16), IMAGE_ICON, 16, 16, LR_DEFAULTCOLOR).unwrap_or_default();
        let big = LoadImageW(Some(instance.into()), windows::core::PCWSTR(1usize as *const u16), IMAGE_ICON, 32, 32, LR_DEFAULTCOLOR).unwrap_or_default();
        if !small.is_invalid() { let _ = SendMessageW(hwnd, WM_SETICON as u32, Some(WPARAM(0)), Some(LPARAM(small.0 as isize))); }
        if !big.is_invalid() { let _ = SendMessageW(hwnd, WM_SETICON as u32, Some(WPARAM(1)), Some(LPARAM(big.0 as isize))); }
    }
}

fn dark_brush(color: u32) -> windows::Win32::Graphics::Gdi::HBRUSH {
    thread_local! {
        static BRUSHES: std::cell::RefCell<Vec<(u32, windows::Win32::Graphics::Gdi::HBRUSH)>> = const { std::cell::RefCell::new(Vec::new()) };
    }
    BRUSHES.with(|brushes| {
        let mut brushes = brushes.borrow_mut();
        if let Some((_, brush)) = brushes.iter().find(|(c, _)| *c == color) {
            return *brush;
        }
        let brush = unsafe { windows::Win32::Graphics::Gdi::CreateSolidBrush(windows::Win32::Foundation::COLORREF(color)) };
        brushes.push((color, brush));
        brush
    })
}

unsafe fn dark_control_color(wparam: windows::Win32::Foundation::WPARAM, background: u32) {
    use windows::Win32::Foundation::COLORREF;
    use windows::Win32::Graphics::Gdi::{SetBkColor, SetTextColor};
    let hdc = windows::Win32::Graphics::Gdi::HDC(wparam.0 as *mut _);
    let _ = SetTextColor(hdc, COLORREF(TEXT_MAIN));
    let _ = SetBkColor(hdc, COLORREF(background));
}

// ---------------------------------------------------------------------------
// Controls
// ---------------------------------------------------------------------------
fn build_controls(hwnd: windows::Win32::Foundation::HWND, instance: windows::Win32::Foundation::HINSTANCE) {
    unsafe {
        create_owner_button(hwnd, instance, "Apps", ID_TAB_APPS);
        create_owner_button(hwnd, instance, "Enabled", ID_TAB_ENABLED);
        create_owner_button(hwnd, instance, "Disabled", ID_TAB_DISABLED);
        create_owner_button(hwnd, instance, "Tools", ID_TOOLS_BUTTON);
        create_owner_button(hwnd, instance, "Disable", ID_ACTION_BUTTON);
        create_search_box(hwnd, instance);
        create_list_view(hwnd, instance);
    }
}

unsafe fn create_owner_button(parent: windows::Win32::Foundation::HWND, instance: windows::Win32::Foundation::HINSTANCE, label: &str, id: i32) {
    use windows::Win32::UI::WindowsAndMessaging::*;
    let text = wstr(label);
    let class = wstr("BUTTON");
    let _ = CreateWindowExW(
        Default::default(),
        windows::core::PCWSTR(class.as_ptr()),
        windows::core::PCWSTR(text.as_ptr()),
        WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | BS_OWNERDRAW as u32 | WS_TABSTOP.0),
        0, 0, 10, 10,
        Some(parent), Some(HMENU(id as *mut _)), Some(instance), None)
        .ok();
}

unsafe fn create_search_box(parent: windows::Win32::Foundation::HWND, instance: windows::Win32::Foundation::HINSTANCE) {
    use windows::Win32::UI::WindowsAndMessaging::*;
    let class = wstr("EDIT");
    let empty = wstr("");
    if let Some(hwnd) = CreateWindowExW(
        Default::default(),
        windows::core::PCWSTR(class.as_ptr()),
        windows::core::PCWSTR(empty.as_ptr()),
        WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | ES_AUTOHSCROLL as u32 | WS_TABSTOP.0),
        0, 0, 10, 10,
        Some(parent), Some(HMENU(ID_SEARCH as *mut _)), Some(instance), None)
        .ok()
    {
        apply_control_font(hwnd);
        let _ = windows::Win32::UI::Controls::SetWindowTheme(hwnd, windows::core::PCWSTR(wstr("DarkMode_Explorer").as_ptr()), None);
        let cue = wstr("Search startup routes");
        let _ = SendMessageW(hwnd, 0x1501 /* EM_SETCUEBANNER */, Some(windows::Win32::Foundation::WPARAM(1)), Some(windows::Win32::Foundation::LPARAM(cue.as_ptr() as isize)));
    }
}

unsafe fn create_list_view(parent: windows::Win32::Foundation::HWND, instance: windows::Win32::Foundation::HINSTANCE) {
    use windows::Win32::UI::WindowsAndMessaging::*;
    let class = wstr("LISTBOX");
    let empty = wstr("");
    if let Some(hwnd) = CreateWindowExW(
        WS_EX_CLIENTEDGE,
        windows::core::PCWSTR(class.as_ptr()),
        windows::core::PCWSTR(empty.as_ptr()),
        WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | LBS_NOTIFY as u32 | WS_VSCROLL.0 | LBS_NOINTEGRALHEIGHT as u32 | LBS_OWNERDRAWFIXED as u32),
        0, 0, 10, 10,
        Some(parent), Some(HMENU(ID_LIST as *mut _)), Some(instance), None)
        .ok()
    {
        apply_control_font(hwnd);
        let _ = windows::Win32::UI::Controls::SetWindowTheme(hwnd, windows::core::PCWSTR(wstr("DarkMode_Explorer").as_ptr()), None);
        subclass_list(hwnd);
    }
}

unsafe fn apply_control_font(hwnd: windows::Win32::Foundation::HWND) {
    let font = fonts_for(CURRENT_DPI.load(Ordering::SeqCst)).body;
    let _ = windows::Win32::UI::WindowsAndMessaging::SendMessageW(
        hwnd,
        windows::Win32::UI::WindowsAndMessaging::WM_SETFONT as u32,
        Some(windows::Win32::Foundation::WPARAM(font.0 as usize)),
        Some(windows::Win32::Foundation::LPARAM(1)));
}

struct Layout {
    s: f32,
    width: i32,
    margin: i32,
    toolbar_h: i32,
    row_h: i32,
    header_h: i32,
    status_h: i32,
    tab_w: i32,
    tools_w: i32,
    search_w: i32,
    action_w: i32,
    schedule_x: i32,
    source_x: i32,
    toggle_w: i32,
    toggle_h: i32,
    toggle_x: i32,
}

/// Geometry from the CLIENT width. Row hit-testing inside the listbox uses the
/// same client width so drawn switches and clickable zones always agree.
fn layout_metrics(width: i32) -> Layout {
    let s = dpi_scale();
    let margin = (20.0 * s) as i32;
    Layout {
        s,
        width,
        margin,
        toolbar_h: (36.0 * s) as i32,
        row_h: (56.0 * s) as i32,
        header_h: (34.0 * s) as i32,
        status_h: (44.0 * s) as i32,
        tab_w: (108.0 * s) as i32,
        tools_w: (108.0 * s) as i32,
        search_w: (320.0 * s) as i32,
        action_w: (168.0 * s) as i32,
        schedule_x: ((width as f32) * 0.47).clamp(400.0 * s, 620.0 * s) as i32,
        source_x: 0, // filled below
        toggle_w: (40.0 * s) as i32,
        toggle_h: (22.0 * s) as i32,
        toggle_x: 0, // filled below
    }
    .with_columns()
}

impl Layout {
    fn with_columns(mut self) -> Layout {
        self.source_x = self.schedule_x + (216.0 * self.s) as i32;
        self.toggle_x = self.width - self.margin - self.toggle_w - (16.0 * self.s) as i32;
        self
    }
}

/// Client width corresponding to a listbox row (rows span margin..width-margin).
fn client_width_from_list(list_width: i32) -> i32 {
    list_width + 2 * ((20.0 * dpi_scale()) as i32)
}

fn layout_controls(hwnd: windows::Win32::Foundation::HWND) {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::WindowsAndMessaging::*;
    unsafe {
        let mut client = RECT::default();
        let _ = GetClientRect(hwnd, &mut client);
        let width = client.right - client.left;
        let height = client.bottom - client.top;
        let layout = layout_metrics(width);
        let top = layout.margin;
        let mut x = layout.margin;
        for id in [ID_TAB_APPS, ID_TAB_ENABLED, ID_TAB_DISABLED] {
            if let Ok(handle) = GetDlgItem(Some(hwnd), id) {
                let _ = MoveWindow(handle, x, top, layout.tab_w, layout.toolbar_h, true);
            }
            x += layout.tab_w + (8.0 * layout.s) as i32;
        }
        if let Ok(tools) = GetDlgItem(Some(hwnd), ID_TOOLS_BUTTON) {
            let _ = MoveWindow(tools, x + (4.0 * layout.s) as i32, top, layout.tools_w, layout.toolbar_h, true);
            x += (4.0 * layout.s) as i32 + layout.tools_w;
        }
        let action_x = width - layout.margin - layout.action_w;
        if let Ok(action) = GetDlgItem(Some(hwnd), ID_ACTION_BUTTON) {
            let _ = MoveWindow(action, action_x, top, layout.action_w, layout.toolbar_h, true);
        }
        let search_x = x + (14.0 * layout.s) as i32;
        let search_w = (action_x - (12.0 * layout.s) as i32 - search_x).max(120);
        if let Ok(search) = GetDlgItem(Some(hwnd), ID_SEARCH) {
            let _ = MoveWindow(search, search_x, top + 2, search_w, layout.toolbar_h - 4, true);
        }
        let list_top = layout.margin + layout.toolbar_h + (14.0 * layout.s) as i32 + layout.header_h;
        let list_h = (height - list_top - layout.status_h).max((100.0 * layout.s) as i32);
        if let Ok(list) = GetDlgItem(Some(hwnd), ID_LIST) {
            let _ = MoveWindow(list, layout.margin, list_top, width - layout.margin * 2, list_h, true);
        }
    }
}

fn relayout_and_repaint_fonts() {
    unsafe {
        let hwnd = windows::Win32::Foundation::HWND(MAIN_HWND.load(Ordering::SeqCst) as *mut _);
        if hwnd.0.is_null() { return; }
        if let Ok(search) = windows::Win32::UI::WindowsAndMessaging::GetDlgItem(Some(hwnd), ID_SEARCH) {
            apply_control_font(search);
        }
        if let Ok(list) = windows::Win32::UI::WindowsAndMessaging::GetDlgItem(Some(hwnd), ID_LIST) {
            apply_control_font(list);
            // Force WM_MEASUREITEM again with the new DPI row height.
            let _ = windows::Win32::UI::WindowsAndMessaging::SetWindowPos(list, None, 0, 0, 0, 0,
                windows::Win32::UI::WindowsAndMessaging::SWP_NOMOVE
                | windows::Win32::UI::WindowsAndMessaging::SWP_NOSIZE
                | windows::Win32::UI::WindowsAndMessaging::SWP_NOZORDER
                | windows::Win32::UI::WindowsAndMessaging::SWP_FRAMECHANGED);
        }
        let _ = windows::Win32::UI::WindowsAndMessaging::SendMessageW(
            hwnd, windows::Win32::UI::WindowsAndMessaging::WM_SIZE, None, None);
    }
    refresh_list();
}

// ---------------------------------------------------------------------------
// Painting
// ---------------------------------------------------------------------------
fn paint_window(hwnd: windows::Win32::Foundation::HWND) {
    use windows::Win32::Foundation::{COLORREF, RECT};
    use windows::Win32::Graphics::Gdi::{BeginPaint, CreatePen, DeleteObject, EndPaint, FillRect, LineTo, MoveToEx, PAINTSTRUCT, PS_SOLID, SelectObject, SetBkMode, TRANSPARENT};
    use windows::Win32::UI::WindowsAndMessaging::*;
    unsafe {
        let mut paint = PAINTSTRUCT::default();
        let context = BeginPaint(hwnd, &mut paint);
        if context.is_invalid() { return; }
        let mut client = RECT::default();
        let _ = GetClientRect(hwnd, &mut client);
        let width = client.right - client.left;
        let height = client.bottom - client.top;
        let layout = layout_metrics(width);
        let fonts = fonts_for(CURRENT_DPI.load(Ordering::SeqCst));

        let brush = windows::Win32::Graphics::Gdi::CreateSolidBrush(COLORREF(BG));
        FillRect(context, &client, brush);
        let _ = DeleteObject(brush.into());

        let list_top = layout.margin + layout.toolbar_h + (14.0 * layout.s) as i32 + layout.header_h;
        let list_bottom = height - layout.status_h;

        // Column header band.
        let old_font = SelectObject(context, fonts.small.into());
        let _ = SetBkMode(context, TRANSPARENT);
        let _ = windows::Win32::Graphics::Gdi::SetTextColor(context, COLORREF(TEXT_MUTED));
        let header_y = list_top - layout.header_h + ((layout.header_h as f32 - 16.0 * layout.s) as i32) / 2;
        draw_text_left(context, "Startup item", (44.0 * layout.s) as i32, header_y);
        draw_text_left(context, "Schedule", layout.schedule_x, header_y);
        draw_text_left(context, "Source", layout.source_x, header_y);
        let impact = wstr("Impact");
        let _ = windows::Win32::Graphics::Gdi::TextOutW(context, layout.toggle_x - (76.0 * layout.s) as i32, header_y, &impact[..impact.len() - 1]);

        // Hairline under the header.
        let pen = CreatePen(PS_SOLID, 1, COLORREF(BORDER));
        let old_pen = SelectObject(context, pen.into());
        let _ = MoveToEx(context, layout.margin, list_top - 1, None);
        let _ = LineTo(context, width - layout.margin, list_top - 1);

        // Status bar hairline + content.
        let status_top = height - layout.status_h;
        let _ = MoveToEx(context, layout.margin, status_top, None);
        let _ = LineTo(context, width - layout.margin, status_top);
        let _ = SelectObject(context, old_pen);
        let _ = DeleteObject(pen.into());

        let (status_text, counts) = STATE.with(|s| {
            s.borrow().as_ref().map(|state| {
                let snapshot = state.snapshot.lock().unwrap();
                let scanning = state.scanning.load(Ordering::SeqCst);
                let enabled = snapshot.items.iter().filter(|x| x.enabled).count();
                let counts = if scanning {
                    format!("{} routes \u{00B7} scanning\u{2026}", snapshot.items.len())
                } else {
                    format!("{} routes \u{00B7} {} enabled \u{00B7} {} disabled", snapshot.items.len(), enabled, snapshot.items.len().saturating_sub(enabled))
                };
                (state.last_status.lock().unwrap().clone(), counts)
            }).unwrap_or_default()
        });

        let _ = SelectObject(context, fonts.body.into());
        let _ = windows::Win32::Graphics::Gdi::SetTextColor(context, COLORREF(TEXT_MUTED));
        let status_rect = RECT { left: layout.margin, top: status_top, right: width - layout.margin, bottom: height };
        draw_text_clipped(context, &status_text, &status_rect, (12.0 * layout.s) as i32);
        if !counts.is_empty() {
            let counts_w = text_width(context, &counts, fonts.body);
            draw_text_left_at(context, &counts, width - layout.margin - counts_w, status_top + (12.0 * layout.s) as i32);
        }

        // Empty / loading state over the list area.
        let row_count = STATE.with(|s| s.borrow().as_ref().map(|state| state.snapshot.lock().unwrap().rows.len()).unwrap_or(0));
        if row_count == 0 {
            let scanning = STATE.with(|s| s.borrow().as_ref().map(|state| state.scanning.load(Ordering::SeqCst)).unwrap_or(false));
            let _ = SelectObject(context, fonts.body.into());
            let _ = windows::Win32::Graphics::Gdi::SetTextColor(context, COLORREF(TEXT_MUTED));
            let message = if scanning { "Scanning every startup surface\u{2026}" } else { "No startup routes match this view." };
            let size = text_size(context, message, fonts.body);
            let x = (width - size.cx) / 2;
            let y = list_top + ((list_bottom - list_top) / 3);
            let mut wide = wstr(message);
            wide.pop();
            let _ = windows::Win32::Graphics::Gdi::TextOutW(context, x, y, &wide);
        }

        let _ = SelectObject(context, old_font);
        let _ = EndPaint(hwnd, &paint);
    }
}

unsafe fn draw_text_left(context: windows::Win32::Graphics::Gdi::HDC, text: &str, x: i32, y: i32) {
    let mut wide = wstr(text);
    wide.pop();
    let _ = windows::Win32::Graphics::Gdi::TextOutW(context, x, y, &wide);
}

unsafe fn draw_text_left_at(context: windows::Win32::Graphics::Gdi::HDC, text: &str, x: i32, y: i32) {
    draw_text_left(context, text, x, y);
}

unsafe fn draw_text_clipped(context: windows::Win32::Graphics::Gdi::HDC, text: &str, bounds: &windows::Win32::Foundation::RECT, dy: i32) {
    use windows::Win32::Graphics::Gdi::{DrawTextExW, DT_END_ELLIPSIS, DT_NOPREFIX, DT_SINGLELINE, DRAW_TEXT_FORMAT};
    let mut wide = wstr(text);
    let mut rect = bounds.clone();
    rect.top += dy;
    let _ = DrawTextExW(context, wide.as_mut_slice(), &mut rect, DRAW_TEXT_FORMAT(DT_SINGLELINE.0 | DT_END_ELLIPSIS.0 | DT_NOPREFIX.0), None);
}

unsafe fn text_width(context: windows::Win32::Graphics::Gdi::HDC, text: &str, font: windows::Win32::Graphics::Gdi::HFONT) -> i32 {
    text_size(context, text, font).cx
}

unsafe fn text_size(context: windows::Win32::Graphics::Gdi::HDC, text: &str, _font: windows::Win32::Graphics::Gdi::HFONT) -> windows::Win32::Foundation::SIZE {
    let mut size = windows::Win32::Foundation::SIZE::default();
    #[allow(non_snake_case)]
    let GetTextExtentPoint32W: unsafe extern "system" fn(windows::Win32::Graphics::Gdi::HDC, windows::core::PCWSTR, i32, *mut windows::Win32::Foundation::SIZE) -> windows::core::BOOL = {
        extern "system" {
            #[link_name = "GetTextExtentPoint32W"]
            fn imp(hdc: windows::Win32::Graphics::Gdi::HDC, text: windows::core::PCWSTR, length: i32, size: *mut windows::Win32::Foundation::SIZE) -> windows::core::BOOL;
        }
        imp
    };
    let wide = wstr(text);
    let _ = GetTextExtentPoint32W(context, windows::core::PCWSTR(wide.as_ptr()), wide.len() as i32 - 1, &mut size);
    size
}

unsafe fn on_measure_item(lparam: windows::Win32::Foundation::LPARAM) {
    use windows::Win32::UI::Controls::MEASUREITEMSTRUCT;
    let item = &mut *(lparam.0 as *mut MEASUREITEMSTRUCT);
    if item.CtlID == ID_LIST as u32 {
        item.itemHeight = (56.0 * dpi_scale()) as u32;
    }
}

unsafe fn on_draw_item(wparam: windows::Win32::Foundation::WPARAM, lparam: windows::Win32::Foundation::LPARAM) {
    use windows::Win32::UI::Controls::DRAWITEMSTRUCT;
    let item = &*(lparam.0 as *const DRAWITEMSTRUCT);
    let id = wparam.0 as u32;
    if id == ID_LIST as u32 {
        draw_list_row(item);
    } else if id != 0 {
        draw_button(item, id as i32);
    }
}

unsafe fn draw_button(item: &windows::Win32::UI::Controls::DRAWITEMSTRUCT, id: i32) {
    use windows::Win32::Foundation::{COLORREF, RECT};
    use windows::Win32::Graphics::Gdi::{CreatePen, DeleteObject, FillRect, RoundRect, SelectObject, SetBkMode, SetTextColor, TRANSPARENT, PS_SOLID};
    let context = item.hDC;
    let rect = item.rcItem;
    let fonts = fonts_for(CURRENT_DPI.load(Ordering::SeqCst));
    let label: String = if id == ID_ACTION_BUTTON { action_label() } else { control_text(item.hwndItem) };
    let pressed = item.itemState.0 & 0x0004 != 0; // ODS_SELECTED

    let (face, text_color, border_color) = match id {
        ID_ACTION_BUTTON => (ACCENT, rgb(0x16, 0x16, 0x1A), ACCENT),
        ID_TOOLS_BUTTON => (SURFACE_2, TEXT_MAIN, BORDER),
        _ => {
            if current_filter_matches(id) { (blend(SURFACE_2, ACCENT, 0.16), ACCENT, blend(BORDER, ACCENT, 0.5)) }
            else { (SURFACE, TEXT_MUTED, BORDER) }
        }
    };
    let face = if pressed && id != ID_ACTION_BUTTON { blend(face, TEXT_MAIN, 0.06) } else { face };

    let brush = windows::Win32::Graphics::Gdi::CreateSolidBrush(COLORREF(face));
    FillRect(context, &rect, brush);
    let _ = DeleteObject(brush.into());

    if id != ID_ACTION_BUTTON {
        let pen = CreatePen(PS_SOLID, 1, COLORREF(border_color));
        let old_pen = SelectObject(context, pen.into());
        let old_brush = SelectObject(context, windows::Win32::Graphics::Gdi::GetStockObject(windows::Win32::Graphics::Gdi::NULL_BRUSH));
        let radius = (7.0 * dpi_scale()) as i32;
        let _ = RoundRect(context, rect.left, rect.top, rect.right, rect.bottom, radius, radius);
        let _ = SelectObject(context, old_brush);
        let _ = SelectObject(context, old_pen);
        let _ = DeleteObject(pen.into());
    }

    // Active tab: accent underline.
    if id != ID_ACTION_BUTTON && id != ID_TOOLS_BUTTON && current_filter_matches(id) {
        let pen = CreatePen(PS_SOLID, (2.0 * dpi_scale()) as i32, COLORREF(ACCENT));
        let old_pen = SelectObject(context, pen.into());
        let _ = windows::Win32::Graphics::Gdi::MoveToEx(context, rect.left + (10.0 * dpi_scale()) as i32, rect.bottom - 2, None);
        let _ = windows::Win32::Graphics::Gdi::LineTo(context, rect.right - (10.0 * dpi_scale()) as i32, rect.bottom - 2);
        let _ = SelectObject(context, old_pen);
        let _ = DeleteObject(pen.into());
    }

    let _ = SetBkMode(context, TRANSPARENT);
    let _ = SetTextColor(context, COLORREF(text_color));
    let font = if id == ID_ACTION_BUTTON { fonts.semibold } else { fonts.body };
    let old = SelectObject(context, font.into());
    let size = text_size(context, &label, font);
    let x = rect.left + (rect.right - rect.left - size.cx) / 2;
    let y = rect.top + (rect.bottom - rect.top - size.cy) / 2;
    draw_text_left(context, &label, x, y);
    if item.itemState.0 & 0x0010 != 0 { // ODS_FOCUS
        let _ = windows::Win32::Graphics::Gdi::DrawFocusRect(context, &rect);
    }
    let _ = SelectObject(context, old);
}

unsafe fn draw_list_row(item: &windows::Win32::UI::Controls::DRAWITEMSTRUCT) {
    use windows::Win32::Foundation::{COLORREF, RECT};
    use windows::Win32::Graphics::Gdi::{CreatePen, CreateSolidBrush, DeleteObject, Ellipse, FillRect, RoundRect, SelectObject, SetBkMode, SetTextColor, TRANSPARENT, PS_SOLID};

    let index = item.itemID as usize;
    let context = item.hDC;
    let rect = item.rcItem;
    let fonts = fonts_for(CURRENT_DPI.load(Ordering::SeqCst));
    let _ = SetBkMode(context, TRANSPARENT);

    let row = STATE.with(|s| s.borrow().as_ref().and_then(|state| {
        let snapshot = state.snapshot.lock().unwrap();
        snapshot.rows.get(index).map(|row| (
            row.display_name.clone(),
            row.schedule_text(),
            row.source_text(),
            row.status_text().trim_start_matches(['\u{25CF}', '\u{25CB}', '\u{25D0}']).trim().to_string(),
            row.aggregate_impact_text(),
            row.entry_text(),
            row.effective_enabled(),
            row.all_enabled(),
            row.all_disabled(),
            row.routes.len(),
        ))
    }));
    let Some((name, schedule, source, status, impact, entry, enabled, all_enabled, all_disabled, route_count)) = row else {
        return;
    };

    let selected = item.itemState.0 & 0x0001 != 0; // ODS_SELECTED
    let mut list_rect = RECT::default();
    let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(item.hwndItem, &mut list_rect);
    let layout = layout_metrics(client_width_from_list(list_rect.right - list_rect.left));

    // Row background: zebra + selection tint + accent bar.
    let face = if selected { SELECTION }
        else if index % 2 == 1 { SURFACE }
        else { blend(BG, SURFACE, 0.55) };
    let brush = CreateSolidBrush(COLORREF(face));
    FillRect(context, &rect, brush);
    let _ = DeleteObject(brush.into());
    if selected {
        let pen = CreatePen(PS_SOLID, 3, COLORREF(ACCENT));
        let old_pen = SelectObject(context, pen.into());
        let _ = windows::Win32::Graphics::Gdi::MoveToEx(context, rect.left + 1, rect.top, None);
        let _ = windows::Win32::Graphics::Gdi::LineTo(context, rect.left + 1, rect.bottom);
        let _ = SelectObject(context, old_pen);
        let _ = DeleteObject(pen.into());
    }

    let s = dpi_scale();
    let center_y = rect.top + (rect.bottom - rect.top) / 2;

    // Status dot.
    let dot_color = if all_enabled { GOOD } else if all_disabled { DANGER } else { ACCENT };
    let dot = (4.0 * s) as i32;
    let dot_brush = CreateSolidBrush(COLORREF(dot_color));
    let old_brush = SelectObject(context, dot_brush.into());
    let _ = Ellipse(context, (20.0 * s) as i32, center_y - dot, (20.0 * s) as i32 + dot * 2, center_y + dot);
    let _ = SelectObject(context, old_brush);
    let _ = DeleteObject(dot_brush.into());

    // Name (line 1).
    let name_x = (44.0 * s) as i32;
    let status_end = layout.schedule_x - (14.0 * s) as i32;
    let name_w = (status_end - name_x - (120.0 * s) as i32).max(80);
    let _ = SelectObject(context, fonts.semibold.into());
    let _ = SetTextColor(context, COLORREF(TEXT_MAIN));
    let name_rect = RECT { left: rect.left + name_x, top: rect.top + (9.0 * s) as i32, right: rect.left + name_x + name_w, bottom: rect.top + (30.0 * s) as i32 };
    draw_text_clipped(context, &name, &name_rect, 0);

    // Status word, right-aligned just before the schedule column.
    let status_color = if all_enabled { GOOD } else if all_disabled { DANGER } else { ACCENT };
    let status_label = if route_count > 1 { format!("{} \u{00B7} {}", status, route_count) } else { status.to_string() };
    let _ = SelectObject(context, fonts.small.into());
    let _ = SetTextColor(context, COLORREF(status_color));
    let sw = text_width(context, &status_label, fonts.small);
    draw_text_left(context, &status_label, rect.left + status_end - sw, rect.top + (12.0 * s) as i32);

    // Entry / command (line 2, mono).
    let _ = SelectObject(context, fonts.mono.into());
    let _ = SetTextColor(context, COLORREF(TEXT_MUTED));
    let entry_rect = RECT { left: rect.left + name_x, top: rect.top + (32.0 * s) as i32, right: rect.left + layout.schedule_x - (12.0 * s) as i32, bottom: rect.top + (50.0 * s) as i32 };
    draw_text_clipped(context, &entry, &entry_rect, 0);

    // Schedule chip — the "time every week".
    if !schedule.is_empty() {
        let _ = SelectObject(context, fonts.mono.into());
        let tw = text_width(context, &schedule, fonts.mono);
        let chip_w = (tw + (20.0 * s) as i32).min((230.0 * s) as i32);
        let chip_h = (24.0 * s) as i32;
        let chip_x = rect.left + layout.schedule_x;
        let chip_y = center_y - chip_h / 2;
        let chip_face = blend(SURFACE_2, ACCENT, 0.13);
        let brush = CreateSolidBrush(COLORREF(chip_face));
        let old_brush = SelectObject(context, brush.into());
        let pen = CreatePen(PS_SOLID, 1, COLORREF(blend(BORDER, ACCENT, 0.35)));
        let old_pen = SelectObject(context, pen.into());
        let _ = RoundRect(context, chip_x, chip_y, chip_x + chip_w, chip_y + chip_h, (7.0 * s) as i32, (7.0 * s) as i32);
        let _ = SelectObject(context, old_pen);
        let _ = SelectObject(context, old_brush);
        let _ = DeleteObject(pen.into());
        let _ = DeleteObject(brush.into());
        let _ = SetTextColor(context, COLORREF(ACCENT));
        let chip_rect = RECT { left: chip_x + (10.0 * s) as i32, top: chip_y, right: chip_x + chip_w - (6.0 * s) as i32, bottom: chip_y + chip_h };
        draw_text_clipped(context, &schedule, &chip_rect, (chip_h - (15.0 * s) as i32) / 2);
    }

    // Source.
    let _ = SelectObject(context, fonts.small.into());
    let _ = SetTextColor(context, COLORREF(TEXT_MUTED));
    let source_rect = RECT { left: rect.left + layout.source_x, top: center_y - (8.0 * s) as i32, right: rect.left + layout.toggle_x - (74.0 * s) as i32, bottom: center_y + (12.0 * s) as i32 };
    draw_text_clipped(context, &source, &source_rect, 0);

    // Impact (right-aligned before the toggle).
    let impact_color = match impact.as_str() {
        "High impact" => DANGER, "Medium impact" => ACCENT, _ => TEXT_MUTED,
    };
    let _ = SetTextColor(context, COLORREF(impact_color));
    let iw = text_width(context, &impact, fonts.small);
    draw_text_left(context, &impact, rect.left + layout.toggle_x - (68.0 * s) as i32 - iw, center_y - (8.0 * s) as i32);

    // Toggle switch (the one-click enable/disable).
    let tx = rect.left + layout.toggle_x;
    let ty = center_y - layout.toggle_h / 2;
    let pen = CreatePen(PS_SOLID, 1, COLORREF(if enabled { GOOD } else { blend(BORDER, TEXT_MAIN, 0.12) }));
    let track_brush = CreateSolidBrush(COLORREF(if enabled { blend(GOOD, BG, 0.72) } else { SURFACE_2 }));
    let old_pen = SelectObject(context, pen.into());
    let old_brush = SelectObject(context, track_brush.into());
    let _ = RoundRect(context, tx, ty, tx + layout.toggle_w, ty + layout.toggle_h, layout.toggle_h, layout.toggle_h);
    let _ = SelectObject(context, old_brush);
    let _ = SelectObject(context, old_pen);
    let _ = DeleteObject(pen.into());
    let _ = DeleteObject(track_brush.into());
    let knob = CreateSolidBrush(COLORREF(if enabled { GOOD } else { TEXT_MUTED }));
    let old_brush = SelectObject(context, knob.into());
    let knob_r = layout.toggle_h / 2 - (2.0 * s) as i32;
    let knob_x = if enabled { tx + layout.toggle_w - knob_r - (3.0 * s) as i32 } else { tx + (3.0 * s) as i32 };
    let _ = Ellipse(context, knob_x - knob_r, ty + layout.toggle_h / 2 - knob_r, knob_x + knob_r, ty + layout.toggle_h / 2 + knob_r);
    let _ = SelectObject(context, old_brush);
    let _ = DeleteObject(knob.into());
}

fn current_filter_matches(tab_id: i32) -> bool {
    STATE.with(|s| s.borrow().as_ref().map(|state| {
        let filter = state.snapshot.lock().unwrap().filter.clone();
        match tab_id {
            ID_TAB_APPS => filter == "Apps",
            ID_TAB_ENABLED => filter == "Enabled",
            ID_TAB_DISABLED => filter == "Disabled",
            _ => false,
        }
    }).unwrap_or(false))
}

unsafe fn control_text(hwnd: windows::Win32::Foundation::HWND) -> String {
    use windows::Win32::UI::WindowsAndMessaging::GetWindowTextLengthW;
    let length = GetWindowTextLengthW(hwnd);
    let mut buffer = vec![0u16; (length + 1) as usize];
    let copied = windows::Win32::UI::WindowsAndMessaging::GetWindowTextW(hwnd, &mut buffer);
    String::from_utf16_lossy(&buffer[..copied.max(0) as usize])
}

fn action_label() -> String {
    let label = STATE.with(|s| s.borrow().as_ref().and_then(|state| {
        let snapshot = state.snapshot.lock().unwrap();
        snapshot.rows.get(snapshot.selected).map(|row| {
            let enable = !row.effective_enabled();
            let verb = if enable { "Enable" } else { "Disable" };
            let routes = msm_ui::view::bulk_state_candidates(row, enable).len().max(1);
            if row.is_ambiguous() && routes > 1 { format!("{} \u{00B7} {} routes", verb, routes) } else { verb.to_string() }
        })
    }));
    label.unwrap_or_else(|| "Disable".into())
}

fn update_action_button() {
    unsafe {
        let hwnd = windows::Win32::Foundation::HWND(MAIN_HWND.load(Ordering::SeqCst) as *mut _);
        if hwnd.0.is_null() { return; }
        if let Ok(button) = windows::Win32::UI::WindowsAndMessaging::GetDlgItem(Some(hwnd), ID_ACTION_BUTTON) {
            let label = wstr(&action_label());
            let _ = windows::Win32::UI::WindowsAndMessaging::SetWindowTextW(button, windows::core::PCWSTR(label.as_ptr()));
        }
    }
}

// ---------------------------------------------------------------------------
// List behavior — one-click toggle switch, context menu, keyboard
// ---------------------------------------------------------------------------
static OLD_LIST_PROC: AtomicIsize = AtomicIsize::new(0);

unsafe fn subclass_list(hwnd: windows::Win32::Foundation::HWND) {
    // Direct GWLP_WNDPROC subclassing: no comctl32 dependency, always installed.
    use windows::Win32::UI::WindowsAndMessaging::{GWLP_WNDPROC, SetWindowLongPtrW};
    let previous = SetWindowLongPtrW(hwnd, GWLP_WNDPROC, list_subclass_proc as usize as isize);
    OLD_LIST_PROC.store(previous, Ordering::SeqCst);
}

unsafe extern "system" fn list_subclass_proc(hwnd: windows::Win32::Foundation::HWND, message: u32, wparam: windows::Win32::Foundation::WPARAM, lparam: windows::Win32::Foundation::LPARAM) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::*;
    fn default_list_proc(hwnd: windows::Win32::Foundation::HWND, message: u32, wparam: windows::Win32::Foundation::WPARAM, lparam: windows::Win32::Foundation::LPARAM) -> LRESULT {
        unsafe {
            let previous = OLD_LIST_PROC.load(Ordering::SeqCst);
            if previous == 0 { return DefWindowProcW(hwnd, message, wparam, lparam) }
            let proc: WNDPROC = std::mem::transmute(previous as *const ());
            CallWindowProcW(proc, hwnd, message, wparam, lparam)
        }
    }
    match message {
        WM_LBUTTONUP => {
            let x = (lparam.0 & 0xFFFF) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
            if let Some(index) = item_at_point(hwnd, x, y) {
                let mut rect = windows::Win32::Foundation::RECT::default();
                let _ = SendMessageW(hwnd, LB_GETITEMRECT as u32, Some(WPARAM(index)), Some(LPARAM(&mut rect as *mut _ as isize)));
                let mut list_rect = windows::Win32::Foundation::RECT::default();
                let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut list_rect);
                let layout = layout_metrics(client_width_from_list(list_rect.right - list_rect.left));
                if x >= rect.left + layout.toggle_x - (10.0 * dpi_scale()) as i32 && x <= rect.left + layout.toggle_x + layout.toggle_w + (10.0 * dpi_scale()) as i32 {
                    toggle_row(index);
                    return LRESULT(0);
                }
                remember_selection(index);
            }
            default_list_proc(hwnd, message, wparam, lparam)
        }
        WM_LBUTTONDBLCLK => {
            let x = (lparam.0 & 0xFFFF) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
            if let Some(index) = item_at_point(hwnd, x, y) {
                remember_selection(index);
                open_edit_dialog();
                return LRESULT(0);
            }
            default_list_proc(hwnd, message, wparam, lparam)
        }
        WM_RBUTTONUP => {
            let x = (lparam.0 & 0xFFFF) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
            if let Some(index) = item_at_point(hwnd, x, y) {
                remember_selection(index);
                show_row_context_menu();
                return LRESULT(0);
            }
            default_list_proc(hwnd, message, wparam, lparam)
        }
        WM_KEYDOWN => {
            let key = (wparam.0 & 0xFFFF) as i32;
            if key == 0x0D { // VK_RETURN
                let current = SendMessageW(hwnd, LB_GETCURSEL as u32, Some(WPARAM(0)), Some(LPARAM(0))).0;
                if current >= 0 {
                    toggle_row(current as usize);
                    return LRESULT(0);
                }
            } else if key == 0x41 && get_key_state_ctrl() { // Ctrl+A
                copy_visible_tsv();
                return LRESULT(0);
            } else if key == 0x43 && get_key_state_ctrl() { // Ctrl+C
                copy_selected_tsv();
                return LRESULT(0);
            } else if key == 0x46 && get_key_state_ctrl() { // Ctrl+F
                focus_search();
                return LRESULT(0);
            }
            default_list_proc(hwnd, message, wparam, lparam)
        }
        _ => default_list_proc(hwnd, message, wparam, lparam),
    }
}

fn get_key_state_ctrl() -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_CONTROL};
    unsafe { (GetAsyncKeyState(VK_CONTROL.0 as i32) as u16) & 0x8000 != 0 }
}

unsafe fn item_at_point(hwnd: windows::Win32::Foundation::HWND, x: i32, y: i32) -> Option<usize> {
    let point = windows::Win32::Foundation::POINT { x, y };
    let index = windows::Win32::UI::Controls::LBItemFromPt(hwnd, point, false);
    if index < 0 { None } else { Some(index as usize) }
}

fn remember_selection(index: usize) {
    STATE.with(|s| {
        if let Some(state) = s.borrow().as_ref() {
            state.snapshot.lock().unwrap().selected = index;
        }
    });
    update_action_button();
}

fn focus_search() {
    unsafe {
        let hwnd = windows::Win32::Foundation::HWND(MAIN_HWND.load(Ordering::SeqCst) as *mut _);
        if hwnd.0.is_null() { return; }
        if let Ok(search) = windows::Win32::UI::WindowsAndMessaging::GetDlgItem(Some(hwnd), ID_SEARCH) {
            let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(Some(search));
        }
    }
}

fn handle_list_notify(code: u32) {
    // LBN_SELCHANGE only records the selection; actions fire from the row
    // toggle switch, the primary button, or Enter — never a bare click.
    if code == 1 {
        let list = list_view_hwnd();
        if list.0.is_null() { return; }
        unsafe {
            let selection = windows::Win32::UI::WindowsAndMessaging::SendMessageW(list, 0x0188 /*LB_GETCURSEL*/, Some(windows::Win32::Foundation::WPARAM(0)), Some(windows::Win32::Foundation::LPARAM(0))).0;
            if selection >= 0 { remember_selection(selection as usize); }
        }
    }
}

fn list_view_hwnd() -> windows::Win32::Foundation::HWND {
    unsafe {
        let main = windows::Win32::Foundation::HWND(MAIN_HWND.load(Ordering::SeqCst) as *mut _);
        if main.0.is_null() { return windows::Win32::Foundation::HWND::default(); }
        windows::Win32::UI::WindowsAndMessaging::GetDlgItem(Some(main), ID_LIST).unwrap_or_default()
    }
}

fn refresh_list() {
    let list = list_view_hwnd();
    if list.0.is_null() { return; }
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::*;
    unsafe {
        let (count, selected) = STATE.with(|s| s.borrow().as_ref().map(|state| {
            let mut snapshot = state.snapshot.lock().unwrap();
            snapshot.selected = snapshot.selected.min(snapshot.rows.len().saturating_sub(1));
            (snapshot.rows.len(), snapshot.selected)
        }).unwrap_or((0, 0)));
        let _ = SendMessageW(list, LB_RESETCONTENT as u32, Some(WPARAM(0)), Some(LPARAM(0)));
        for _ in 0..count {
            let _ = SendMessageW(list, LB_ADDSTRING as u32, Some(WPARAM(0)), Some(LPARAM(0)));
        }
        if count > 0 {
            let _ = SendMessageW(list, LB_SETCURSEL as u32, Some(WPARAM(selected)), Some(LPARAM(0)));
        }
        // Hide the list when empty so the designed empty state shows through.
        let _ = ShowWindow(list, if count > 0 { SW_SHOW } else { SW_HIDE });
    }
    update_action_button();
    invalidate_main();
}

fn apply_search() {
    let query = unsafe {
        let hwnd = windows::Win32::Foundation::HWND(MAIN_HWND.load(Ordering::SeqCst) as *mut _);
        if hwnd.0.is_null() { return; }
        match windows::Win32::UI::WindowsAndMessaging::GetDlgItem(Some(hwnd), ID_SEARCH) {
            Ok(search) => control_text(search),
            Err(_) => return,
        }
    };
    STATE.with(|s| {
        if let Some(state) = s.borrow().as_ref() {
            let mut snapshot = state.snapshot.lock().unwrap();
            snapshot.query = query;
            snapshot.rows = msm_ui::view::build_visible_rows(&snapshot.items, &snapshot.filter, &snapshot.query);
        }
    });
    refresh_list();
}

fn set_filter(filter: &str) {
    STATE.with(|s| {
        if let Some(state) = s.borrow().as_ref() {
            let mut snapshot = state.snapshot.lock().unwrap();
            snapshot.filter = filter.to_string();
            snapshot.selected = 0;
            snapshot.rows = msm_ui::view::build_visible_rows(&snapshot.items, &snapshot.filter, &snapshot.query);
            *state.last_status.lock().unwrap() = format!("{} view \u{00B7} {} rows", filter, snapshot.rows.len());
        }
    });
    refresh_list();
    invalidate_main();
}

fn selected_row() -> Option<msm_ui::view::InventoryRow> {
    STATE.with(|s| s.borrow().as_ref().and_then(|state| {
        let snapshot = state.snapshot.lock().unwrap();
        if snapshot.selected >= snapshot.rows.len() { return None }
        Some(snapshot.rows[snapshot.selected].clone())
    }))
}

fn toggle_row(index: usize) {
    STATE.with(|s| {
        if let Some(state) = s.borrow().as_ref() {
            state.snapshot.lock().unwrap().selected = index;
        }
    });
    run_primary_action();
}

fn run_primary_action() {
    let selected = STATE.with(|s| {
        s.borrow().as_ref().and_then(|state| {
            let snapshot = state.snapshot.lock().unwrap();
            if snapshot.selected >= snapshot.rows.len() { return None }
            let row = &snapshot.rows[snapshot.selected];
            if row.is_ambiguous() {
                let enable_requested = !row.effective_enabled();
                let candidates = msm_ui::view::bulk_state_candidates(row, enable_requested);
                if candidates.is_empty() {
                    return Some((format!("{} routes are already {}.", row.display_name, if enable_requested { "enabled" } else { "disabled" }), None))
                }
                Some((format!("Changing {} routes for {}.", candidates.len(), row.display_name), Some((candidates, enable_requested, row.display_name.clone()))))
            } else {
                let item = row.routes[0].clone();
                let enable_requested = !item.enabled;
                Some((format!("{} {}.", if enable_requested { "Enabling" } else { "Disabling" }, item.human_name()), Some((vec![item], enable_requested, row.display_name.clone()))))
            }
        })
    });
    let Some((status, Some((candidates, enable_requested, label)))) = selected else {
        if let Some((status, None)) = selected { set_status(&status); }
        return;
    };
    set_status(&status);
    let result = if candidates.len() == 1 {
        let item = &candidates[0];
        if enable_requested { msm_actions::enable(item) } else { msm_actions::disable(item) }.map(|_| 1usize)
    } else if enable_requested {
        msm_actions::enable_all_routes(&candidates, false)
    } else {
        msm_actions::disable_all_routes(&candidates, false)
    };
    match result {
        Ok(count) => set_status(&format!("{} {} route{} for {}.", if enable_requested { "Enabled" } else { "Disabled" }, count, if count == 1 { "" } else { "s" }, label)),
        Err(e) => set_status(&format!("Action failed and everything was restored: {}", e)),
    }
    request_rescan();
}

fn request_rescan() {
    STATE.with(|s| {
        if let Some(state) = s.borrow().as_ref() {
            run_inventory_refresh(state.clone());
        }
    });
}

// ---------------------------------------------------------------------------
// Menus
// ---------------------------------------------------------------------------
unsafe fn append_menu(menu: windows::Win32::UI::WindowsAndMessaging::HMENU, label: &str, id: usize) {
    use windows::Win32::UI::WindowsAndMessaging::*;
    let wide = wstr(label);
    let _ = AppendMenuW(menu, MF_STRING, id, windows::core::PCWSTR(wide.as_ptr()));
}

fn show_tools_menu(hwnd: windows::Win32::Foundation::HWND) {
    use windows::Win32::UI::WindowsAndMessaging::*;
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        append_menu(menu, "Add startup\u{2026}", MENU_ADD as usize);
        append_menu(menu, "Edit startup\u{2026}", MENU_EDIT as usize);
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        append_menu(menu, "Disable at boot", MENU_DISABLE as usize);
        append_menu(menu, "Enable at boot", MENU_ENABLE as usize);
        append_menu(menu, "Quiet (tray)", MENU_QUIET as usize);
        append_menu(menu, "Window mode", MENU_WINDOW as usize);
        append_menu(menu, "Run now", MENU_RUN_NOW as usize);
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        append_menu(menu, "Open location", MENU_OPEN_LOCATION as usize);
        append_menu(menu, "Copy command", MENU_COPY_COMMAND as usize);
        append_menu(menu, "Copy selected startup data", MENU_COPY_SELECTED as usize);
        append_menu(menu, "Copy visible startup data", MENU_COPY_VISIBLE as usize);
        append_menu(menu, "Copy all startup data", MENU_COPY_ALL as usize);
        append_menu(menu, "Select all visible", MENU_SELECT_ALL as usize);
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        append_menu(menu, "Protect disabled", MENU_PROTECT as usize);
        append_menu(menu, "Repair startup rules", MENU_REPAIR as usize);
        append_menu(menu, "Verify coverage", MENU_VERIFY as usize);
        append_menu(menu, "Open startup folders", MENU_OPEN_FOLDERS as usize);
        append_menu(menu, "Permanently delete managed task", MENU_DELETE_MANAGED as usize);
        let needs_selection = [MENU_EDIT, MENU_DISABLE, MENU_ENABLE, MENU_QUIET, MENU_WINDOW, MENU_RUN_NOW,
            MENU_OPEN_LOCATION, MENU_COPY_COMMAND, MENU_COPY_SELECTED, MENU_DELETE_MANAGED];
        if selected_row().is_none() {
            for id in needs_selection {
                let _ = EnableMenuItem(menu, id as u32, MF_GRAYED);
            }
        }
        let mut point = windows::Win32::Foundation::POINT::default();
        let _ = GetCursorPos(&mut point);
        let _ = TrackPopupMenu(menu, TPM_RIGHTBUTTON, point.x, point.y, None, hwnd, None);
        let _ = DestroyMenu(menu);
    }
}

fn show_row_context_menu() {
    unsafe {
        let hwnd = windows::Win32::Foundation::HWND(MAIN_HWND.load(Ordering::SeqCst) as *mut _);
        if hwnd.0.is_null() { return; }
        let Ok(menu) = windows::Win32::UI::WindowsAndMessaging::CreatePopupMenu() else { return };
        let enabled = selected_row().map(|row| row.effective_enabled()).unwrap_or(false);
        let (verb, action_id) = if enabled { ("Disable", MENU_DISABLE) } else { ("Enable", MENU_ENABLE) };
        append_menu(menu, verb, action_id as usize);
        append_menu(menu, "Edit…", MENU_EDIT as usize);
        let managed = selected_row().map(|row| row.has_managed()).unwrap_or(false);
        if managed {
            append_menu(menu, "Quiet (tray)", MENU_QUIET as usize);
            append_menu(menu, "Window mode", MENU_WINDOW as usize);
        }
        append_menu(menu, "Run now", MENU_RUN_NOW as usize);
        let _ = windows::Win32::UI::WindowsAndMessaging::AppendMenuW(menu, windows::Win32::UI::WindowsAndMessaging::MF_SEPARATOR, 0, None);
        append_menu(menu, "Open location", MENU_OPEN_LOCATION as usize);
        append_menu(menu, "Copy command", MENU_COPY_COMMAND as usize);
        append_menu(menu, "Copy startup data", MENU_COPY_SELECTED as usize);
        append_menu(menu, "Protect disabled", MENU_PROTECT as usize);
        if managed {
            append_menu(menu, "Permanently delete managed task", MENU_DELETE_MANAGED as usize);
        }
        let mut point = windows::Win32::Foundation::POINT::default();
        let _ = windows::Win32::UI::WindowsAndMessaging::SetForegroundWindow(hwnd);
        let _ = windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut point);
        let _ = windows::Win32::UI::WindowsAndMessaging::TrackPopupMenu(menu, windows::Win32::UI::WindowsAndMessaging::TPM_RIGHTBUTTON, point.x, point.y, None, hwnd, None);
        let _ = windows::Win32::UI::WindowsAndMessaging::DestroyMenu(menu);
    }
}

fn handle_command(hwnd: windows::Win32::Foundation::HWND, id: i32) {
    match id {
        ID_TAB_APPS => set_filter("Apps"),
        ID_TAB_ENABLED => set_filter("Enabled"),
        ID_TAB_DISABLED => set_filter("Disabled"),
        ID_ACTION_BUTTON => run_primary_action(),
        ID_TOOLS_BUTTON => show_tools_menu(hwnd),
        ID_FOCUS_SEARCH => focus_search(),
        MENU_ADD => open_add_dialog(),
        MENU_EDIT => open_edit_dialog(),
        MENU_DISABLE | MENU_ENABLE => run_primary_action(),
        MENU_QUIET => set_managed_mode(true),
        MENU_WINDOW => set_managed_mode(false),
        MENU_RUN_NOW => run_selected_now(),
        MENU_OPEN_LOCATION => open_selected_location(),
        MENU_COPY_COMMAND => copy_command(),
        MENU_COPY_SELECTED => copy_selected_tsv(),
        MENU_COPY_VISIBLE | MENU_SELECT_ALL => copy_visible_tsv(),
        MENU_COPY_ALL => copy_all_tsv(),
        MENU_PROTECT => protect_disabled(),
        MENU_REPAIR => repair_rules(),
        MENU_VERIFY => verify_coverage(),
        MENU_OPEN_FOLDERS => open_startup_folders(),
        MENU_DELETE_MANAGED => delete_selected_managed(),
        MENU_TRAY_SHOW => tray_command_show(),
        MENU_TRAY_REFRESH => request_rescan(),
        MENU_TRAY_QUIT => tray_command_quit(),
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Tools implementations
// ---------------------------------------------------------------------------
struct DialogSpec<'a> {
    title: &'a str,
    fields: &'a [(&'a str, String, bool)], // label, prefill, mono
    mode_toggle: bool,
}

thread_local! {
    static DIALOG_RESULT: std::cell::RefCell<Option<Vec<String>>> = const { std::cell::RefCell::new(None) };
    static DIALOG_MODE: std::cell::RefCell<bool> = const { std::cell::RefCell::new(false) };
    static DIALOG_OPEN: std::cell::RefCell<bool> = const { std::cell::RefCell::new(false) };
    static DIALOG_SPEC: std::cell::RefCell<Option<SpecCopy>> = const { std::cell::RefCell::new(None) };
}

#[derive(Clone)]
struct SpecCopy {
    fields: Vec<(String, String, bool)>,
    mode_toggle: bool,
}

const DLG_FIELD_BASE: i32 = 3400;

fn open_add_dialog() {
    let Some(values) = modal_dialog(&DialogSpec {
        title: "Add startup",
        fields: &[
            ("Name", String::new(), false),
            ("Program", String::new(), true),
            ("Arguments", String::new(), true),
        ],
        mode_toggle: false,
    }) else { return };
    if values[0].trim().is_empty() || values[1].trim().is_empty() {
        set_status("Add startup needs a name and a program.");
        return;
    }
    let quiet = DIALOG_MODE.with(|m| *m.borrow());
    match msm_actions::managed::add_managed_startup(values[0].trim(), values[1].trim(), values[2].trim(), quiet, false) {
        Ok(_) => set_status(&format!("Added {} \u{00B7} runs at every log on.", values[0].trim())),
        Err(e) => set_status(&format!("Add failed: {}", e)),
    }
    request_rescan();
}

fn open_edit_dialog() {
    let Some(row) = selected_row() else {
        set_status("Select a startup route first.");
        return;
    };
    let item = row.primary_item().clone();
    let (target, arguments) = msm_actions::resolve_launch_target_or_fail(&item)
        .unwrap_or_else(|_| split_command(&item.command));
    let can_edit_mode = msm_actions::managed::can_mutate_startup_mode(&item);
    let Some(values) = modal_dialog(&DialogSpec {
        title: "Edit startup",
        fields: &[
            ("Name", row.display_name.clone(), false),
            ("Program", target, true),
            ("Arguments", arguments, true),
        ],
        mode_toggle: can_edit_mode,
    }) else { return };
    if values[0].trim().is_empty() || values[1].trim().is_empty() {
        set_status("Edit needs a name and a program.");
        return;
    }
    let quiet = DIALOG_MODE.with(|m| *m.borrow());
    match msm_actions::managed::edit_startup(&item, values[0].trim(), values[1].trim(), values[2].trim(), quiet) {
        Ok(_) => set_status(&format!("Updated {}.", values[0].trim())),
        Err(e) => set_status(&format!("Edit failed: {}", e)),
    }
    request_rescan();
}

fn split_command(command: &str) -> (String, String) {
    let trimmed = command.trim();
    if let Some(stripped) = trimmed.strip_prefix('"') {
        if let Some(end) = stripped.find('"') {
            return (stripped[..end].to_string(), stripped[end + 1..].trim().to_string());
        }
    }
    match trimmed.split_once(' ') {
        Some((head, rest)) => (head.to_string(), rest.trim().to_string()),
        None => (trimmed.to_string(), String::new()),
    }
}

fn modal_dialog(spec: &DialogSpec) -> Option<Vec<String>> {
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::*;
    unsafe {
        let parent = windows::Win32::Foundation::HWND(MAIN_HWND.load(Ordering::SeqCst) as *mut _);
        if parent.0.is_null() { return None; }
        let instance = GetModuleHandleW(None).unwrap_or_default();
        let scale = dpi_scale();
        let width = (520.0 * scale) as i32;
        let row_h = (58.0 * scale) as i32;
        let height = (112.0 * scale) as i32 + row_h * spec.fields.len() as i32 + (if spec.mode_toggle { row_h } else { 0 });
        let class_name = wstr("MichStartupMasterDialog");
        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(dialog_proc),
            hInstance: instance.into(),
            lpszClassName: windows::core::PCWSTR(class_name.as_ptr()),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hbrBackground: windows::Win32::Graphics::Gdi::HBRUSH::default(),
            ..Default::default()
        };
        let _ = RegisterClassW(&wc); // already-registered on the second dialog is fine
        let title = wstr(spec.title);
        let mut parent_rect = windows::Win32::Foundation::RECT::default();
        let _ = GetWindowRect(parent, &mut parent_rect);
        let x = parent_rect.left + ((parent_rect.right - parent_rect.left) - width) / 2;
        let y = parent_rect.top + ((parent_rect.bottom - parent_rect.top) - height) / 3;
        DIALOG_RESULT.with(|r| *r.borrow_mut() = None);
        DIALOG_MODE.with(|m| *m.borrow_mut() = false);
        DIALOG_OPEN.with(|o| *o.borrow_mut() = true);
        DIALOG_SPEC.with(|s| *s.borrow_mut() = Some(SpecCopy {
            fields: spec.fields.iter().map(|(label, value, mono)| (label.to_string(), value.clone(), *mono)).collect(),
            mode_toggle: spec.mode_toggle,
        }));
        let hwnd = CreateWindowExW(
            WS_EX_DLGMODALFRAME | WS_EX_TOPMOST,
            windows::core::PCWSTR(class_name.as_ptr()),
            windows::core::PCWSTR(title.as_ptr()),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU,
            x, y, width, height,
            Some(parent), None, Some(instance.into()), None).ok();
        let Some(dialog) = hwnd else { return None; };
        enable_dark_title_bar(dialog);
        let _ = ShowWindow(dialog, SW_SHOW);
        let _ = windows::Win32::Graphics::Gdi::UpdateWindow(dialog);
        let _ = SetForegroundWindow(dialog);
        let _ = windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow(parent, false);
        let mut message = MSG::default();
        loop {
            let result = GetMessageW(&mut message, None, 0, 0);
            if result.0 <= 0 { break }
            if !DIALOG_OPEN.with(|o| *o.borrow()) { break }
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        let _ = windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow(parent, true);
        let _ = SetForegroundWindow(parent);
        DIALOG_RESULT.with(|r| r.borrow().clone())
    }
}

unsafe extern "system" fn dialog_proc(hwnd: windows::Win32::Foundation::HWND, message: u32, wparam: windows::Win32::Foundation::WPARAM, lparam: windows::Win32::Foundation::LPARAM) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::Foundation::{LPARAM, LRESULT, RECT, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::*;
    match message {
        WM_CREATE => {
            let instance = windows::Win32::System::LibraryLoader::GetModuleHandleW(None).unwrap_or_default();
            let scale = dpi_scale();
            let spec = DIALOG_SPEC.with(|s| s.borrow().clone());
            let Some(spec) = spec else { return LRESULT(-1) };
            let SpecCopy { fields, mode_toggle } = spec;
            let mut y = (44.0 * scale) as i32;
            let label_w = (96.0 * scale) as i32;
            let field_w = (520.0 * scale) as i32 - label_w - (110.0 * scale) as i32;
            for (index, (label, prefill, mono)) in fields.iter().enumerate() {
                let label_wide = wstr(label);
                let static_class = wstr("STATIC");
                let _ = CreateWindowExW(Default::default(), windows::core::PCWSTR(static_class.as_ptr()),
                    windows::core::PCWSTR(label_wide.as_ptr()),
                    WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0),
                    (24.0 * scale) as i32, y + 8, label_w, (20.0 * scale) as i32,
                    Some(hwnd), None, Some(instance.into()), None);
                let edit_class = wstr("EDIT");
                let prefill_wide = wstr(prefill);
                let field = CreateWindowExW(Default::default(), windows::core::PCWSTR(edit_class.as_ptr()),
                    windows::core::PCWSTR(prefill_wide.as_ptr()),
                    WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | ES_AUTOHSCROLL as u32),
                    (24.0 * scale) as i32 + label_w, y, field_w, (28.0 * scale) as i32,
                    Some(hwnd), Some(HMENU((DLG_FIELD_BASE + index as i32) as *mut _)), Some(instance.into()), None).ok();
                if let Some(field) = field {
                    apply_control_font(field);
                    if *mono {
                        let font = create_font_dpi(13, 400, "Cascadia Mono", CURRENT_DPI.load(Ordering::SeqCst));
                        let _ = SendMessageW(field, WM_SETFONT as u32, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
                    }
                }
                y += (58.0 * scale) as i32;
            }
            if mode_toggle {
                let text = wstr("Startup mode: Window");
                let button_class = wstr("BUTTON");
                let _ = CreateWindowExW(Default::default(), windows::core::PCWSTR(button_class.as_ptr()),
                    windows::core::PCWSTR(text.as_ptr()),
                    WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | BS_OWNERDRAW as u32 | WS_TABSTOP.0),
                    (24.0 * scale) as i32, y + 4, (240.0 * scale) as i32, (32.0 * scale) as i32,
                    Some(hwnd), Some(HMENU(DLG_ID_MODE as *mut _)), Some(instance.into()), None);
            }
            let button_y = y + (66.0 * scale) as i32 + (if mode_toggle { (6.0 * scale) as i32 } else { 0 });
            let buttons: [(&str, i32); 3] = [("OK", DLG_ID_OK), ("Cancel", DLG_ID_CANCEL), ("Browse\u{2026}", DLG_ID_BROWSE)];
            let mut bx = (520.0 * scale) as i32 - (24.0 * scale) as i32 - 3 * ((104.0 * scale) as i32) - (16.0 * scale) as i32;
            for (label, id) in buttons {
                let wide = wstr(label);
                let button_class = wstr("BUTTON");
                let _ = CreateWindowExW(Default::default(), windows::core::PCWSTR(button_class.as_ptr()),
                    windows::core::PCWSTR(wide.as_ptr()),
                    WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | BS_OWNERDRAW as u32 | WS_TABSTOP.0),
                    bx, button_y, (104.0 * scale) as i32, (34.0 * scale) as i32,
                    Some(hwnd), Some(HMENU(id as *mut _)), Some(instance.into()), None);
                bx += (104.0 * scale) as i32 + (8.0 * scale) as i32;
            }
            LRESULT(0)
        }
        WM_PAINT => {
            use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, PAINTSTRUCT};
            let mut paint = PAINTSTRUCT::default();
            let context = BeginPaint(hwnd, &mut paint);
            if !context.is_invalid() {
                let mut client = RECT::default();
                let _ = GetClientRect(hwnd, &mut client);
                let brush = windows::Win32::Graphics::Gdi::CreateSolidBrush(windows::Win32::Foundation::COLORREF(BG));
                windows::Win32::Graphics::Gdi::FillRect(context, &client, brush);
                let _ = windows::Win32::Graphics::Gdi::DeleteObject(brush.into());
                let _ = EndPaint(hwnd, &paint);
            }
            LRESULT(0)
        }
        WM_CTLCOLORSTATIC => {
            let hdc = windows::Win32::Graphics::Gdi::HDC(wparam.0 as *mut _);
            let _ = windows::Win32::Graphics::Gdi::SetTextColor(hdc, windows::Win32::Foundation::COLORREF(TEXT_MUTED));
            let _ = windows::Win32::Graphics::Gdi::SetBkColor(hdc, windows::Win32::Foundation::COLORREF(BG));
            LRESULT(dark_brush(BG).0 as isize)
        }
        WM_CTLCOLOREDIT => {
            let hdc = windows::Win32::Graphics::Gdi::HDC(wparam.0 as *mut _);
            let _ = windows::Win32::Graphics::Gdi::SetTextColor(hdc, windows::Win32::Foundation::COLORREF(TEXT_MAIN));
            let _ = windows::Win32::Graphics::Gdi::SetBkColor(hdc, windows::Win32::Foundation::COLORREF(SURFACE_2));
            LRESULT(dark_brush(SURFACE_2).0 as isize)
        }
        WM_DRAWITEM => { on_draw_item(wparam, lparam); LRESULT(1) }
        WM_COMMAND => {
            let id = (wparam.0 & 0xFFFF) as i32;
            if id == DLG_ID_OK || id == DLG_ID_CANCEL {
                if id == DLG_ID_OK {
                    let fields = DIALOG_SPEC.with(|s| s.borrow().as_ref().map(|spec| spec.fields.len()).unwrap_or(0));
                    let mut values = Vec::new();
                    for index in 0..fields {
                        if let Ok(field) = GetDlgItem(Some(hwnd), DLG_FIELD_BASE + index as i32) {
                            values.push(control_text(field));
                        }
                    }
                    DIALOG_RESULT.with(|r| *r.borrow_mut() = Some(values));
                } else {
                    DIALOG_RESULT.with(|r| *r.borrow_mut() = None);
                }
                DIALOG_OPEN.with(|o| *o.borrow_mut() = false);
                let _ = DestroyWindow(hwnd);
                PostQuitMessage(0); // wakes the modal loop
                LRESULT(0)
            } else if id == DLG_ID_BROWSE {
                if let Some(path) = browse_program(hwnd) {
                    if let Ok(field) = GetDlgItem(Some(hwnd), DLG_FIELD_BASE + 1) {
                        let wide = wstr(&path);
                        let _ = SetWindowTextW(field, windows::core::PCWSTR(wide.as_ptr()));
                    }
                }
                LRESULT(0)
            } else if id == DLG_ID_MODE {
                let quiet = !DIALOG_MODE.with(|m| *m.borrow());
                DIALOG_MODE.with(|m| *m.borrow_mut() = quiet);
                if let Ok(button) = GetDlgItem(Some(hwnd), DLG_ID_MODE) {
                    let label = wstr(if quiet { "Startup mode: Quiet (tray)" } else { "Startup mode: Window" });
                    let _ = SetWindowTextW(button, windows::core::PCWSTR(label.as_ptr()));
                }
                LRESULT(0)
            } else {
                LRESULT(0)
            }
        }
        WM_CLOSE => {
            DIALOG_RESULT.with(|r| *r.borrow_mut() = None);
            DIALOG_OPEN.with(|o| *o.borrow_mut() = false);
            let _ = DestroyWindow(hwnd);
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

fn browse_program(owner: windows::Win32::Foundation::HWND) -> Option<String> {
    use windows::Win32::UI::Controls::Dialogs::{GetOpenFileNameW, OPENFILENAMEW, OFN_EXPLORER, OFN_FILEMUSTEXIST, OFN_PATHMUSTEXIST};
    let mut buffer = vec![0u16; 1024];
    let filter = wstr("Programs\0*.exe;*.bat;*.cmd;*.lnk;*.ps1\0All files\0*.*\0\0");
    let mut ofn = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: owner,
        lpstrFilter: windows::core::PCWSTR(filter.as_ptr()),
        lpstrFile: windows::core::PWSTR(buffer.as_mut_ptr()),
        nMaxFile: buffer.len() as u32,
        Flags: OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST,
        ..Default::default()
    };
    unsafe {
        if GetOpenFileNameW(&mut ofn).as_bool() {
            let end = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
            Some(String::from_utf16_lossy(&buffer[..end]))
        } else { None }
    }
}

fn set_managed_mode(quiet: bool) {
    let Some(row) = selected_row() else { return };
    let item = row.primary_item().clone();
    let (target, arguments) = msm_actions::resolve_launch_target_or_fail(&item)
        .unwrap_or_else(|_| split_command(&item.command));
    match msm_actions::managed::edit_startup(&item, item.human_name(), &target, &arguments, quiet) {
        Ok(_) => set_status(&format!("{} now starts {}.", item.human_name(), if quiet { "quietly in the tray" } else { "in a window" })),
        Err(e) => set_status(&format!("Mode change failed: {}", e)),
    }
    request_rescan();
}

fn run_selected_now() {
    let Some(row) = selected_row() else { return };
    let item = row.primary_item().clone();
    match msm_actions::resolve_launch_target_or_fail(&item) {
        Ok((target, arguments)) => {
            let command_line = if arguments.trim().is_empty() {
                format!("\"{}\"", target)
            } else {
                format!("\"{}\" {}", target, arguments)
            };
            let mut wide = wstr(&command_line);
            use windows::Win32::System::Threading::{CreateProcessW, PROCESS_CREATION_FLAGS, PROCESS_INFORMATION, STARTUPINFOW};
            let mut startup = STARTUPINFOW { cb: std::mem::size_of::<STARTUPINFOW>() as u32, ..Default::default() };
            let mut info = PROCESS_INFORMATION::default();
            let launched = unsafe {
                CreateProcessW(None, Some(windows::core::PWSTR(wide.as_mut_ptr())), None, None, false,
                    PROCESS_CREATION_FLAGS::default(), None, None, &startup, &mut info).is_ok()
            };
            if launched {
                set_status(&format!("Started {}.", item.human_name()));
                unsafe {
                    let _ = windows::Win32::Foundation::CloseHandle(info.hProcess);
                    let _ = windows::Win32::Foundation::CloseHandle(info.hThread);
                }
            } else {
                set_status(&format!("Could not start {}.", item.human_name()));
            }
        }
        Err(e) => set_status(&format!("Could not start {}: {}", item.human_name(), e)),
    }
}

fn open_selected_location() {
    let Some(row) = selected_row() else { return };
    let item = row.primary_item().clone();
    let location = if let Ok((target, _)) = msm_actions::resolve_launch_target_or_fail(&item) {
        let path = std::path::PathBuf::from(msm_core::util::expand_env(target.trim().trim_matches('"')));
        if path.is_file() { path } else { path.parent().map(|p| p.to_path_buf()).unwrap_or_default() }
    } else {
        std::path::PathBuf::new()
    };
    unsafe {
        let verb = wstr("open");
        let explorer = wstr("explorer.exe");
        if !location.as_os_str().is_empty() {
            let args = wstr(&format!("/select,\"{}\"", location.display()));
            let _ = windows::Win32::UI::Shell::ShellExecuteW(None, windows::core::PCWSTR(verb.as_ptr()),
                windows::core::PCWSTR(explorer.as_ptr()), windows::core::PCWSTR(args.as_ptr()), None,
                windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL);
            set_status(&format!("Opened location of {}.", item.human_name()));
        } else if item.location.starts_with("HK") {
            let regedit = wstr("regedit.exe");
            let _ = windows::Win32::UI::Shell::ShellExecuteW(None, windows::core::PCWSTR(verb.as_ptr()),
                windows::core::PCWSTR(regedit.as_ptr()), None, None, windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL);
            set_status(&format!("Opened Registry Editor for {}.", item.human_name()));
        } else {
            set_status("No on-disk location for this route.");
        }
    }
}

fn copy_command() {
    let Some(row) = selected_row() else { return };
    copy_to_clipboard(&row.primary_item().command);
    set_status("Command copied to the clipboard.");
}

fn copy_selected_tsv() {
    let Some(row) = selected_row() else { return };
    let items: Vec<StartupItem> = row.routes.clone();
    copy_to_clipboard(&msm_ui::view::build_startup_export_tsv(&items));
    set_status("Selected startup data copied as TSV.");
}

fn copy_visible_tsv() {
    let items: Vec<StartupItem> = STATE.with(|s| s.borrow().as_ref().map(|state| {
        let snapshot = state.snapshot.lock().unwrap();
        snapshot.rows.iter().flat_map(|row| row.routes.iter().cloned()).collect()
    }).unwrap_or_default());
    if items.is_empty() { set_status("Nothing visible to copy."); return }
    copy_to_clipboard(&msm_ui::view::build_startup_export_tsv(&items));
    set_status(&format!("{} visible routes copied as TSV.", items.len()));
}

fn copy_all_tsv() {
    let items: Vec<StartupItem> = STATE.with(|s| s.borrow().as_ref().map(|state| state.snapshot.lock().unwrap().items.clone()).unwrap_or_default());
    if items.is_empty() { set_status("Nothing to copy."); return }
    copy_to_clipboard(&msm_ui::view::build_startup_export_tsv(&items));
    set_status(&format!("All {} routes copied as TSV.", items.len()));
}

fn protect_disabled() {
    let targets: Vec<StartupItem> = STATE.with(|s| s.borrow().as_ref().map(|state| {
        let snapshot = state.snapshot.lock().unwrap();
        if let Some(row) = snapshot.rows.get(snapshot.selected) {
            row.routes.clone()
        } else {
            snapshot.items.iter().filter(|item| !item.enabled).cloned().collect()
        }
    }).unwrap_or_default());
    if targets.is_empty() { set_status("Nothing disabled to protect."); return }
    let mut count = 0usize;
    for item in &targets {
        if !item.enabled {
            msm_state::protected_disabled::protect(item);
            count += 1;
        }
    }
    set_status(&format!("Protected {} disabled route{} \u{00B7} repair will never re-enable them.", count, if count == 1 { "" } else { "s" }));
}

fn repair_rules() {
    match msm_actions::reconcile::reconcile_managed_startup_routes(None) {
        Ok(report) => set_status(&format!("Repair finished: {}", report)),
        Err(e) => set_status(&format!("Repair failed: {}", e)),
    }
    request_rescan();
}

fn verify_coverage() {
    let (passed, receipt) = msm_ui::verify::verify_live_inventory();
    set_status(&format!("Coverage {}: {}", if passed { "verified" } else { "gaps found" }, receipt));
}

fn open_startup_folders() {
    unsafe {
        let verb = wstr("open");
        let explorer = wstr("explorer.exe");
        for folder in ["shell:startup", "shell:common startup"] {
            let args = wstr(folder);
            let _ = windows::Win32::UI::Shell::ShellExecuteW(None, windows::core::PCWSTR(verb.as_ptr()),
                windows::core::PCWSTR(explorer.as_ptr()), windows::core::PCWSTR(args.as_ptr()), None,
                windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL);
        }
    }
    set_status("Opened the per-user and common startup folders.");
}

fn delete_selected_managed() {
    let Some(row) = selected_row() else { return };
    let item = row.primary_item().clone();
    if !item.is_managed {
        set_status("Only managed tasks can be permanently deleted here.");
        return;
    }
    let answer = unsafe {
        let message = wstr(&format!("Permanently delete the managed task for {}?\nThis removes the scheduled task entirely.", item.human_name()));
        let caption = wstr("Mich Startup Master");
        windows::Win32::UI::WindowsAndMessaging::MessageBoxW(None, windows::core::PCWSTR(message.as_ptr()),
            windows::core::PCWSTR(caption.as_ptr()),
            windows::Win32::UI::WindowsAndMessaging::MB_YESNO | windows::Win32::UI::WindowsAndMessaging::MB_ICONWARNING)
    };
    if answer.0 != 6 { return } // IDYES
    match msm_actions::managed::delete_managed_task(&item.location) {
        Ok(()) => set_status(&format!("Deleted managed task {}.", item.human_name())),
        Err(e) => set_status(&format!("Delete failed: {}", e)),
    }
    request_rescan();
}

// ---------------------------------------------------------------------------
// Clipboard
// ---------------------------------------------------------------------------
fn copy_to_clipboard(text: &str) {
    use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
    use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
    use windows::Win32::System::Ole::CF_UNICODETEXT;
    unsafe {
        if !OpenClipboard(None).is_ok() { return }
        let _ = EmptyClipboard();
        let mut wide = msm_core::paths::to_wide(text);
        if wide.last() != Some(&0) { wide.push(0) }
        let Ok(handle) = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2) else { let _ = CloseClipboard(); return };
        let target = GlobalLock(handle);
        if !target.is_null() {
            std::ptr::copy_nonoverlapping(wide.as_ptr(), target as *mut u16, wide.len());
            let _ = GlobalUnlock(handle);
        }
        let _ = SetClipboardData(CF_UNICODETEXT.0 as u32, Some(windows::Win32::Foundation::HANDLE(handle.0)));
        let _ = CloseClipboard();
    }
}

// ---------------------------------------------------------------------------
// Tray
// ---------------------------------------------------------------------------
struct TrayIcon {
    hwnd: isize,
    id: u32,
}

static TRAY: OnceLock<Mutex<Option<TrayIcon>>> = OnceLock::new();

fn tray() -> &'static Mutex<Option<TrayIcon>> {
    TRAY.get_or_init(|| Mutex::new(None))
}

fn create_tray_icon(hwnd: windows::Win32::Foundation::HWND) {
    use windows::Win32::UI::Shell::{Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NOTIFYICONDATAW};
    unsafe {
        let instance = windows::Win32::System::LibraryLoader::GetModuleHandleW(None).unwrap_or_default();
        let icon = windows::Win32::UI::WindowsAndMessaging::LoadImageW(Some(instance.into()),
            windows::core::PCWSTR(1usize as *const u16), windows::Win32::UI::WindowsAndMessaging::IMAGE_ICON,
            0, 0, windows::Win32::UI::WindowsAndMessaging::LR_DEFAULTSIZE | windows::Win32::UI::WindowsAndMessaging::LR_SHARED)
            .unwrap_or_default();
        let tip = wstr("Mich Startup Master");
        let mut data = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: 1,
            uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
            uCallbackMessage: WM_APP_TRAY,
            hIcon: windows::Win32::UI::WindowsAndMessaging::HICON(icon.0),
            ..Default::default()
        };
        let tip_len = tip.len().min(128);
        data.szTip[..tip_len].copy_from_slice(&tip[..tip_len]);
        let added = Shell_NotifyIconW(NIM_ADD, &mut data).as_bool();
        *tray().lock().unwrap() = if added { Some(TrayIcon { hwnd: hwnd.0 as isize, id: 1 }) } else { None };
    }
}

fn remove_tray_icon() {
    use windows::Win32::UI::Shell::{Shell_NotifyIconW, NIM_DELETE, NOTIFYICONDATAW};
    if let Some(saved) = tray().lock().unwrap().take() {
        unsafe {
            let mut data = NOTIFYICONDATAW {
                cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
                hWnd: windows::Win32::Foundation::HWND(saved.hwnd as *mut _),
                uID: saved.id,
                ..Default::default()
            };
            let _ = Shell_NotifyIconW(NIM_DELETE, &mut data);
        }
    }
}

fn saved_clone_holder(saved: &TrayIcon) -> Option<TrayIcon> { Some(TrayIcon { hwnd: saved.hwnd, id: saved.id }) }


fn tray_balloon(title: &str, body: &str) {
    use windows::Win32::UI::Shell::{Shell_NotifyIconW, NIF_INFO, NIM_MODIFY, NOTIFYICONDATAW};
    let saved = tray().lock().unwrap().take();
    let Some(saved) = saved else { return };
    *tray().lock().unwrap() = saved_clone_holder(&saved);
    unsafe {
        let mut data = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: windows::Win32::Foundation::HWND(saved.hwnd as *mut _),
            uID: saved.id,
            uFlags: NIF_INFO,
            ..Default::default()
        };
        let title_w = wstr(title);
        let info_w = wstr(body);
        let title_len = title_w.len().min(64);
        let info_len = info_w.len().min(256);
        data.szInfoTitle[..title_len].copy_from_slice(&title_w[..title_len]);
        data.szInfo[..info_len].copy_from_slice(&info_w[..info_len]);
        let _ = Shell_NotifyIconW(NIM_MODIFY, &mut data);
    }
}

unsafe fn on_tray_message(hwnd: windows::Win32::Foundation::HWND, message: u32) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::Foundation::LRESULT;
    use windows::Win32::UI::WindowsAndMessaging::*;
    match message {
        0x0203 => { // WM_LBUTTONDBLCLK — tray double-click opens
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = SetForegroundWindow(hwnd);
            LRESULT(0)
        }
        0x0205 => { // WM_RBUTTONUP — tray menu
            let Ok(menu) = CreatePopupMenu() else { return LRESULT(0) };
            append_menu(menu, "Open", MENU_TRAY_SHOW as usize);
            append_menu(menu, "Refresh now", MENU_TRAY_REFRESH as usize);
            append_menu(menu, "Quit", MENU_TRAY_QUIT as usize);
            let mut point = windows::Win32::Foundation::POINT::default();
            let _ = SetForegroundWindow(hwnd);
            let _ = GetCursorPos(&mut point);
            let _ = TrackPopupMenu(menu, TPM_RIGHTBUTTON, point.x, point.y, None, hwnd, None);
            let _ = DestroyMenu(menu);
            LRESULT(0)
        }
        _ => LRESULT(0),
    }
}

fn tray_command_show() {
    unsafe {
        let hwnd = windows::Win32::Foundation::HWND(MAIN_HWND.load(Ordering::SeqCst) as *mut _);
        if !hwnd.0.is_null() {
            let _ = windows::Win32::UI::WindowsAndMessaging::ShowWindow(hwnd, windows::Win32::UI::WindowsAndMessaging::SW_SHOW);
            let _ = windows::Win32::UI::WindowsAndMessaging::SetForegroundWindow(hwnd);
        }
    }
}

fn tray_command_quit() {
    unsafe {
        let hwnd = windows::Win32::Foundation::HWND(MAIN_HWND.load(Ordering::SeqCst) as *mut _);
        if !hwnd.0.is_null() {
            let _ = windows::Win32::UI::WindowsAndMessaging::DestroyWindow(hwnd);
        }
    }
}
