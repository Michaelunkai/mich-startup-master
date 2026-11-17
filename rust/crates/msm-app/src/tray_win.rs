//! Resident Win32 tray icon on its own thread + message loop. The egui window
//! owns no Win32 plumbing; the tray talks to it through channels and a cloned
//! egui context (repaint + viewport commands).

use egui::Context;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};

pub enum TrayEvent {
    Open,
    Refresh,
    Quit,
}

const WM_APP_TRAY: u32 = 0x8000 + 0x100;
const MENU_OPEN: usize = 3101;
const MENU_REFRESH: usize = 3102;
const MENU_QUIT: usize = 3103;

struct Wide(Vec<u16>);
fn w(text: &str) -> Wide { Wide(msm_core::paths::to_wide(text)) }

/// Start the tray thread. Returns the event receiver the app polls each frame.
pub fn spawn(ctx: Context) -> Receiver<TrayEvent> {
    let (tx, rx) = std::sync::mpsc::channel::<TrayEvent>();
    std::thread::Builder::new().name("msm-tray".into()).spawn(move || {
        unsafe { tray_loop(tx.clone(), ctx); }
    }).ok();
    rx
}

unsafe fn tray_loop(events: Sender<TrayEvent>, ctx: Context) {
    use windows::Win32::UI::WindowsAndMessaging::*;
    use windows::Win32::UI::Shell::{Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW};
    let instance = windows::Win32::System::LibraryLoader::GetModuleHandleW(None).unwrap_or_default();
    let class_name = w("MichStartupMasterTrayWnd");
    let wc = WNDCLASSW {
        style: Default::default(),
        lpfnWndProc: Some(tray_proc),
        hInstance: instance.into(),
        lpszClassName: windows::core::PCWSTR(class_name.0.as_ptr()),
        hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
        hbrBackground: windows::Win32::Graphics::Gdi::HBRUSH::default(),
        ..Default::default()
    };
    if RegisterClassW(&wc) == 0 { return; }
    let title = w("MSM tray");
    let Some(hwnd) = (|| {
        CreateWindowExW(Default::default(), windows::core::PCWSTR(class_name.0.as_ptr()),
            windows::core::PCWSTR(title.0.as_ptr()), WINDOW_STYLE(WS_OVERLAPPED.0),
            0, 0, 0, 0, None, None, Some(instance.into()), None).ok()
    })() else { return };
    let _ = ShowWindow(hwnd, SW_HIDE);
    let icon = LoadImageW(Some(instance.into()), windows::core::PCWSTR(1usize as *const u16), IMAGE_ICON,
        0, 0, LR_DEFAULTSIZE | LR_SHARED).unwrap_or_default();
    let tip = w("Mich Startup Master");
    let mut data = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
        uCallbackMessage: WM_APP_TRAY,
        hIcon: windows::Win32::UI::WindowsAndMessaging::HICON(icon.0),
        ..Default::default()
    };
    let tip_len = tip.0.len().min(128);
    data.szTip[..tip_len].copy_from_slice(&tip.0[..tip_len]);
    let added = Shell_NotifyIconW(NIM_ADD, &mut data).as_bool();
    if !added { return; }

    let mut message = MSG::default();
    loop {
        let result = GetMessageW(&mut message, None, 0, 0);
        if result.0 <= 0 { break; }
        let _ = TranslateMessage(&message);
        DispatchMessageW(&message);
        if message.message == WM_APP_TRAY {
            match (message.lParam.0 as u32) & 0xFFFF {
                0x0203 => { // double-click: open
                    let _ = events.send(TrayEvent::Open);
                    ctx.request_repaint();
                }
                0x0205 => { // right-click: menu
                    let Ok(menu) = CreatePopupMenu() else { continue };
                    for (label, id) in [("Open", MENU_OPEN), ("Refresh now", MENU_REFRESH), ("Quit", MENU_QUIT)] {
                        let wide = w(label);
                        let _ = AppendMenuW(menu, MF_STRING, id, windows::core::PCWSTR(wide.0.as_ptr()));
                    }
                    let _ = SetForegroundWindow(hwnd);
                    let mut point = windows::Win32::Foundation::POINT::default();
                    let _ = GetCursorPos(&mut point);
                    let _ = TrackPopupMenu(menu, TPM_RIGHTBUTTON, point.x, point.y, None, hwnd, None);
                    let _ = DestroyMenu(menu);
                }
                _ => {}
            }
        } else if message.message == WM_COMMAND {
            match (message.wParam.0 & 0xFFFF) as usize {
                MENU_OPEN => { let _ = events.send(TrayEvent::Open); ctx.request_repaint(); }
                MENU_REFRESH => { let _ = events.send(TrayEvent::Refresh); ctx.request_repaint(); }
                MENU_QUIT => { let _ = events.send(TrayEvent::Quit); ctx.request_repaint(); break; }
                _ => {}
            }
        }
    }
    let mut remove = NOTIFYICONDATAW { cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32, hWnd: hwnd, uID: 1, ..Default::default() };
    let _ = Shell_NotifyIconW(NIM_DELETE, &mut remove);
}

unsafe extern "system" fn tray_proc(hwnd: windows::Win32::Foundation::HWND, message: u32, wparam: windows::Win32::Foundation::WPARAM, lparam: windows::Win32::Foundation::LPARAM) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::UI::WindowsAndMessaging::{DefWindowProcW, WM_DESTROY, WM_CLOSE, PostQuitMessage};
    match message {
        WM_CLOSE | WM_DESTROY => { PostQuitMessage(0); windows::Win32::Foundation::LRESULT(0) }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

/// Drain the tray channel; true when the app should exit.
pub fn poll(events: &Receiver<TrayEvent>, mut on_event: impl FnMut(TrayEvent)) -> bool {
    loop {
        match events.try_recv() {
            Ok(event) => {
                if matches!(event, TrayEvent::Quit) {
                    return true;
                }
                on_event(event);
            }
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => return false,
        }
    }
}
