//! Glance's elevated half (`glance-sensors.exe`): reads the sensors that
//! only an administrator may (the board's chip, the processor's own
//! temperatures, the memory's, the drives'), the frames games present, and
//! tells them over the link to the rest of Glance, which runs as the user.
//! It watches the edges that open the panel too (see `push`): it hears the
//! mouse whatever window is in front, which the rest does not.
//! It does nothing else it is asked but what is checked in `link::Asked`,
//! and goes once the rest has gone (and has not come back for a while).

use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::link::{self, Asked, Switch, Told, UpdateState, Want};
use crate::push::{Edges, Pusher};
use crate::metrics::Sampler;

/// How long the rest may be gone (restarting, updating) before the sensors go too.
const WAITED_FOR: Duration = Duration::from_secs(20);
/// How long what came has to say who it is.
const HELLO_WITHIN: Duration = Duration::from_secs(5);
/// How long a reading may take before the sensors are taken to be stuck (a driver, a device, a system call that
/// does not come back): they end, and the rest of Glance starts them again.
pub(crate) const STUCK_AFTER: Duration = Duration::from_secs(30);
/// How long the first look at the hardware may take before they are: it opens every device and asks each what it is,
/// and just after signing in, or an update, it does so on a machine with much else to do. The rest of Glance reads what
/// it can itself while it waits on this (see `source`).
pub const FIRST_LOOK_WITHIN: Duration = Duration::from_secs(120);
/// What is done as the sensors start that the rest of Glance need not wait for, if it takes longer than this, is noted.
const NOTED_AFTER: Duration = Duration::from_secs(5);
/// A part of the first look at the hardware that took less than this is counted in its note, not named: of its 14
/// parts, those so left out take under 2 s together of the 5 s it is noted after. (The GPUs' makers' libraries are no
/// part of it: they are looked up on a thread of their own, which notes its own time, see `gpu_libraries`.)
const PART_NAMED_FROM: Duration = Duration::from_millis(100);

/// Whether this account's tasks are in order yet (see `keep_tasks`).
static TASKS_KEPT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// What an elevated start of Glance has always done, for later starts, on a thread of its own: its tasks kept, and the
/// installer of an update taken last time gone. Nothing that serves this start waits for either. Whether Glance starts
/// at sign-in is told once it is done: till then its task may be under the name an earlier version gave it.
fn keep_tasks() {
    let began = Instant::now();
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
    }
    crate::elevation::register_launch_task();
    crate::elevation::forget_download();
    if began.elapsed() >= NOTED_AFTER {
        crate::journal::note(format!("sensors: their tasks took {} s to keep", began.elapsed().as_secs()));
    }
    TASKS_KEPT.store(true, std::sync::atomic::Ordering::Release);
    tell_autostart();
}

/// What is noted of a first look at the hardware that took `took`: the parts of it (`parts`, in the order they ran) that
/// took `PART_NAMED_FROM` or more, each by how long, and how many others took less.
fn first_look_noted(took: Duration, parts: &[(&str, Duration)]) -> String {
    let mut told: Vec<String> = parts.iter().filter(|(_, part)| *part >= PART_NAMED_FROM).map(|(name, part)| format!("{name} {} ms", part.as_millis())).collect();
    let others = parts.len() - told.len();
    if others > 0 {
        told.push(format!("{others} more under {} ms", PART_NAMED_FROM.as_millis()));
    }
    format!("sensors: the first look at the hardware took {} s ({})", took.as_secs(), told.join(", "))
}

/// Tells the rest, if it is there and this account's tasks are in order, whether Glance starts at sign-in. (Said after
/// `PUSHES_TO` is set as it comes, and after `TASKS_KEPT` is set here: it is told at least once, whichever is first.)
fn tell_autostart() {
    if !TASKS_KEPT.load(std::sync::atomic::Ordering::Acquire) {
        return;
    }
    let telling = PUSHES_TO.lock().unwrap().clone();
    if let Some(telling) = telling {
        let _ = link::tell(&mut telling.lock().unwrap(), &Told::Switched { switch: Switch::Autostart, on: crate::elevation::autostart_enabled() });
    }
}

