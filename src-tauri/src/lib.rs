mod capture;
mod detector;
mod metrics;
mod panel;
mod settings;

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use serde::Serialize;
use tauri::http::{Response, StatusCode};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{
    App, AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, State, WebviewUrl, WebviewWindowBuilder,
    WindowEvent,
};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use windows::UI::ViewManagement::{UIColorType, UISettings};

use windows::Win32::Graphics::Gdi::{GetSysColor, COLOR_DESKTOP};
use windows::Win32::UI::WindowsAndMessaging::{SystemParametersInfoW, SPI_GETDESKWALLPAPER};

use metrics::{Sampler, StaticInfo};
use panel::{Controller, PageRect, Surface};
use settings::Settings;

/// The settings window's size, and the least it can be resized to (logical px).
const SETTINGS_SIZE: (f64, f64) = (1040.0, 720.0);
const SETTINGS_MIN_SIZE: (f64, f64) = (860.0, 560.0);

/// With the panel hidden, the process list and network adapter are read once
/// in this many samples: often enough to be current when the panel opens.
const HIDDEN_SLOW_TICKS: u64 = 5;

struct AppState {
    info: StaticInfo,
    controller: Arc<Controller>,
    settings: Mutex<Settings>,
    /// The wallpaper, served to the settings window's preview while it is open.
    wallpaper: Mutex<Option<(Arc<Vec<u8>>, &'static str)>>,
    /// The work area the settings window opened on (logical px).
    preview_size: Mutex<(f64, f64)>,
}

/// What the preview draws the panel over: the desktop wallpaper, as the
/// panel mostly sits over it, or the desktop colour when there is no picture.
#[derive(Serialize)]
struct PreviewInfo {
    wallpaper: Option<String>,
    color: String,
    width: f64,
    height: f64,
}

/// The Windows accent colour, in the shades the system pairs with each app theme.
#[derive(Serialize)]
struct Accent {
    on_light: String,
    on_dark: String,
}

#[derive(Serialize)]
struct Bootstrap<'a> {
    info: &'a StaticInfo,
    settings: Settings,
    accent: Accent,
    autostart: bool,
}

fn accent() -> windows::core::Result<Accent> {
    let settings = UISettings::new()?;
    let hex = |shade| settings.GetColorValue(shade).map(|c| format!("#{:02x}{:02x}{:02x}", c.R, c.G, c.B));
    Ok(Accent { on_light: hex(UIColorType::AccentDark1)?, on_dark: hex(UIColorType::AccentLight2)? })
}

// Commands are async so that none of them runs on the main thread: the panel
// controller's lock must never be waited on by the thread that owns the window.

#[tauri::command(async)]
fn bootstrap(app: AppHandle, state: State<AppState>) -> serde_json::Value {
    serde_json::to_value(Bootstrap {
        info: &state.info,
        settings: state.settings.lock().unwrap().clone(),
        accent: accent().expect("system accent colour"),
        autostart: app.autolaunch().is_enabled().unwrap_or(false),
    })
    .unwrap()
}

#[tauri::command(async)]
fn save_settings(app: AppHandle, state: State<AppState>, settings: Settings) {
    state.controller.apply(&settings);
    if let Err(error) = settings.save(&app) {
        eprintln!("could not save settings: {error}");
    }
    *state.settings.lock().unwrap() = settings.clone();
    app.emit_to("panel", "settings-changed", settings).unwrap();
}

#[tauri::command(async)]
fn history(state: State<AppState>) -> Vec<metrics::Sample> {
    state.controller.history()
}

#[tauri::command(async)]
fn preview(state: State<AppState>) -> PreviewInfo {
    let (width, height) = *state.preview_size.lock().unwrap();
    let wallpaper = state.wallpaper.lock().unwrap();
    let rgb = unsafe { GetSysColor(COLOR_DESKTOP) };
    PreviewInfo {
        wallpaper: wallpaper.as_ref().map(|(bytes, _)| format!("http://backdrop.localhost/wallpaper/{}", Arc::as_ptr(bytes) as usize)),
        color: format!("#{:02x}{:02x}{:02x}", rgb & 0xff, (rgb >> 8) & 0xff, (rgb >> 16) & 0xff),
        width,
        height,
    }
}

