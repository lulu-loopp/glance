// On macOS and Linux the shared parts (readings, layout, the panel's
// content) have no program to use them yet: the ports are under way.
#![cfg_attr(not(windows), allow(dead_code))]

mod detector;
mod frames;
#[cfg(windows)]
mod diagnostics;
#[cfg(windows)]
mod dimm;
#[cfg(windows)]
mod drives;
#[cfg(windows)]
mod elevation;
#[cfg(windows)]
mod gpu_power;
#[cfg(windows)]
mod journal;
#[cfg(windows)]
mod metrics;
#[cfg(windows)]
mod panel;
#[cfg(windows)]
mod pawnio;
#[cfg(windows)]
mod presents;
mod os;
mod reading;
#[cfg(windows)]
mod sensors;
#[cfg(all(windows, feature = "studio"))]
mod studio;
mod settings;
#[cfg(windows)]
mod smbios;
#[cfg(windows)]
mod superio;
#[cfg(windows)]
mod tray;
mod ui;
#[cfg(windows)]
mod update;
#[cfg(windows)]
mod watch;


#[cfg(windows)]
use std::path::PathBuf;
#[cfg(windows)]
use std::sync::{Arc, Mutex, OnceLock};
#[cfg(windows)]
use std::thread;
#[cfg(windows)]
use std::time::Instant;

#[cfg(windows)]
use windows::core::{w, PCWSTR};
#[cfg(windows)]
use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS};
#[cfg(windows)]
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
#[cfg(windows)]
use windows::Win32::System::Threading::{CreateMutexW, OpenMutexW, SYNCHRONIZATION_SYNCHRONIZE};

#[cfg(windows)]
use metrics::Sampler;
#[cfg(windows)]
use reading::StaticInfo;
#[cfg(windows)]
use panel::Controller;
#[cfg(windows)]
use settings::Settings;

/// With the panel hidden, the process list and network adapter are read once
/// in this many samples: often enough to be current when the panel opens.
#[cfg(windows)]
const HIDDEN_SLOW_TICKS: u64 = 5;

/// What every part of Glance shares.
#[cfg(windows)]
pub(crate) struct App {
    pub info: StaticInfo,
    pub controller: Arc<Controller>,
    pub settings: Mutex<Settings>,
    /// Where the settings are kept.
    pub config: PathBuf,
}

#[cfg(windows)]
static APP: OnceLock<App> = OnceLock::new();

#[cfg(windows)]
pub(crate) fn app() -> &'static App {
    APP.get().unwrap()
}

