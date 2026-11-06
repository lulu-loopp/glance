// On macOS and Linux the shared parts (readings, layout, the panel's
// content) have no program to use them yet: the ports are under way.
#![cfg_attr(not(windows), allow(dead_code))]

#[cfg(windows)]
mod asus;
#[cfg(windows)]
mod lenovo;
#[cfg(windows)]
mod battery;
mod detector;
#[cfg(windows)]
mod push;
#[cfg(windows)]
mod handoff;
mod frames;
#[cfg(windows)]
mod clipboard;
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
mod hang;
#[cfg(windows)]
mod journal;
#[cfg(windows)]
mod metrics;
#[cfg(windows)]
mod mic;
#[cfg(windows)]
mod mouse;
#[cfg(windows)]
mod overlay;
#[cfg(windows)]
mod panel;
#[cfg(windows)]
mod openrgb;
#[cfg(windows)]
mod pawnio;
#[cfg(windows)]
mod perf;
#[cfg(windows)]
mod phone;
#[cfg(windows)]
mod presents;
mod os;
mod numbered;
mod reading;
#[cfg(windows)]
mod remote;
#[cfg(windows)]
pub mod search;
// (What a touch of the PC said: read the same way by Glance and by its remote, of which this file is a part too.)
#[cfg(windows)]
#[path = "../remote/src/touch.rs"]
mod touch;
#[cfg(windows)]
mod touches;
#[cfg(windows)]
mod screens;
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
mod playing;
#[cfg(windows)]
mod lyrics;
#[cfg(windows)]
mod library;
#[cfg(windows)]
mod sound;
#[cfg(windows)]
mod cover;
#[cfg(windows)]
mod fullscreen;
#[cfg(windows)]
mod serif;
#[cfg(windows)]
mod netease;
#[cfg(all(test, windows))]
mod test_crashes;
// What plays and how loud, read and changed as the phone remote does: its own code for it.
#[cfg(windows)]
#[allow(dead_code)]
#[path = "../remote/src/actions.rs"]
mod actions;
#[cfg(windows)]
#[allow(dead_code)]
#[path = "../remote/src/media.rs"]
mod media;
#[cfg(windows)]
mod link;
#[cfg(windows)]
mod sensing;
#[cfg(windows)]
mod source;
#[cfg(windows)]
mod own_signature;
#[cfg(windows)]
mod update;
#[cfg(windows)]
mod reports;
#[cfg(windows)]
mod station;

mod notes;
mod todo;
#[cfg(windows)]
mod watch;
#[cfg(windows)]
mod carry;
#[cfg(windows)]
mod widget;
#[cfg(windows)]
mod wsl;
#[cfg(windows)]
mod docker;
#[cfg(windows)]
mod latency;


#[cfg(windows)]
use std::path::PathBuf;
#[cfg(windows)]
use std::sync::{Arc, Mutex, OnceLock};
#[cfg(windows)]
use std::thread;
#[cfg(windows)]
use std::time::{Duration, Instant};

#[cfg(windows)]
use windows::core::{w, PCWSTR};
#[cfg(windows)]
use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS};
#[cfg(windows)]
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
#[cfg(windows)]
use windows::Win32::System::Threading::{CreateMutexW, OpenMutexW, SYNCHRONIZATION_SYNCHRONIZE};

#[cfg(windows)]
use reading::StaticInfo;
#[cfg(windows)]
use panel::Controller;
#[cfg(windows)]
use settings::Settings;

/// With the panel hidden, the process list and network adapter are read once
/// in this many samples: often enough to be current when the panel opens.
#[cfg(windows)]
pub(crate) const HIDDEN_SLOW_TICKS: u64 = 5;

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

/// Glance as pictures of its windows need it, off screen: this machine's names, the settings kept, and `history`
/// (made up) as what it has read.
#[cfg(all(test, feature = "studio"))]
pub(crate) fn pictured(info: StaticInfo, history: Vec<reading::Sample>) {
    let config = settings::config_dir();
    let settings = Settings::load(&config);
    let controller = Arc::new(Controller::new(info.clone(), &settings));
    for sample in history {
        controller.record(sample);
    }
    let _ = APP.set(App { info, controller, settings: Mutex::new(settings), config });
}