/// Runs the sensors till the rest has gone for good. Not elevated, or with the sensors already running for
/// this user, it does nothing.
pub fn run() {
    if !crate::elevation::is_elevated() {
        return;
    }
    // Its lines go to the rest of Glance, which writes the log (two processes writing one file lose lines); what could
    // not be said by the time this process ends is left for it (see `journal::leave_held`), a crash's too.
    crate::journal::hold_for_the_app();
    crate::journal::note_crashes();
    let Some(pipes) = link::Sensors::open() else {
        crate::journal::note("sensors: their pipes could not be made (sensors running already, or their pipes still held)");
        crate::journal::leave_held();
        return;
    };
    std::thread::Builder::new().name("glance sensors: tasks kept".into()).spawn(keep_tasks).expect("a thread started");
    // The driver in place (its installer not waited for: see `ensure_pawnio`).
    if let Ok(exe) = std::env::current_exe() {
        crate::elevation::ensure_pawnio(&exe.with_file_name("resources").join("PawnIO_setup.exe"));
    }
    // Watched from now: the first look at the hardware as each reading is, with the time it has.
    std::thread::Builder::new().name("glance sensors: readings watched".into()).spawn(watch_readings).expect("a thread started");
    let began = Instant::now();
    *READING_SINCE.lock().unwrap() = Some((awake(), FIRST_LOOK_WITHIN));
    let (mut sampler, parts) = Sampler::timed();
    *READING_SINCE.lock().unwrap() = None;
    if began.elapsed() >= NOTED_AFTER {
        crate::journal::note(first_look_noted(began.elapsed(), &parts));
    }
    crate::presents::start();
    crate::openrgb::watch();
    std::thread::Builder::new().name("glance sensors: edges".into()).spawn(watch_edges).expect("a thread started");
    let want = Arc::new(Mutex::new(Want { interval_ms: 1000, refresh_slow: true, wsl: false, docker: false }));
    let pipes = Arc::new(pipes);
    loop {
        // The rest, come (or back): waited for no longer than it may be gone.
        let (sender, came) = mpsc::channel();
        std::thread::spawn({
            let pipes = pipes.clone();
            move || {
                let _ = sender.send(pipes.accept());
            }
        });
        let Ok(Some((told, asked))) = came.recv_timeout(WAITED_FOR) else { break };
        // What came says who it is first, within a few seconds (what says nothing is let go: it would keep the
        // rest out); it is checked (through the pipe) before anything it says is heeded, its version too, or it is
        // told anything.
        let first = link::receive_within::<Asked>(&asked, HELLO_WITHIN);
        let checked = match first {
            Ok(Asked::Hello { version }) => pipes.check_client().map(|pid| (pid, version)),
            Ok(_) => Err("it said something else first".into()),
            Err(link::Unheard::InTime) => Err(format!("it said nothing in {} s", HELLO_WITHIN.as_secs())),
            Err(link::Unheard::Broke) => Err("it went before saying anything".into()),
        };
        let pid = match checked {
            Ok((pid, version)) if version == env!("CARGO_PKG_VERSION") => pid,
            // Another version's Glance: it starts the sensors of its own version once these have gone.
            Ok((_, version)) => {
                crate::journal::note(format!("sensors: Glance {version} came, these are {}: going, for its own", env!("CARGO_PKG_VERSION")));
                break;
            }
            Err(why) => {
                crate::journal::note(format!("sensors: what came is not taken ({why})"));
                drop((told, asked));
                pipes.let_go();
                continue;
            }
        };
        // (The rest's windows are no game's.)
        crate::presents::set_partner(pid);
        let telling = Arc::new(Mutex::new(told));
        let tell = |told: &Told| link::tell(&mut telling.lock().unwrap(), told);
        let hello = Told::Hello { version: env!("CARGO_PKG_VERSION").into(), info: sampler.info.clone(), found: sampler.info.found.clone() };
        let greeted = tell(&hello)
            && tell(&Told::Buttons { down: crate::mouse::buttons_held(), pressed: false })
            && tell(&Told::Switched { switch: Switch::Remote, on: crate::phone::on() })
            && tell(&Told::Switched { switch: Switch::RaisedLights, on: crate::openrgb::on() });
        // What it is asked, heard on a thread of its own; a report is said by the reading loop, the rest done here.
        let (report_wanted, report_asked) = mpsc::channel::<()>();
        let listener = std::thread::Builder::new().name("glance sensors: asked".into()).spawn({
            let want = want.clone();
            let telling = telling.clone();
            let mut asked = asked.try_clone().ok();
            move || {
                let tell = |told: &Told| link::tell(&mut telling.lock().unwrap(), told);
                while let Some(asking) = asked.as_mut().and_then(link::heard) {
                    match asking {
                        // (Said once, first; heard again, it is nothing.)
                        Asked::Hello { .. } => {}
                        Asked::Want(wanted) => *want.lock().unwrap() = wanted.checked(),
                        Asked::GameMode(mode) => crate::presents::set_game_mode(mode.into()),
                        Asked::Report => {
                            let _ = report_wanted.send(());
                        }
                        Asked::Switch { switch: Switch::Autostart, on } => {
                            tell(&Told::Switched { switch: Switch::Autostart, on: crate::elevation::set_autostart(on) });
                        }
                        Asked::Switch { switch: Switch::Remote, on } => {
                            tell(&Told::Switched { switch: Switch::Remote, on: crate::phone::turn(on) });
                        }
                        Asked::Switch { switch: Switch::RaisedLights, on } => {
                            tell(&Told::Switched { switch: Switch::RaisedLights, on: crate::openrgb::turn(on) });
                        }
                        // On a thread of its own (a download may take a while): what else is asked meanwhile is done.
                        Asked::Install(release) => {
                            tell(&Told::Updating(UpdateState::Downloading));
                            let telling = telling.clone();
                            std::thread::spawn(move || {
                                let state = crate::update::fetch_and_run_sent(&release);
                                link::tell(&mut telling.lock().unwrap(), &Told::Updating(state));
                            });
                        }
                        Asked::Edges(edges) => *WATCHED.lock().unwrap() = edges.checked(),
                        Asked::Activity(on) => HEARING_ACTIVITY.store(on, std::sync::atomic::Ordering::Relaxed),
                    }
                }
            }
        }).expect("a thread started");
        // The pushes seen from now on, told to it.
        *PUSHES_TO.lock().unwrap() = Some(telling.clone());
        tell_autostart();
        let mut next = Instant::now();
        let mut tick = 0u64;
        while greeted && !listener.is_finished() {
            // Till the next reading is due, a report asked for meanwhile is said at once; the listener gone (the
            // rest of Glance gone with it), the reading ends.
            let now = Instant::now();
            if now < next {
                match report_asked.recv_timeout(next - now) {
                    Ok(()) if !tell(&Told::Report(crate::diagnostics::sensor_lines())) => break,
                    Ok(()) | Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
            let wanted = *want.lock().unwrap();
            let interval = Duration::from_millis(wanted.interval_ms as u64);
            // (Fallen behind by more than a reading, it goes on from now.)
            next = if next + interval < now { now + interval } else { next + interval };
            tick += 1;
            let refresh_slow = wanted.refresh_slow || tick % crate::HIDDEN_SLOW_TICKS == 0;
            if !crate::journal::say_held(|line| tell(&Told::Note(line))) {
                break;
            }
            *READING_SINCE.lock().unwrap() = Some((awake(), STUCK_AFTER));
            let sample = sampler.sample(refresh_slow, wanted.wsl, wanted.docker, interval);
            *READING_SINCE.lock().unwrap() = None;
            if let Some(sample) = sample {
                if !tell(&Told::Sample(Box::new(sample))) || !tell(&Told::Presenting { games: crate::presents::presenting(), mode: crate::presents::game_mode().into() }) {
                    break;
                }
            }
            if let Some(found) = sampler.found_anew() {
                if !tell(&Told::Found(found)) {
                    break;
                }
            }
        }
        // Gone: the pipes let go (which ends the listener's reading), for the rest to come again; no edge watched
        // till it says again which.
        *PUSHES_TO.lock().unwrap() = None;
        *WATCHED.lock().unwrap() = Edges::none();
        HEARING_ACTIVITY.store(false, std::sync::atomic::Ordering::Relaxed);
        drop(telling);
        drop(asked);
        pipes.let_go();
        // (Its read, which letting the pipes go does not end, given up on: it ends, and is waited for.)
        unsafe {
            use std::os::windows::io::AsRawHandle;
            let _ = windows::Win32::System::IO::CancelSynchronousIo(windows::Win32::Foundation::HANDLE(listener.as_raw_handle()));
        }
        let _ = listener.join();
    }
    crate::presents::stop();
    crate::journal::leave_held();
}

/// When the reading under way began, in time awake (see `awake`), and how long it may take (none: none is).
static READING_SINCE: Mutex<Option<(Duration, Duration)>> = Mutex::new(None);

/// How long the system has run awake: the time it slept left out (a reading under way as it went to sleep has not
/// been taking all that while; counted by the clock, every one was taken for stuck as the PC woke, the sensors
/// ended, and its CPU's readings lost).
pub(crate) fn awake() -> Duration {
    let mut unbiased = 0u64;
    let _ = unsafe { windows::Win32::System::WindowsProgramming::QueryUnbiasedInterruptTime(&mut unbiased) };
    Duration::from_nanos(unbiased * 100)
}

/// Whether a reading that began as `reading` says (in time awake) has been under way too long by `now`.
fn stuck(reading: Option<(Duration, Duration)>, now: Duration) -> bool {
    reading.is_some_and(|(since, within)| now.saturating_sub(since) > within)
}

/// Ends the sensors once a reading has been under way for longer than it may be, the rest of Glance told why (on the
/// pipe the reading does not hold meanwhile, or, there being none, by what is left for it to find): stuck, they serve
/// nothing, and the rest starts them again.
fn watch_readings() {
    loop {
        std::thread::sleep(STUCK_AFTER / 6);
        let reading = *READING_SINCE.lock().unwrap();
        if stuck(reading, awake()) {
            let why = format!("sensors: starting or a reading has taken over {} s; ending, to be started again", reading.map_or(0, |(_, within)| within.as_secs()));
            let telling = PUSHES_TO.try_lock().ok().and_then(|pushes| pushes.clone());
            // On a thread of its own, waited for a few seconds at most (a pipe the rest has stopped reading holds
            // whoever writes to it, and a disk as busy as this may hold what writes to it): the end comes either way.
            let (told, done) = mpsc::channel();
            std::thread::spawn(move || {
                let said = telling.is_some_and(|telling| telling.try_lock().is_ok_and(|mut telling| link::tell(&mut telling, &Told::Note(why.clone()))));
                if !said {
                    crate::journal::note(why);
                }
                crate::journal::leave_held();
                let _ = told.send(());
            });
            let _ = done.recv_timeout(Duration::from_secs(3));
            std::process::exit(1);
        }
    }
}

/// The edges watched, as the rest last said; and where each push seen is told (none: the rest is not there).
static WATCHED: Mutex<Edges> = Mutex::new(Edges::none());
static PUSHES_TO: Mutex<Option<Arc<Mutex<std::fs::File>>>> = Mutex::new(None);
/// Whether the rest wants to hear what touches the PC (see `Asked::Activity`).
static HEARING_ACTIVITY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// What is heard of what touches the PC is told at most this often (a mouse reports a thousand times a second), and
/// at once when its kind changes (a touch after a report that said nothing, or the other way).
const ACTIVITY_EVERY: Duration = Duration::from_millis(50);

/// How often the pointer is looked at (for one that moves without the mouse's motion being heard: see
/// `Pusher::looked`), and how often again while it rests on an edge.
const LOOK_MS: u32 = 100;
const RESTING_MS: u32 = 16;
const LOOK_TIMER: usize = 1;
const RESTING_TIMER: usize = 2;

/// Watches the edges that open the panel, on a thread of its own: the mouse's own motion (raw input, which reaches
/// an elevated process whatever window is in front) and the pointer's place; each push seen told to the rest. And
/// the mouse's buttons, told as they change; and, while the rest asks, what touches the PC (every kind of device the
/// phone remote wakes the screens for), all heard by the one window (a process hears a kind of device at one
/// window only).
fn watch_edges() {
    use windows::core::w;
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::SystemInformation::GetTickCount;
    use windows::Win32::UI::Input::{RegisterRawInputDevices, HRAWINPUT, RAWINPUTDEVICE, RIDEV_INPUTSINK};
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, KillTimer, RegisterClassW, SetTimer, MSG, WINDOW_STYLE, WM_DISPLAYCHANGE, WM_INPUT, WM_TIMER, WNDCLASSW, WS_EX_TOOLWINDOW,
    };

    // A window never shown, not a message-only one: it hears that the screens changed, as those do not.
    unsafe extern "system" fn procedure(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if message == WM_DISPLAYCHANGE {
            crate::screens::changed();
        }
        unsafe { DefWindowProcW(window, message, wparam, lparam) }
    }
    // (Where it is told taken first: what writes is not held by the list of where, which others look at.)
    let say = |told: &Told| {
        let telling = PUSHES_TO.lock().unwrap().clone();
        if let Some(telling) = telling {
            let _ = link::tell(&mut telling.lock().unwrap(), told);
        }
    };
    let tell = |pushed: Option<crate::push::Pushed>| {
        let Some((at, _, edge)) = pushed else { return };
        say(&Told::Pushed { edge, at: (at.x, at.y) });
    };
    // Whether a button was down as last said, and which are, as the raw reports said (see `held_after`; one down as
    // this began, of no known button).
    let mut down = crate::mouse::buttons_held();
    let mut held: u8 = if down { 1 } else { 0 };
    // Whether the system had the buttons otherwise than the reports at the last look (see the timer, below).
    let mut disagreed = false;
    // What touches the PC, while it is heard: what was heard, and what kind was told when.
    let mut activity: Option<(crate::touch::Heard, Option<(bool, Instant)>)> = None;
    let tell_activity = |heard: &crate::touch::Heard| {
        let named = |heard: &Option<(String, u32)>| heard.as_ref().map(|(device, at)| (device.chars().take(link::DEVICE_MOST).collect(), *at));
        say(&Told::Activity { touched: named(&heard.touched), nothing: named(&heard.nothing) });
    };
    unsafe {
        let Ok(module) = GetModuleHandleW(None) else { return };
        let class = WNDCLASSW { lpfnWndProc: Some(procedure), hInstance: module.into(), lpszClassName: w!("GlanceEdges"), ..Default::default() };
        RegisterClassW(&class);
        let Ok(window) = CreateWindowExW(WS_EX_TOOLWINDOW, class.lpszClassName, w!(""), WINDOW_STYLE(0), 0, 0, 0, 0, None, None, Some(module.into()), None) else { return };
        let mouse = RAWINPUTDEVICE { usUsagePage: 1, usUsage: 2, dwFlags: RIDEV_INPUTSINK, hwndTarget: window };
        if RegisterRawInputDevices(&[mouse], size_of::<RAWINPUTDEVICE>() as u32).is_err() {
            return;
        }
        SetTimer(Some(window), LOOK_TIMER, LOOK_MS, None);
        let mut pusher = Pusher::default();
        let mut resting = false;
        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            let now = Instant::now();
            match message.message {
                WM_INPUT => {
                    if let Some((moved, buttons)) = crate::mouse::read_mouse(HRAWINPUT(message.lParam.0 as *mut _)) {
                        if let Some(cursor) = crate::mouse::cursor_position() {
                            tell(pusher.heard(&WATCHED.lock().unwrap(), moved, cursor, now));
                        }
                        // The buttons: a press as the device reports it, whether one is down as its reports have it.
                        // (Read from the system as the report came, a button let go still read down: the system had
                        // not taken its going up yet, and it stayed down till the mouse next moved; meanwhile each
                        // panel opened closed at once, as at a press elsewhere.)
                        let pressed = buttons & crate::mouse::ANY_BUTTON_DOWN != 0;
                        held = crate::mouse::held_after(buttons, held);
                        let now_down = held != 0;
                        if pressed || now_down != down {
                            down = now_down;
                            say(&Told::Buttons { down, pressed });
                        }
                    }
                    if let Some((heard, last)) = &mut activity {
                        if let Some(said) = heard.take(message.lParam, GetTickCount()) {
                            if last.is_none_or(|(kind, at)| kind != said || now.duration_since(at) >= ACTIVITY_EVERY) {
                                tell_activity(heard);
                                *last = Some((said, now));
                            }
                        }
                    }
                }
                WM_TIMER if message.wParam.0 == LOOK_TIMER => {
                    // What touches the PC heard from now on, or no longer, as the rest asks; each time anew (what
                    // was heard the last time is not of this one).
                    let wanted = HEARING_ACTIVITY.load(std::sync::atomic::Ordering::Relaxed);
                    // (The mouse is heard here throughout.)
                    if wanted != activity.is_some() && crate::touch::hear_but_the_mouse(window, wanted).is_ok() {
                        activity = wanted.then(|| {
                            let heard = crate::touch::Heard::default();
                            tell_activity(&heard);
                            (heard, None)
                        });
                    }
                    if let Some(cursor) = crate::mouse::cursor_position() {
                        pusher.looked(&WATCHED.lock().unwrap(), cursor, now);
                    }
                    // The buttons as the system has them by now (this process reads them truly), where the reports
                    // missed one going up or down (one let go on the secure desktop, say): taken once the system
                    // has had them so at two looks running (at one, it may not have taken what was just reported).
                    let now_down = crate::mouse::buttons_held();
                    if now_down != down && disagreed {
                        held = if now_down { held.max(1) } else { 0 };
                        down = now_down;
                        say(&Told::Buttons { down, pressed: false });
                    }
                    disagreed = now_down != down;
                }
                WM_TIMER if message.wParam.0 == RESTING_TIMER => {
                    if let Some(cursor) = crate::mouse::cursor_position() {
                        tell(pusher.rested(&WATCHED.lock().unwrap(), cursor, now));
                    }
                }
                _ => {}
            }
            if pusher.resting() != resting {
                resting = pusher.resting();
                if resting {
                    SetTimer(Some(window), RESTING_TIMER, RESTING_MS, None);
                } else {
                    let _ = KillTimer(Some(window), RESTING_TIMER);
                }
            }
            DispatchMessageW(&message);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A slow first look is noted with where its time went: the parts that took a while named in the order they ran,
    /// in ms, and the others only counted.
    #[test]
    fn first_look_noted_by_its_parts() {
        let ms = Duration::from_millis;
        let parts = [("counters", ms(312)), ("processes", ms(40)), ("GPUs", ms(5120)), ("GPU libraries", ms(99)), ("CPU sensors", ms(100)), ("microphone", ms(3))];
        assert_eq!(
            first_look_noted(ms(8045), &parts),
            "sensors: the first look at the hardware took 8 s (counters 312 ms, GPUs 5120 ms, CPU sensors 100 ms, 3 more under 100 ms)"
        );
        // Every part named, nothing is said of others.
        assert_eq!(first_look_noted(ms(5400), &parts[2..3]), "sensors: the first look at the hardware took 5 s (GPUs 5120 ms)");
    }

    /// A reading is stuck by the time it has taken awake: the clock counted on through a sleep (the clock as
    /// `Instant` keeps it, on Windows) is not what it took.
    #[test]
    fn stuck_by_time_awake() {
        let began = awake();
        assert!(!stuck(Some((began, STUCK_AFTER)), awake()));
        assert!(!stuck(None, began + Duration::from_secs(3600)));
        assert!(!stuck(Some((began, STUCK_AFTER)), began + STUCK_AFTER));
        assert!(stuck(Some((began, STUCK_AFTER)), began + STUCK_AFTER + Duration::from_secs(1)));
        // The first look has longer: a machine busy as it starts is not one that is stuck.
        assert!(!stuck(Some((began, FIRST_LOOK_WITHIN)), began + STUCK_AFTER + Duration::from_secs(1)));
        assert!(stuck(Some((began, FIRST_LOOK_WITHIN)), began + FIRST_LOOK_WITHIN + Duration::from_secs(1)));
        // The time awake goes on as the clock does while awake, by much the same.
        let (clock, from) = (Instant::now(), awake());
        std::thread::sleep(Duration::from_millis(200));
        let (by_clock, by_awake) = (clock.elapsed(), awake() - from);
        assert!(by_awake.abs_diff(by_clock) < Duration::from_millis(50), "{by_clock:?} {by_awake:?}");
    }
}