#[cfg(windows)]
impl App {
    /// Takes new settings: the panel follows them at once, and they are kept
    /// for next time.
    pub fn save(&self, settings: Settings) {
        self.controller.apply(&settings);
        if let Err(error) = settings.save(&self.config) {
            journal::note(format!("could not save settings: {error}"));
        }
        *self.settings.lock().unwrap() = settings;
        // The tray reads the shortcut's setting from here: told once it is in.
        tray::follow_settings();
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
#[cfg(windows)]
pub(crate) fn show_settings() {
    app().controller.dismiss();
    ui::settings_window::show();
}

#[cfg(windows)]
pub(crate) fn quit() {
    tray::quit();
}

/// The mutex a running Glance holds, one per sign-in session. A debug build
/// is a Glance of its own (its own settings too, see `settings::config_dir`):
/// it runs beside an installed one without either taking the other's place.
#[cfg(all(windows, not(debug_assertions)))]
const SINGLE_INSTANCE: PCWSTR = w!("Local\\Glance.SingleInstance");
#[cfg(all(windows, debug_assertions))]
const SINGLE_INSTANCE: PCWSTR = w!("Local\\Glance.SingleInstance.Debug");

/// Whether Glance already runs in this session.
#[cfg(windows)]
fn already_running() -> bool {
    match unsafe { OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, false, SINGLE_INSTANCE) } {
        Ok(mutex) => {
            let _ = unsafe { CloseHandle(mutex) };
            true
        }
        // An elevated Glance's mutex is there but out of an ordinary start's reach.
        Err(error) => error.code() == ERROR_ACCESS_DENIED.to_hresult(),
    }
}

#[cfg(windows)]
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
        // And an update's installer, if one was left behind.
        elevation::forget_download();
        return;
    }
    // The installer and uninstaller, elevated, have their own copy of Glance
    // do what touches the install folder. Exit codes: 0 done (or, asked
    // what a folder is, Glance could start unasked there), 1 a folder above
    // can be changed by others, 2 the folder cannot be vouched for, 3 it is
    // a link or a root, 4 the files could not be put there or taken away.
    let args: Vec<String> = std::env::args().collect();
    let code = |kind: elevation::InstallFolder| match kind {
        elevation::InstallFolder::Holds => 0,
        elevation::InstallFolder::Open => 1,
        elevation::InstallFolder::Occupied => 2,
        elevation::InstallFolder::Indirect => 3,
        elevation::InstallFolder::Failed => 4,
    };
    let done = |result: Result<(), elevation::InstallFolder>| std::process::exit(result.map_or_else(code, |_| 0));
    match args.as_slice() {
        [_, flag, folder] if flag == "--check-install-folder" => {
            std::process::exit(code(elevation::install_folder(std::path::Path::new(folder))))
        }
        [_, flag, folder] if flag == "--uninstall-from" => done(elevation::uninstall_from(std::path::Path::new(folder))),
        [_, flag] if flag == "--remove-settings" => std::process::exit(if elevation::remove_settings() { 0 } else { 4 }),
        // The studio films the panel for the video and the README.
        #[cfg(feature = "studio")]
        [_, flag, script, out] if flag == "--studio" => {
            if let Err(error) = studio::run(std::path::Path::new(script), std::path::Path::new(out)) {
                eprintln!("{error}");
                std::process::exit(1);
            }
            std::process::exit(0);
        }
        [_, flag, folder, payload] if flag == "--install" || flag == "--install-anyway" => {
            // "anyway": the user chose an unprotected place all the same.
            let anyway = flag == "--install-anyway";
            done(elevation::install_into(std::path::Path::new(folder), std::path::Path::new(payload), anyway))
        }
        _ => {}
    }
    // One Glance at a time: starting it again opens its settings. Asked
    // before handing over to an elevated start, which the launch task
    // refuses while the Glance it started is running.
    if already_running() {
        tray::ask_for_settings();
        return;
    }
    // The sensors need administrator rights. An ordinary start hands over to
    // an elevated one and leaves; if the user declines, Glance runs without
    // the driver's sensors. A debug build runs as started, so that tools
    // without those rights can drive it.
    if !cfg!(debug_assertions) && !elevation::is_elevated() && elevation::relaunch_elevated() {
        return;
    }
    // Two starts at once: the later one asks the first for its settings.
    let _single = unsafe { CreateMutexW(None, true, SINGLE_INSTANCE) };
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        tray::ask_for_settings();
        return;
    }

    journal::note_crashes();
    let config = settings::config_dir();
    let settings = Settings::load(&config);

    // Elevated (see elevation.rs): keep later starts from asking again, and
    // put the sensor driver in place before the sensors are looked for.
    if elevation::is_elevated() {
        elevation::register_launch_task();
        // The installer of an update taken last time, once it has run.
        elevation::forget_download();
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
    thread::spawn(update::watch);
    // The frames programs present (for the game in front), while Glance runs.
    presents::start();
    thread::spawn(move || {
        let mut watch = watch::Watch::default();
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
                watch.sample(&sample, Instant::now());
                controller.record(sample);
            }
        }
    });
    tray::run();
    presents::stop();
}

#[cfg(target_os = "macos")]
mod mac;

#[cfg(target_os = "macos")]
pub fn run() {
    mac::run()
}

/// Glance does not run on this system yet: the Linux port comes after the
/// macOS one.
#[cfg(not(any(windows, target_os = "macos")))]
pub fn run() {
    eprintln!("Glance runs on Windows for now; the macOS and Linux versions are on their way.");
    std::process::exit(1);
}