#[cfg(windows)]
impl App {
    /// Shows the settings as `change` would make them, without keeping them (a slider while it is slid): under
    /// their lock as `change` does, so that a change made elsewhere meanwhile is neither undone nor reaches the panel
    /// out of order.
    pub fn preview(&self, change: impl FnOnce(&mut Settings)) {
        let held = self.settings.lock().unwrap();
        let mut shown = held.clone();
        change(&mut shown);
        self.controller.apply(&shown);
    }

    /// Changes the settings where they are held, under their lock, so that
    /// changes made at once in several places (the panel, the overlay, the
    /// settings window) all stand; keeps them; and only then has the panel,
    /// the settings window and the tray follow, so that they read them as
    /// changed.
    pub fn change(&self, change: impl FnOnce(&mut Settings)) {
        {
            let mut held = self.settings.lock().unwrap();
            change(&mut held);
            // Written to the disk on another thread: whoever changed them goes on (the settings window, every
            // window's thread), and the lock is not held while the disk is.
            save_later(held.clone(), self.config.clone());
            // Still under the lock: two changes at once reach the panel in
            // the order they were made, the later last.
            self.controller.apply(&held);
            // (The station reads them as it is told: see `station::run::Ask::Settings`.)
            station::run::ask(station::run::Ask::Settings);
            station::apps::follow(&held.apps);
            notes::follow_settings(&held);
            station::recent::follow_settings(&held);
        }
        // The tray reads the shortcut's setting from here: told once it is in.
        tray::follow_settings();
    }

    /// The process list was sorted from the panel: kept, as if chosen in the
    /// settings.
    pub fn set_process_sort(&self, sort: ui::prefs::ProcessSort) {
        let known = self.controller.known_modules();
        self.change(|settings| {
            let mut prefs = ui::prefs::Prefs::resolve(&settings.view, &known);
            prefs.processes.sort = sort;
            settings.view = ui::prefs::kept(&prefs, &settings.view);
        });
    }

    /// The overlay turned on or off from the panel.
    pub fn set_overlay(&self, on: bool) {
        self.change(|settings| {
            settings.overlay.on = on;
            settings.overlay.offered = true;
        });
    }

    /// A game, `name`, has started for the first time with the overlay off:
    /// told once that there is one.
    pub fn offer_overlay(&self, name: &str) {
        let mut offer = false;
        self.change(|settings| {
            offer = !settings.overlay.offered && !settings.overlay.on && !settings.overlay.in_game;
            settings.overlay.offered = true;
        });
        if !offer {
            return;
        }
        let lang = ui::text::Lang::resolve(ui::prefs::Prefs::resolve(&self.settings.lock().unwrap().view, &[]).language);
        let (title, text) = match lang {
            ui::text::Lang::Zh => (
                format!("检测到游戏：{name}"),
                "可以在游戏画面上显示帧率等信息：在设置的“悬浮窗”页打开“游戏时自动显示”。".to_string(),
            ),
            ui::text::Lang::En => (
                format!("A game is running: {name}"),
                "Glance can show the frame rate and more over the game: turn on \"Show while playing\" on the settings' Overlay page.".to_string(),
            ),
        };
        tray::notify(&title, &text);
    }

    /// The overlay locked where it is, or let loose.
    pub fn lock_overlay(&self, locked: bool) {
        self.change(|settings| settings.overlay.locked = locked);
    }

    /// The overlay's glass made clearer by `by` (less clear, below zero).
    pub fn clear_overlay(&self, by: f32) {
        self.change(|settings| settings.overlay.clear = (settings.overlay.clear + by).clamp(0.0, 1.0));
    }

    /// The overlay lets clicks through, or takes them.
    pub fn overlay_through(&self, through: bool) {
        self.change(|settings| settings.overlay.click_through = through);
    }

