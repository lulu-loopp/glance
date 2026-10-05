mod detector;
mod dimm;
mod drives;
mod elevation;
mod metrics;
mod panel;
mod pawnio;
mod sensors;
mod settings;
mod smbios;
mod superio;
mod tray;
mod ui;

use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::Instant;

use windows::core::w;
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
use windows::Win32::System::Threading::CreateMutexW;

use metrics::{Sampler, StaticInfo};
use panel::Controller;
use settings::Settings;

/// With the panel hidden, the process list and network adapter are read once
/// in this many samples: often enough to be current when the panel opens.
const HIDDEN_SLOW_TICKS: u64 = 5;

/// What every part of Glance shares.
pub(crate) struct App {
    pub info: StaticInfo,
    pub controller: Arc<Controller>,
    pub settings: Mutex<Settings>,
    /// Where the settings are kept.
    pub config: PathBuf,
}

static APP: OnceLock<App> = OnceLock::new();

pub(crate) fn app() -> &'static App {
    APP.get().unwrap()
}

impl App {
    /// Takes new settings: the panel follows them at once, and they are kept
    /// for next time.
    pub fn save(&self, settings: Settings) {
        self.controller.apply(&settings);
        if let Err(error) = settings.save(&self.config) {
            eprintln!("could not save settings: {error}");
        }
        *self.settings.lock().unwrap() = settings;
    }

    /// The process list was sorted from the panel: kept, as if chosen in the
    /// settings.
    pub fn set_process_sort(&self, sort: ui::prefs::ProcessSort) {
        let mut settings = self.settings.lock().unwrap().clone();
        let mut prefs = ui::prefs::Prefs::resolve(&settings.view, &self.controller.known_modules());
        prefs.processes.sort = sort;
        settings.view = serde_json::to_value(prefs).unwrap();
        self.save(settings);
    }
}

/// Brings the settings window up; the panel gives way to it.
pub(crate) fn show_settings() {
    app().controller.dismiss();
    ui::settings_window::show();
}

pub(crate) fn quit() {
    tray::quit();
}

pub fn run() {
    // One multithreaded COM apartment for the life of the process: every
    // thread that uses COM or WinRT (the panel's and the settings' threads)
    // is in it, so objects and the factories cached for them never outlive
    // the apartment they were made in.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    // The uninstaller, elevated, asks the installed copy to remove every
    // account's tasks: through Task Scheduler itself, nothing else loaded.
    if std::env::args().any(|arg| arg == "--remove-tasks") {
        elevation::remove_all_tasks();
        return;
    }
    // The sensors need administrator rights. An ordinary start hands over to
    // an elevated one and leaves; if the user declines, Glance runs without
    // the driver's sensors. A debug build runs as started, so that tools
    // without those rights can drive it.
    if !cfg!(debug_assertions) && !elevation::is_elevated() && elevation::relaunch_elevated() {
        return;
    }
    // One Glance at a time: starting it again opens its settings.
    let _single = unsafe { CreateMutexW(None, true, w!("Local\\Glance.SingleInstance")) };
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        tray::ask_for_settings();
        return;
    }

    let config = settings::config_dir();
    let settings = Settings::load(&config);

    // Elevated (see elevation.rs): keep later starts from asking again, and
    // put the sensor driver in place before the sensors are looked for.
    if elevation::is_elevated() {
        elevation::register_launch_task();
        let exe = std::env::current_exe().expect("own path");
        elevation::ensure_pawnio(&exe.with_file_name("resources").join("PawnIO_setup.exe"));
    }

    let mut sampler = Sampler::new();
    let controller = Arc::new(Controller::new(sampler.info.clone(), &settings));
    let _ = APP.set(App { info: sampler.info.clone(), controller: controller.clone(), settings: Mutex::new(settings), config });

    thread::spawn({
        let controller = controller.clone();
        move || controller.run()
    });
    thread::spawn(move || {
        let mut next = Instant::now();
        for tick in 0u64.. {
            let interval = app().settings.lock().unwrap().interval();
            next += interval;
            // Fallen far behind (the machine slept): carry on from now rather
            // than sampling again and again to catch up.
            let now = Instant::now();
            if next + interval < now {
                next = now;
            }
            thread::sleep(next.saturating_duration_since(now));
            let refresh_slow = controller.is_shown() || ui::settings_window::is_open() || tick % HIDDEN_SLOW_TICKS == 0;
            if let Some(sample) = sampler.sample(refresh_slow) {
                controller.record(sample);
            }
        }
    });
    tray::run();
}