/// The desktop wallpaper's image file and its media type; `None` for a plain colour.
fn wallpaper() -> Option<(Vec<u8>, &'static str)> {
    let mut path = [0u16; 260];
    unsafe { SystemParametersInfoW(SPI_GETDESKWALLPAPER, path.len() as u32, Some(path.as_mut_ptr().cast()), Default::default()) }
        .ok()?;
    let path = String::from_utf16_lossy(&path[..path.iter().position(|&c| c == 0)?]);
    let extension = std::path::Path::new(&path).extension()?.to_str()?.to_ascii_lowercase();
    let mime = match extension.as_str() {
        "jpg" | "jpeg" | "jfif" => "image/jpeg",
        "png" => "image/png",
        "bmp" => "image/bmp",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => return None,
    };
    Some((std::fs::read(&path).ok()?, mime))
}

#[tauri::command(async)]
fn open_settings(app: AppHandle) {
    show_settings(&app);
}

/// Brings the settings window up, creating it centred on the monitor the
/// pointer is on, with a capture of that desktop for its preview.
/// Brings the settings window up, centred on the monitor the pointer is on.
/// It shows itself when its page has drawn (see `settings_ready`), so it
/// never appears blank. Closing it ends it: keeping it hidden saved about a
/// tenth of a second on the next opening and cost 150 MB meanwhile.
fn show_settings(app: &AppHandle) {
    let state = app.state::<AppState>();
    if let Some(window) = app.get_webview_window("settings") {
        let _ = window.unminimize();
        let _ = window.set_focus();
        return;
    }
    // The settings take over from the panel.
    state.controller.dismiss();
    let Some((work, scale)) = panel::work_area_at_cursor() else { return };
    *state.preview_size.lock().unwrap() =
        ((work.right - work.left) as f64 / scale, (work.bottom - work.top) as f64 / scale);
    *state.wallpaper.lock().unwrap() = wallpaper().map(|(bytes, mime)| (Arc::new(bytes), mime));
    state.controller.set_settings_open(true);
    let window = WebviewWindowBuilder::new(app, "settings", WebviewUrl::App("index.html?settings".into()))
        .title("Glance 设置")
        .visible(false)
        .build()
        .expect("settings window");
    let controller = state.controller.clone();
    let handle = app.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::Destroyed = event {
            controller.set_settings_open(false);
            *handle.state::<AppState>().wallpaper.lock().unwrap() = None;
        }
    });
    // Sized and centred in physical pixels of the monitor it opens on: a
    // logical size would be scaled by whichever monitor the window happens
    // to be created on.
    let px = |logical: f64| (logical * scale).round() as i32;
    let (work_width, work_height) = (work.right - work.left, work.bottom - work.top);
    let width = px(SETTINGS_SIZE.0).min(work_width * 9 / 10);
    let height = px(SETTINGS_SIZE.1).min(work_height * 9 / 10);
    window.set_min_size(Some(PhysicalSize::new(px(SETTINGS_MIN_SIZE.0), px(SETTINGS_MIN_SIZE.1)))).unwrap();
    window
        .set_position(PhysicalPosition::new(work.left + (work_width - width) / 2, work.top + (work_height - height) / 2))
        .unwrap();
    window.set_size(PhysicalSize::new(width, height)).unwrap();
}

#[tauri::command(async)]
fn settings_ready(app: AppHandle) {
    let window = app.get_webview_window("settings").unwrap();
    window.show().unwrap();
    window.set_focus().unwrap();
}

#[tauri::command(async)]
fn set_autostart(app: AppHandle, enabled: bool) -> bool {
    let launcher = app.autolaunch();
    let result = if enabled { launcher.enable() } else { launcher.disable() };
    if let Err(error) = result {
        eprintln!("could not change autostart: {error}");
    }
    launcher.is_enabled().unwrap_or(false)
}

#[tauri::command(async)]
fn set_surface(state: State<AppState>, surface: Surface) {
    state.controller.set_surface(surface);
}

