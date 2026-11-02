//! Where Glance's readings come from: the sensors' own process, elevated,
//! over the link (see `sensing`); or, where there is none to be had (the
//! user did not let Glance run as administrator), a sampler of Glance's own,
//! reading what needs no rights. Glance itself runs as the user either way,
//! but where it was started elevated (an older task, a build run by hand),
//! when it reads the sensors itself, as it always did.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::link::{Asked, GameModeSent, Link, Switch, Told, Want};
use crate::metrics::Sampler;
use crate::presents::GameMode;
use crate::push::Edges;
use crate::reading::{Sample, StaticInfo};

/// How long the sensors are waited for as they are started (an elevated start Windows asks the user about may
/// take a while to be answered).
const STARTED_WITHIN: Duration = Duration::from_secs(60);
/// How long the sensors of another version are waited for to go, as they hear this one (they go at once).
const GONE_WITHIN: Duration = Duration::from_secs(10);
/// How long they are waited for as they are started again through their task, the link lost (no one is asked).
const STARTED_AGAIN_WITHIN: Duration = Duration::from_secs(15);
/// How long a link must have lasted for its sensors, gone, to be started again: those that fail again and again
/// as they start are not started without end.
const LASTED_TO_START_AGAIN: Duration = Duration::from_secs(60);

/// What the readings come from.
pub enum Source {
    /// (Its link let go of, for a moment, as the sensors are started again: see `next`.)
    Linked(Option<Link>),
    Own { sampler: Sampler, next: Instant, tick: u64 },
}

/// The link's asking end, for what is asked of the sensors from any thread; whether there is a link.
/// What the sensors are asked goes to them through this, written to their pipe on a thread of the link's own
/// (see `linked_to`): asked from the threads that draw, a pipe the sensors have stopped reading would hold them.
static ASKING: Mutex<Option<std::sync::mpsc::Sender<Vec<u8>>>> = Mutex::new(None);
static LINKED: AtomicBool = AtomicBool::new(false);
/// Since when the link there is has been there.
static LINKED_SINCE: Mutex<Option<Instant>> = Mutex::new(None);
/// How many links there have been (one asked something before the latest knows nothing of it).
static LINKS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// What the sensors last said for a report, as it came.
static REPORTED: Mutex<Option<Vec<String>>> = Mutex::new(None);
/// What was last asked of them (asked again only when it changes): the readings, and the edges they watch.
static WANTED: Mutex<Option<Want>> = Mutex::new(None);
static EDGES: Mutex<Option<Edges>> = Mutex::new(None);
/// How the switches that need their rights are, as they last told (none: not told yet), and the turn of each
/// told (for one waiting on an answer).
static SWITCHES: Mutex<[(Option<bool>, u64); 3]> = Mutex::new([(None, 0), (None, 0), (None, 0)]);

fn slot(switch: Switch) -> usize {
    match switch {
        Switch::Autostart => 0,
        Switch::Remote => 1,
        Switch::RaisedLights => 2,
    }
}

/// Whether Glance starts at sign-in: as the sensors' process says (their task was made elevated), else as read
/// here.
pub fn autostart() -> bool {
    if linked() {
        if let Some(on) = SWITCHES.lock().unwrap()[0].0 {
            return on;
        }
    }
    crate::elevation::autostart_enabled()
}

