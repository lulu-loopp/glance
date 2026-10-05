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
use tauri::{App, AppHandle, Manager, State};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use windows::UI::ViewManagement::{UIColorType, UISettings};

use metrics::{Sampler, StaticInfo};
use panel::{Controller, PageRect, Surface, View};
use settings::Settings;

/// With the panel hidden, the process list and network adapter are read once
/// in this many samples: often enough to be current when the panel opens.
const HIDDEN_SLOW_TICKS: u64 = 5;

struct AppState {
    info: StaticInfo,
    controller: Arc<Controller>,
    settings: Mutex<Settings>,
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
    *state.settings.lock().unwrap() = settings;
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
fn set_hold(state: State<AppState>, hold: bool) {
    state.controller.set_hold(hold);
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
    app.manage(AppState {
        info: sampler.info.clone(),
        controller: controller.clone(),
        settings: Mutex::new(settings),
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

    // Left click shows the readings, right click the settings; both open the
    // panel itself, so there is one interface rather than a menu beside it.
    TrayIconBuilder::new()
        .icon(app.default_window_icon().unwrap().clone())
        .tooltip("Glance")
        .on_tray_icon_event(move |_, event| {
            if let TrayIconEvent::Click { button, button_state: MouseButtonState::Up, .. } = event {
                let view = if button == MouseButton::Right { View::Settings } else { View::Monitor };
                // Not on this thread: this is the main thread, and placing the
                // window from another thread waits on it while holding the
                // panel's lock, which opening needs.
                let controller = controller.clone();
                thread::spawn(move || controller.open_from_tray(view));
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
            let shot = request.uri().path().trim_start_matches('/').parse().ok();
            match shot.and_then(|shot| state.controller.snapshot(shot)) {
                Some(bytes) => Response::builder()
                    .header("Content-Type", "image/bmp")
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
            set_hold,
            close_panel,
            relocate,
            panel_hidden,
            quit
        ])
        .setup(setup)
        .run(tauri::generate_context!())
        .expect("failed to start");
}