    /// The pinned panel moved away from the edge to `at`, or (none) unpinned.
    pub fn place_panel(&self, at: Option<settings::PanelAt>) {
        self.change(|settings| settings.panel_at = at);
    }

    /// The panel at `edge` of the screen `point` (physical px) is on pinned open there, along it at `point`, or
    /// (not `pinned`) no longer: kept for that edge alone, the other edges' as they are.
    pub fn pin_panel(&self, point: (i32, i32), edge: settings::Edge, pinned: bool) {
        let screen = |(x, y): (i32, i32)| crate::mouse::monitor_at(windows::Win32::Foundation::POINT { x, y }).map(|contact| contact.monitor);
        self.change(|settings| settings::pin(&mut settings.panels_pinned, point, edge, pinned, |a, b| screen(a) == screen(b)));
    }

    /// The overlay was dragged where, and to the size, `placed` says.
    pub fn place_overlay(&self, placed: overlay::Placed) {
        self.change(|settings| {
            settings.overlay.at = placed.at;
            settings.overlay.screen = Some(placed.point);
            settings.overlay.size = placed.size;
        });
    }
}

/// Brings the settings window up, poured out of `place` (what it was asked from on the screen, physical px: see
/// `ui::genie`), the panel giving way to it.
#[cfg(windows)]
pub(crate) fn show_settings(place: Option<windows::Win32::Foundation::RECT>) {
    app().controller.dismiss();
    ui::settings_window::show(place);
}

/// The settings shown at the page of the reports this PC sent, poured out of `place`.
pub(crate) fn show_reports(place: Option<windows::Win32::Foundation::RECT>) {
    app().controller.dismiss();
    ui::settings_window::open_at_reports(place);
}