/// Turns a switch that needs the sensors' rights: by their process (its answer waited for, a while), or here.
/// Whether it is on then.
pub fn switch(switch: Switch, on: bool) -> bool {
    if !linked() {
        return match switch {
            Switch::Autostart => crate::elevation::set_autostart(on),
            Switch::Remote => crate::phone::turn(on),
            Switch::RaisedLights => crate::elevation::is_elevated() && crate::openrgb::turn(on),
        };
    }
    let turn = SWITCHES.lock().unwrap()[slot(switch)].1;
    ask(&Asked::Switch { switch, on });
    let until = Instant::now() + Duration::from_secs(20);
    while Instant::now() < until {
        let (now, told) = SWITCHES.lock().unwrap()[slot(switch)];
        if told != turn {
            return now.unwrap_or(false);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    SWITCHES.lock().unwrap()[slot(switch)].0.unwrap_or(false)
}

/// Asks the sensors' process to download and run the release found newer.
pub fn install(release: crate::link::ReleaseSent) {
    ask(&Asked::Install(release));
}

/// The readings' source, and what the machine is. Elevated already, Glance reads the sensors itself; else it
/// links to the sensors running, or starts them (through its task, or by asking the user) and links to them;
/// failing that, it reads what it can itself.
pub fn open() -> (StaticInfo, Source) {
    if !crate::elevation::is_elevated() {
        // The sensors running, and of this version; else (none running, or another version's, which goes as it
        // hears this one) this version's started.
        let mut linked = match Link::connect(Duration::from_millis(300)) {
            Some(running) => match greeted(&running) {
                Some(info) => Some((running, info)),
                // Another version's, going as it hears this one: its pipes free once it has gone.
                None => {
                    if !running.gone_within(GONE_WITHIN) {
                        crate::journal::note("readings: the sensors of another version had not gone in time");
                    }
                    None
                }
            },
            None => None,
        };
        if linked.is_none() {
            linked = crate::elevation::start_sensors().then(|| Link::connect(STARTED_WITHIN)).flatten().and_then(|link| greeted(&link).map(|info| (link, info)));
        }
        if let Some((link, info)) = linked {
            linked_to(&link);
            crate::journal::note("readings: from the sensors' process");
            return (info, Source::Linked(Some(link)));
        }
        crate::journal::note("readings: Glance's own, without administrator rights (the sensors' process was not had)");
    }
    let sampler = Sampler::new();
    crate::presents::start();
    (sampler.info.clone(), Source::Own { sampler, next: Instant::now(), tick: 0 })
}

/// Where each of the GPUs Glance started with (`App::info`) is among those the readings now come with, by the
/// index of each in theirs (none: not among them); none while they are the same, in the same order. (A sensors'
/// process started again, or Glance reading itself, finds the GPUs again: docked, undocked, a driver changed, they
/// may be listed otherwise, and each was shown the readings of another.)
static GPUS_AS: Mutex<Option<Vec<Option<usize>>>> = Mutex::new(None);

/// The readings come with the GPUs of `info` from now.
fn gpus_found(info: &StaticInfo) {
    let first = &crate::app().info.gpus;
    let same = first.len() == info.gpus.len() && first.iter().zip(&info.gpus).all(|(a, b)| a.identity == b.identity);
    let order = (!same).then(|| first.iter().map(|gpu| info.gpus.iter().position(|now| now.identity == gpu.identity)).collect());
    if !same {
        crate::journal::note("readings: the GPUs are listed otherwise than as Glance started; each read where it is now (one new is shown once Glance starts again)");
    }
    *GPUS_AS.lock().unwrap() = order;
}

/// `sample` with its GPUs in the order Glance started with (see `GPUS_AS`).
fn as_first(mut sample: Sample) -> Sample {
    let Some(order) = GPUS_AS.lock().unwrap().clone() else { return sample };
    let read = std::mem::take(&mut sample.gpus);
    sample.gpus = order.iter().map(|at| at.and_then(|at| read.get(at).cloned()).unwrap_or_default()).collect();
    if let Some(game) = sample.game.as_mut() {
        game.gpu_index = game.gpu_index.and_then(|now| order.iter().position(|at| *at == Some(now)));
    }
    sample
}

/// `link`, if the sensors at its other end said hello, and are of this version (another's speaks another tongue): with
/// what the machine is.
fn greeted(link: &Link) -> Option<StaticInfo> {
    // (They may be starting: installing their driver, looking at the hardware first; not waited for for ever.)
    match crate::link::receive_within::<Told>(&link.told, STARTED_WITHIN) {
        Some(Told::Hello { version, info, .. }) if version == env!("CARGO_PKG_VERSION") => Some(info),
        Some(Told::Hello { version, .. }) => {
            crate::journal::note(format!("readings: the sensors running are of Glance {version}, not {}", env!("CARGO_PKG_VERSION")));
            None
        }
        _ => None,
    }
}

/// From now on what is asked goes over `link`; nothing asked of the sensors before is taken to be known to them.
fn linked_to(link: &Link) {
    if let Ok(mut asking) = link.asked.try_clone() {
        // (Its thread ends as the link is replaced or lost: what sends to it is dropped then.)
        let (ask, asked) = std::sync::mpsc::channel::<Vec<u8>>();
        std::thread::Builder::new()
            .name("glance: asking the sensors".into())
            .spawn(move || {
                use std::io::Write;
                for framed in asked {
                    if asking.write_all(&framed).is_err() {
                        break;
                    }
                }
            })
            .expect("a thread started");
        *ASKING.lock().unwrap() = Some(ask);
    }
    *WANTED.lock().unwrap() = None;
    *EDGES.lock().unwrap() = None;
    // A sensors' process started again knows nothing of what the last was asked: the game mode chosen is told it
    // again (else it said its own, Auto, and the user's choice went with it); a download the last had under way
    // went with it, and is no longer waited for.
    ask(&Asked::GameMode(GameModeSent::from(crate::presents::game_mode())));
    crate::update::link_lost();
    *LINKED_SINCE.lock().unwrap() = Some(Instant::now());
    LINKS.fetch_add(1, Ordering::Relaxed);
    LINKED.store(true, Ordering::Relaxed);
}

/// The link there is now, by its count (see `LINKS`); none, there is none.
pub fn link() -> Option<u64> {
    linked().then(|| LINKS.load(Ordering::Relaxed))
}

/// Asks the sensors' process to hear what touches the PC, or no longer.
pub fn hear_activity(on: bool) {
    ask(&Asked::Activity(on));
}

/// Whether the readings come from the sensors' process.
pub fn linked() -> bool {
    LINKED.load(Ordering::Relaxed)
}

fn ask(asked: &Asked) {
    if let (Some(asking), Some(framed)) = (ASKING.lock().unwrap().as_ref(), crate::link::framed(asked)) {
        let _ = asking.send(framed);
    }
}

/// Whether a game shows, as the user sets it: here, and in the sensors' process, which finds the games.
pub fn set_game_mode(mode: GameMode) {
    crate::presents::set_game_mode(mode);
    ask(&Asked::GameMode(GameModeSent::from(mode)));
}

/// What the sensors have to say for a report: asked of their process (waited for a moment), or said here.
pub fn sensor_lines() -> Vec<String> {
    if !linked() {
        return crate::diagnostics::sensor_lines();
    }
    *REPORTED.lock().unwrap() = None;
    ask(&Asked::Report);
    let until = Instant::now() + Duration::from_secs(3);
    while Instant::now() < until {
        if let Some(lines) = REPORTED.lock().unwrap().take() {
            return lines;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    vec!["Sensors: no answer from their process".into()]
}

impl Source {
    /// The next sample, as `want` asks for them: waited for (the sensors' process sends each as it reads it; Glance's
    /// own sampler reads at its time). None when there was none this time (a sample that could not be taken). The
    /// link lost, the sensors are started again through their task (if it had lasted: see `LASTED_TO_START_AGAIN`);
    /// failing that, Glance reads what it can itself from then on, and watches the edges itself. The sensors'
    /// process watches `edges` meanwhile, and tells each push it sees.
    pub fn next(&mut self, want: Want, edges: Edges) -> Option<Sample> {
        match self {
            Source::Linked(held) => {
                let link = held.as_mut().expect("a link");
                if *WANTED.lock().unwrap() != Some(want) {
                    *WANTED.lock().unwrap() = Some(want);
                    ask(&Asked::Want(want));
                }
                let mut told = EDGES.lock().unwrap();
                if told.as_ref() != Some(&edges) {
                    ask(&Asked::Edges(edges.clone()));
                    *told = Some(edges);
                }
                drop(told);
                loop {
                    match link.receive() {
                        Some(Told::Sample(sample)) => return Some(as_first(*sample)),
                        Some(Told::Presenting { games, mode }) => crate::presents::mirror(games, mode.into()),
                        Some(Told::Report(lines)) => *REPORTED.lock().unwrap() = Some(lines),
                        Some(Told::Pushed { edge, at }) => crate::app().controller.pushed(edge, at),
                        Some(Told::Switched { switch, on }) => {
                            let mut switches = SWITCHES.lock().unwrap();
                            let kept = &mut switches[slot(switch)];
                            *kept = (Some(on), kept.1 + 1);
                        }
                        Some(Told::Updating(state)) => crate::update::set_state(state),
                        Some(Told::Note(line)) => crate::journal::note(line),
                        Some(Told::Buttons { down, pressed }) => crate::panel::heard_buttons(down, pressed),
                        Some(Told::Activity { touched, nothing }) => crate::touches::heard(touched, nothing),
                        Some(Told::Hello { .. }) => {}
                        None => {
                            LINKED.store(false, Ordering::Relaxed);
                            *ASKING.lock().unwrap() = None;
                            let lasted = LINKED_SINCE.lock().unwrap().take().is_some_and(|since| since.elapsed() >= LASTED_TO_START_AGAIN);
                            // The link let go of, and the process it was to gone, before they are started again: a pipe
                            // lasts while either end holds it, and the new process, finding its pipes still there, could
                            // not make them its own, and ended.
                            let old = held.take().expect("a link");
                            if !old.gone_within(GONE_WITHIN) {
                                crate::journal::note("readings: the sensors' process that went had not ended in time");
                            }
                            if lasted && crate::elevation::start_sensors_unasked() {
                                if let Some((again, info)) = Link::connect(STARTED_AGAIN_WITHIN).and_then(|link| greeted(&link).map(|info| (link, info))) {
                                    crate::journal::note("readings: the sensors' process went, and was started again");
                                    linked_to(&again);
                                    gpus_found(&info);
                                    *held = Some(again);
                                    return None;
                                }
                            }
                            crate::journal::note("readings: the sensors' process went; Glance reads what it can itself");
                            // What it said last is not so any more: the games it saw, an update it was taking.
                            crate::presents::forget_mirrored();
                            crate::update::link_lost();
                            crate::app().controller.hear_own();
                            let sampler = Sampler::new();
                            gpus_found(&sampler.info);
                            crate::presents::start();
                            *self = Source::Own { sampler, next: Instant::now(), tick: 0 };
                            return None;
                        }
                    }
                }
            }
            Source::Own { sampler, next, tick } => {
                let interval = Duration::from_millis(want.interval_ms as u64);
                *next += interval;
                // Fallen far behind (the machine slept): carried on from now rather than sampled again and again.
                let now = Instant::now();
                if *next + interval < now {
                    *next = now;
                }
                std::thread::sleep(next.saturating_duration_since(now));
                *tick += 1;
                sampler.sample(want.refresh_slow || *tick % crate::HIDDEN_SLOW_TICKS == 0, want.wsl, want.docker, interval).map(as_first)
            }
        }
    }
}
