//! Whether the station's drag and drop works as it is built: a drag
//! begun on the station's thread (an STA) as the button is pressed in a window a multithreaded thread owns (as the
//! panel's thread owns its windows), its input shared meanwhile, dropped on a window of the station's own (as its
//! catchers are). (Registered for the other thread's window, a drop target saw nothing, and the drag never ended.)
//!
//! It moves the pointer and presses its button: run it in the test VM, never on a desktop in use.
//! `glance_lib-*.exe --ignored --exact station::spike::drag_and_drop_across_threads --nocapture`

use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
use windows::Win32::UI::Input::KeyboardAndMouse::{SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MOVE, MOUSEINPUT};
use windows::Win32::UI::WindowsAndMessaging::*;

static PRESSED: AtomicBool = AtomicBool::new(false);
static DRAG_FROM: AtomicIsize = AtomicIsize::new(0);
/// The station's thread, as Windows numbers it (to see from outside what a drag holds).
static STATION_THREAD: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// How a drag under way looks from outside: the window that holds the pointer on the thread dragging (its class),
/// and whether that thread is moving or sizing a window.
fn seen_from_outside(thread: u32) -> String {
    let mut info = GUITHREADINFO { cbSize: std::mem::size_of::<GUITHREADINFO>() as u32, ..Default::default() };
    if unsafe { GetGUIThreadInfo(thread, &mut info) }.is_err() {
        return "no thread info".into();
    }
    let mut class = [0u16; 128];
    let length = unsafe { GetClassNameW(info.hwndCapture, &mut class) } as usize;
    format!("capture {:?} of class {:?}, flags {:#x}", info.hwndCapture.0, String::from_utf16_lossy(&class[..length]), info.flags.0)
}

unsafe extern "system" fn procedure(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if message == WM_LBUTTONDOWN {
        PRESSED.store(true, Ordering::Release);
    }
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

/// Moves the pointer to `at` (screen px), pressing or letting go of the left button as `buttons` says.
fn pointer(at: POINT, buttons: windows::Win32::UI::Input::KeyboardAndMouse::MOUSE_EVENT_FLAGS) {
    let (w, h) = unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
    let input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 { mi: MOUSEINPUT { dx: at.x * 65535 / (w - 1), dy: at.y * 65535 / (h - 1), dwFlags: MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | buttons, ..Default::default() } },
    };
    unsafe { SendInput(&[input], std::mem::size_of::<INPUT>() as i32) };
}

#[test]
#[ignore = "moves the pointer and presses its button: run in the test VM"]
fn drag_and_drop_across_threads() {
    // The windows' thread: multithreaded, as the panel's is; the window dragged from (pressed on).
    let (made, windows) = mpsc::channel();
    std::thread::spawn(move || unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let class = WNDCLASSW { lpfnWndProc: Some(procedure), lpszClassName: w!("GlanceSpike"), hbrBackground: windows::Win32::Graphics::Gdi::HBRUSH(windows::Win32::Graphics::Gdi::GetStockObject(windows::Win32::Graphics::Gdi::WHITE_BRUSH).0), ..Default::default() };
        RegisterClassW(&class);
        let from = CreateWindowExW(WS_EX_TOPMOST, class.lpszClassName, w!("spike"), WS_POPUP | WS_VISIBLE, 200, 200, 200, 200, None, None, None, None).unwrap();
        made.send((from.0 as isize, windows::Win32::System::Threading::GetCurrentThreadId())).unwrap();
        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            DispatchMessageW(&message);
        }
    });
    let (from, pressed_on) = windows.recv().unwrap();
    // The station's thread: an STA with OLE; the window dropped on its own (as the station's catchers are), and the
    // drag from the other thread's window as it is pressed, its input shared meanwhile (see `drag::drag`).
    let (dropped_tx, dropped) = mpsc::channel::<String>();
    let (registered_tx, registered) = mpsc::channel();
    let station = std::thread::spawn(move || unsafe {
        STATION_THREAD.store(windows::Win32::System::Threading::GetCurrentThreadId(), Ordering::Release);
        windows::Win32::System::Ole::OleInitialize(None).unwrap();
        let class = WNDCLASSW { lpfnWndProc: Some(procedure), lpszClassName: w!("GlanceSpikeStation"), hbrBackground: windows::Win32::Graphics::Gdi::HBRUSH(windows::Win32::Graphics::Gdi::GetStockObject(windows::Win32::Graphics::Gdi::GRAY_BRUSH).0), ..Default::default() };
        RegisterClassW(&class);
        let to = CreateWindowExW(WS_EX_TOPMOST, class.lpszClassName, w!("spike station"), WS_POPUP | WS_VISIBLE, 600, 200, 200, 200, None, None, None, None).unwrap();
        let target = super::drop::Target::new(move |dragged| {
            if let super::drop::Dragged::Dropped(given) = dragged {
                let _ = dropped_tx.send(format!("{given:?}"));
            }
        });
        let said = windows::Win32::System::Ole::RegisterDragDrop(to, &target);
        registered_tx.send((said.map_err(|error| error.to_string()), to.0 as isize)).unwrap();
        // Waits for the press in the window dragged from, then drags.
        let began = Instant::now();
        while !PRESSED.load(Ordering::Acquire) && began.elapsed() < Duration::from_secs(10) {
            let mut message = MSG::default();
            while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                DispatchMessageW(&message);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        DRAG_FROM.store(1, Ordering::Release);
        let dragged = super::drag::drag(&[super::clipboard::Content::Text("across threads".into())], false, pressed_on);
        println!("the drag ended: {dragged:?}");
        let _ = windows::Win32::System::Ole::RevokeDragDrop(to);
        dragged
    });
    let (registered, to) = registered.recv().unwrap();
    println!("registered for the station's own window: {registered:?}");
    assert!(registered.is_ok(), "a drop target is registered for the station's own window");
    let middle = |window: isize| {
        let mut rect = RECT::default();
        unsafe { GetWindowRect(HWND(window as *mut _), &mut rect).unwrap() };
        POINT { x: (rect.left + rect.right) / 2, y: (rect.top + rect.bottom) / 2 }
    };
    let (a, b) = (middle(from), middle(to));
    pointer(a, Default::default());
    std::thread::sleep(Duration::from_millis(200));
    pointer(a, MOUSEEVENTF_LEFTDOWN);
    // The drag begun on the station's thread: moved over the other window in steps, and let go there.
    let began = Instant::now();
    while DRAG_FROM.load(Ordering::Acquire) == 0 && began.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(10));
    }
    std::thread::sleep(Duration::from_millis(200));
    for step in 1..=20 {
        pointer(POINT { x: a.x + (b.x - a.x) * step / 20, y: a.y + (b.y - a.y) * step / 20 }, Default::default());
        std::thread::sleep(Duration::from_millis(30));
        if step == 10 {
            println!("the drag seen from outside: {}", seen_from_outside(STATION_THREAD.load(Ordering::Acquire)));
            let foreground = unsafe { GetWindowThreadProcessId(GetForegroundWindow(), None) };
            println!("the foreground thread meanwhile: {}", seen_from_outside(foreground));
        }
    }
    std::thread::sleep(Duration::from_millis(200));
    pointer(b, MOUSEEVENTF_LEFTUP);
    let given = dropped.recv_timeout(Duration::from_secs(5));
    println!("dropped on the station's window: {given:?}");
    let dragged = station.join().unwrap();
    assert!(given.is_ok_and(|given| given.contains("across threads")), "the drop came to the station's thread");
    assert_eq!(dragged, Some(super::drag::Dragged::Taken));
}