/// The settings shown at the workbench's page (the station's choices are there), poured out of `place`.
pub(crate) fn show_workbench_settings(place: Option<windows::Win32::Foundation::RECT>) {
    app().controller.dismiss();
    ui::settings_window::open_at_workbench(place);
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
        // The uninstaller, first: every account's tasks removed (through Task Scheduler itself), and every program
        // of Glance's running from the folder ended.
        [_, flag, folder] if flag == "--remove-tasks" => {
            let ended = elevation::remove_all_tasks(std::path::Path::new(folder));
            // The way through the firewall made for the phone remote.
            phone::forget();
            // And an update's installer, if one was left behind.
            elevation::forget_download();
            std::process::exit(if ended { 0 } else { 4 })
        }
        [_, flag, folder] if flag == "--uninstall-from" => done(elevation::uninstall_from(std::path::Path::new(folder))),
        // An update taken elevated for a Glance without its sensors (see `update`).
        [_, flag, ..] if flag == "--update" => std::process::exit(if elevation::is_elevated() { update::run_asked(&args[1..]).unwrap_or(1) } else { 1 }),
        // NetEase's plugin loader put in its folder or taken back, elevated (see `netease`).
        [_, flag, _] if flag == netease::ASKED => std::process::exit(netease::run_asked(&args[1..]).unwrap_or(1)),
        [_, flag, folder] if flag == "--remove-settings" => {
            // (First what the phone remote fetched: by the remote, while its configuration is still there.)
            phone::forget_fetched(std::path::Path::new(folder));
            std::process::exit(if elevation::remove_settings() { 0 } else { 4 })
        }
        // The studio films the panel for the video and the README.
        #[cfg(feature = "studio")]
        [_, flag, script, out] if flag == "--studio" => {
            if let Err(error) = studio::run(std::path::Path::new(script), std::path::Path::new(out)) {
                eprintln!("{error}");
                std::process::exit(1);
            }
            std::process::exit(0);
        }
        // And draws the phone's page of the settings, the ring a colour is made on open.
        #[cfg(feature = "studio")]
        [_, flag, out] if flag == "--phone-page" => {
            if let Err(error) = studio::phone_page(std::path::Path::new(out)) {
                eprintln!("{error}");
                std::process::exit(1);
            }
            std::process::exit(0);
        }
        // And times a widget's frames, part by part, on this PC's own graphics.
        #[cfg(feature = "studio")]
        [_, flag, wallpaper] if flag == "--widget-frames" => {
            if let Err(error) = studio::frames(wallpaper) {
                eprintln!("{error}");
                std::process::exit(1);
            }
            std::process::exit(0);
        }
        // And measures the graphics card's memory a window sized frame by frame takes, each way it may be drawn.
        #[cfg(feature = "studio")]
        [_, flag, way] if flag == "--surface-memory" => {
            if let Err(error) = studio::surface_memory(way) {
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
    // Started elevated with Glance running already: by an earlier version's launch task, which started `glance.exe`
    // (Glance runs it, for the sensors, till their own program makes it again as it is now): the sensors, then, from
    // their own program. Not where the desktop's shell runs elevated itself: everything the user starts is elevated
    // there, and this is the user starting Glance again, which opens its settings (below).
    if elevation::is_elevated() && already_running() && !handoff::shell_elevated() {
        start_sensors_instead();
        return;
    }
    // Started elevated with no Glance running (a task of 0.3.7 or before, a build run by hand): Glance's windows are
    // never shown elevated. The desktop's shell is asked to start Glance as the user, and the sensors are started for
    // it. Only where the shell itself runs elevated (UAC turned off) is there no other way, and Glance runs whole, as
    // it always did; and a start the shell made that is elevated all the same is not handed over again.
    let handed_off = std::env::args().any(|arg| arg == handoff::HANDED_OFF);
    if elevation::is_elevated() && !handed_off {
        let exe = std::env::current_exe().expect("own path");
        let mut asked = handoff::start_as_user(&exe.to_string_lossy(), handoff::HANDED_OFF);
        // At sign-in the shell may not be up yet.
        let until = Instant::now() + Duration::from_secs(30);
        while asked == Err(handoff::Refused::NoShell) && Instant::now() < until {
            thread::sleep(Duration::from_secs(1));
            asked = handoff::start_as_user(&exe.to_string_lossy(), handoff::HANDED_OFF);
        }
        match asked {
            Ok(()) => {
                journal::note("start: started elevated; Glance itself started as the user through the desktop's shell, and its sensors");
                start_sensors_instead();
                return;
            }
            Err(handoff::Refused::ShellElevated) => {}
            Err(why) => {
                journal::note(format!("start: started elevated, and the start could not be handed to the desktop's shell ({why:?}): nothing shown"));
                return;
            }
        }
    }
    // One Glance at a time: starting it again opens its settings.
    if already_running() {
        tray::ask_for_settings();
        return;
    }
    // Two starts at once: the later one asks the first for its settings.
    let _single = unsafe { CreateMutexW(None, true, SINGLE_INSTANCE) };
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        tray::ask_for_settings();
        return;
    }

    journal::note_crashes();
    // (Glance's own run ends waiting for what was noted: see its end.)
    journal::write_apart();
    let config = settings::config_dir();
    update::note_settings_found(settings::saved(&config));
    let mut settings = Settings::load(&config);

    // Started elevated (an earlier version's task, a build run by hand), Glance reads the sensors itself, and does
    // what the sensors' process does as it starts (see elevation.rs): its task kept, the driver in place.
    if elevation::is_elevated() {
        elevation::register_launch_task();
        // The installer of an update taken last time, once it has run.
        elevation::forget_download();
        let exe = std::env::current_exe().expect("own path");
        elevation::ensure_pawnio(&exe.with_file_name("resources").join("PawnIO_setup.exe"));
    }

    // The readings: from the sensors' process, started now if it is not running; else Glance's own.
    let (mut info, mut source) = source::open();
    // Each GPU by the number it was given.
    let new_gpus = settings::slot_gpus(&mut info.gpus, &mut settings.gpu_slots);
    let gpu_slots = settings.gpu_slots.clone();
    let controller = Arc::new(Controller::new(info.clone(), &settings));
    let _ = APP.set(App { info, controller: controller.clone(), settings: Mutex::new(settings), config });
    if new_gpus {
        app().change(|settings| settings.gpu_slots = gpu_slots);
    }

    thread::Builder::new().name("glance: panel".into()).spawn({
        let controller = controller.clone();
        move || controller.run()
    }).expect("a thread started");
    thread::Builder::new().name("glance: updates".into()).spawn(update::watch).expect("a thread started");
    // Its windows watched for a thread that stops answering (see `hang`).
    thread::Builder::new().name("glance: hangs watched".into()).spawn(hang::watch).expect("a thread started");
    // The reports still talked over, asked about each hour (see `reports::follow`).
    thread::Builder::new().name("glance: reports".into()).spawn(reports::follow).expect("a thread started");
    // NetEase's icon row put in where InfLink-rs was installed without it (see `netease::mend`).
    thread::Builder::new().name("glance: netease".into()).spawn(netease::mend).expect("a thread started");
    // The station (its own thread: see `station::run`), the panel woken as what it holds changes.
    station::run::start(|| app().controller.wake());
    station::thumbs::start(|| app().controller.wake());
    station::recent::start(|| app().controller.wake());
    station::apps::follow(&app().settings.lock().unwrap().apps);
    notes::follow_settings(&app().settings.lock().unwrap());
    station::recent::follow_settings(&app().settings.lock().unwrap());
    notes::start(|| app().controller.wake());
    todo::start(|| app().controller.wake());
    // The phone remote, if it is on: running from now, whatever stopped it.
    thread::Builder::new().name("glance: phone remote".into()).spawn(phone::keep).expect("a thread started");
    // What touches the PC while the phone remote has the screens out, heard for it.
    if phone::helper().is_some() {
        touches::start();
    }
    thread::Builder::new().name("glance: samples".into()).spawn(move || {
        // What plays and how loud, while the module that shows them is on and something that can show it is on the
        // screen (the panel, a widget, the settings). The lyrics too, while either shows them.
        playing::watch(|| {
            let seen = app().controller.plays_seen() || ui::settings_window::is_seen();
            let settings = app().settings.lock().unwrap();
            let view = &settings.view;
            // Whatever plays, while a time and media lane shows it (the readings', or the workbench's); music, while an
            // edge or a widget shows it, or a time and media lane its lyrics.
            let time = |lyrics: bool| {
                let shown = |list: &str| ui::prefs::module_on_in(view, list, "time") && (!lyrics || ui::prefs::item_on_in(view, list, "time", "lyrics"));
                shown("modules") || shown("workbench")
            };
            let music = settings.places().shows(settings::Part::Music) || settings.widgets.iter().any(settings::WidgetAt::shows_music) || time(true) || playing::shown_in_settings();
            (time(false), music, seen)
        }, || app().controller.lyric_changed());
        let mut watch = watch::Watch::default();
        // What the remote-control helper is told of each sample.
        let mut told = remote::Told::open();
        loop {
            let want = {
                let settings = app().settings.lock().unwrap();
                // The network's latency measured while its row is on (in this process: the sensors' has nothing to do
                // with it), as often as samples are taken.
                let ping = ui::prefs::module_on(&settings.view, "network") && ui::prefs::item_on_in(&settings.view, "modules", "network", "latency");
                let to = serde_json::from_value::<ui::prefs::NetworkPrefs>(settings.view["network"].clone()).unwrap_or_default().ping_to(update::language_in(&settings.view));
                // (Not `update::language`, which takes the settings held here: this thread, and all that wait on them, stopped.)
                latency::want(ping.then_some(to), settings.interval());
                link::Want {
                    interval_ms: settings.interval().as_millis() as u32,
                    // (What changes slowly is read afresh while something shows it; else now and then.)
                    refresh_slow: controller.is_shown() || controller.widget_processes() || ui::settings_window::is_seen(),
                    wsl: ui::prefs::module_on(&settings.view, "wsl"),
                    docker: ui::prefs::module_on(&settings.view, "docker"),
                }
            };
            if let Some(mut sample) = source.next(want, controller.edges()) {
                sample.ping_ms = latency::latest();
                watch.sample(&sample, Instant::now());
                if let Some(told) = &mut told {
                    told.tell(&sample, &app().info);
                }
                controller.record(sample);
            }
        }
    }).expect("a thread started");
    // Every drive's names found, and kept current, from the start: search is ready as it is called up, and a drive's
    // journal read on while Glance runs does not go past what was kept (that drive then listed again).
    search::start();
    // The desktops the settings' preview shows, read once Glance has settled: the settings open on them.
    thread::Builder::new()
        .name("glance: desktops read for the settings".into())
        .spawn(|| {
            thread::sleep(Duration::from_secs(5));
            ui::settings_window::read_desktops_ahead();
        })
        .expect("a thread started");
    tray::run();
    // (What was changed last is on the disk before Glance goes, a note written in too, and what was noted last.)
    ui::beside::keep_as_glance_goes();
    app().controller.keep_writing();
    // (The drives' names kept as they are now: read again as Glance next starts, not found again.)
    search::keep_all();
    saved_all();
    presents::stop_started();
    journal::written();
    // The other threads end with the process.
    std::process::exit(0);
}

/// `glance.exe` started elevated (see `run`): the sensors' own program started in its place, elevated as it is.
#[cfg(windows)]
fn start_sensors_instead() {
    if !elevation::start_sensors_from_here() {
        journal::note("start: started elevated, and the sensors' program beside it could not be started");
    }
}

/// The settings to be written (the latest changed: those changed before it are not written too), and whether some
/// are being written; on a thread of their own (see `App::change`).
#[cfg(windows)]
static UNSAVED: Mutex<(Option<(settings::Settings, PathBuf)>, bool)> = Mutex::new((None, false));
#[cfg(windows)]
static SAVED: std::sync::Condvar = std::sync::Condvar::new();

/// Writes `settings` to `dir` on the thread that writes them, after those before.
#[cfg(windows)]
fn save_later(settings: settings::Settings, dir: PathBuf) {
    let mut unsaved = UNSAVED.lock().unwrap();
    let idle = unsaved.0.is_none() && !unsaved.1;
    unsaved.0 = Some((settings, dir));
    if !idle {
        return;
    }
    thread::Builder::new()
        .name("glance: settings saved".into())
        .spawn(|| loop {
            let (settings, dir) = {
                let mut unsaved = UNSAVED.lock().unwrap();
                match unsaved.0.take() {
                    Some(next) => {
                        unsaved.1 = true;
                        next
                    }
                    None => {
                        unsaved.1 = false;
                        SAVED.notify_all();
                        return;
                    }
                }
            };
            if let Err(error) = settings.save(&dir) {
                journal::note(format!("could not save settings: {error}"));
            }
        })
        .expect("a thread started");
}

/// Waits till every change is written.
#[cfg(windows)]
fn saved_all() {
    let mut unsaved = UNSAVED.lock().unwrap();
    while unsaved.0.is_some() || unsaved.1 {
        unsaved = SAVED.wait(unsaved).unwrap();
    }
}

/// `glance-sensors.exe`: the sensors, told over the link to Glance running as the user (see `sensing`).
#[cfg(windows)]
pub fn run_sensors() {
    // The one multithreaded apartment, as in `run`.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    sensing::run();
}

/// The sensors have a program of their own on Windows alone.
#[cfg(not(windows))]
pub fn run_sensors() {
    std::process::exit(1);
}

#[cfg(target_os = "macos")]
mod mac;

#[cfg(target_os = "macos")]
pub fn run() {
    mac::run()
}

/// Glance does not run on this system yet: the Linux port comes after the
/// macOS one.
/// For tests: what is pretended shown (the notes, the station's things, the programs found, files lately) is one for
/// all of them, so a test pretending it holds this while it looks at it (tests run side by side: one pretending other
/// notes in the middle of another's made it fail now and then).
#[cfg(test)]
pub fn pretending() -> std::sync::MutexGuard<'static, ()> {
    static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());
    ONE_AT_A_TIME.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn run() {
    eprintln!("Glance runs on Windows for now; the macOS and Linux versions are on their way.");
    std::process::exit(1);
}