#[tauri::command(async)]
fn set_panel_rect(state: State<AppState>, epoch: u64, rect: PageRect) {
    state.controller.set_panel_rect(epoch, rect);
}

#[tauri::command(async)]
fn relocate(state: State<AppState>, epoch: u64) {
    state.controller.relocate(epoch);
}

#[tauri::command(async)]
fn close_panel(state: State<AppState>) {
    state.controller.close();
}

#[tauri::command(async)]
fn panel_hidden(state: State<AppState>, epoch: u64) {
    state.controller.hidden(epoch);
}

#[tauri::command(async)]
fn quit(app: AppHandle) {
    app.exit(0);
}

fn setup(app: &mut App) -> Result<(), Box<dyn std::error::Error>> {
    let handle = app.handle().clone();
    let window = app.get_webview_window("panel").unwrap();
    let settings = Settings::load(&handle);

    let mut sampler = Sampler::new();
    let controller = Arc::new(Controller::new(handle.clone(), window.hwnd()?, &settings));
    controller.apply(&settings);
    // The panel starts hidden.
    controller.set_on_screen(false);
    app.manage(AppState {
        info: sampler.info.clone(),
        controller: controller.clone(),
        settings: Mutex::new(settings),
        wallpaper: Mutex::new(None),
        preview_size: Mutex::new((0.0, 0.0)),
    });

    thread::spawn({
        let controller = controller.clone();
        move || controller.run()
    });
    thread::spawn({
        let controller = controller.clone();
        let handle = handle.clone();
        move || {
            let mut next = Instant::now();
            for tick in 0u64.. {
                next += handle.state::<AppState>().settings.lock().unwrap().interval();
                thread::sleep(next.saturating_duration_since(Instant::now()));
                let refresh_slow = controller.is_shown() || tick % HIDDEN_SLOW_TICKS == 0;
                if let Some(sample) = sampler.sample(refresh_slow) {
                    controller.record(sample);
                }
            }
        }
    });

    // Left click shows the readings, right click the settings: one interface
    // each, rather than a menu beside them.
    TrayIconBuilder::new()
        .icon(app.default_window_icon().unwrap().clone())
        .tooltip("Glance")
        .on_tray_icon_event(move |tray, event| {
            if let TrayIconEvent::Click { button, button_state: MouseButtonState::Up, .. } = event {
                // Not on this thread: this is the main thread, and placing the
                // panel's window from another thread waits on it while
                // holding the panel's lock, which opening needs; building a
                // window waits on it too.
                if button == MouseButton::Right {
                    let app = tray.app_handle().clone();
                    thread::spawn(move || show_settings(&app));
                } else {
                    let controller = controller.clone();
                    thread::spawn(move || controller.open_from_tray());
                }
            }
        })
        .build(app)?;
    Ok(())
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|_, _, _| {}))
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, None))
        .register_uri_scheme_protocol("backdrop", |context, request| {
            let state = context.app_handle().state::<AppState>();
            let path = request.uri().path().trim_start_matches('/');
            let found = if path.starts_with("wallpaper/") {
                state.wallpaper.lock().unwrap().as_ref().map(|(bytes, mime)| (bytes.clone(), *mime))
            } else {
                path.parse().ok().and_then(|shot| state.controller.snapshot(shot)).map(|bytes| (bytes, "image/bmp"))
            };
            match found {
                Some((bytes, mime)) => Response::builder()
                    .header("Content-Type", mime)
                    .header("Cache-Control", "no-store")
                    // The page reads the pixels (to pick a text tone), which needs CORS.
                    .header("Access-Control-Allow-Origin", "*")
                    .body(bytes.as_ref().clone())
                    .unwrap(),
                None => Response::builder().status(StatusCode::NOT_FOUND).body(Vec::new()).unwrap(),
            }
        })
        .invoke_handler(tauri::generate_handler![
            bootstrap,
            save_settings,
            set_autostart,
            set_surface,
            set_panel_rect,
            close_panel,
            history,
            preview,
            open_settings,
            settings_ready,
            relocate,
            panel_hidden,
            quit
        ])
        .setup(setup)
        .run(tauri::generate_context!())
        .expect("failed to start");
}
