//! Where Glance's readings come from: the sensors' own process, elevated,
//! over the link (see `sensing`); or, where there is none to be had (the
//! user did not let Glance run as administrator), a sampler of Glance's own,
//! reading what needs no rights. Glance itself runs as the user either way,
//! but where it was started elevated (an older task, a build run by hand),
//! when it reads the sensors itself, as it always did.
//!
//! Reaching the sensors may take any time (a machine just signed in to, or just
//! updated, a prompt the user has not answered yet), and nothing shown waits on it
//! for long: it goes on on a thread of its own (see `reach`), Glance reading what it
//! can itself meanwhile, and the readings come from the sensors from the moment
//! they answer.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::link::{Asked, GameModeSent, Gone, Link, Switch, Told, Unheard, Want};
use crate::metrics::Sampler;
use crate::presents::GameMode;
use crate::push::Edges;
use crate::reading::{Sample, StaticInfo};

/// How long Glance's start waits for the sensors before it goes on, reading what it can itself till they answer: the
/// tray and the panel come after this. (They answer in a second or two on most machines.)
const WAITED_AT_START: Duration = Duration::from_secs(15);
/// How long readings wait as sensors that went are started again, before Glance reads what it can itself meanwhile.
const WAITED_ONCE_LOST: Duration = Duration::from_secs(5);
/// How long sensors already running are given to be found.
const RUNNING_WITHIN: Duration = Duration::from_millis(300);
/// How long their pipes are waited for as they are started.
const STARTED_WITHIN: Duration = Duration::from_secs(60);
/// How long sensors found are waited for to say hello: they do so after their first look at the hardware, and end if
/// that takes longer than it may (their pipe then breaks, and this wait is over at once).
const HELLO_WITHIN: Duration = Duration::from_secs(crate::sensing::FIRST_LOOK_WITHIN.as_secs() + 30);
/// How long the sensors of another version are waited for to go, as they hear this one (they go at once).
const GONE_WITHIN: Duration = Duration::from_secs(10);
/// How long a link must have lasted for all the tries to be had again: sensors that fail again and again as they start
/// are not started without end.
const LASTED_TO_START_AGAIN: Duration = Duration::from_secs(60);
/// How many times the sensors are tried for, and how long is left between one try and the next.
const TRIES: u32 = 4;
const PAUSES: [Duration; TRIES as usize - 1] = [Duration::from_secs(5), Duration::from_secs(30), Duration::from_secs(120)];
/// The tries still to be had (see `tries_once_lost`).
static TRIES_LEFT: AtomicU32 = AtomicU32::new(TRIES);

/// What the readings come from.
pub enum Source {
    /// (Its link let go of, for a moment, as the sensors are started again: see `next`.)
    Linked(Option<Link>),
    /// `reaching`: the sensors on their way, where they are being tried for.
    Own { sampler: Sampler, next: Instant, tick: u64, reaching: Option<Receiver<(Link, StaticInfo)>> },
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
/// What was found of the hardware by what the readings now come from (none: what Glance started with, `App::info`).
static FOUND: Mutex<Option<Vec<String>>> = Mutex::new(None);
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

/// What reaching the sensors takes (the sensors' own, `TheSensors`; a test's, scripted).
trait Reach {
    type Link;
    type Info;
    /// A link to the sensors, waited for up to `patience`, or why there is none.
    fn connect(&mut self, patience: Duration) -> Result<Self::Link, String>;
    /// Whether the sensors at the end of `link` said hello, and are of this version.
    fn greet(&mut self, link: &Self::Link) -> Result<Self::Info, String>;
    /// Lets `link` go, and waits for its sensors to end (their pipes are free only then).
    fn gone(&mut self, link: Self::Link);
    /// Starts them: `ask`ing the user if there is no way without; whether they were started.
    fn start(&mut self, ask: bool) -> bool;
    fn pause(&mut self, pause: Duration);
    fn note(&mut self, line: String);
    /// They were not had, and are no longer tried for.
    fn given_up(&mut self);
}

/// How long is left before a try with `left` tries to go, the first of all going at once. (A try after a link that did
/// not last waits too, whatever began it: those are the sensors that fail as they start.)
fn pause_before(left: u32) -> Option<Duration> {
    PAUSES.get(TRIES.saturating_sub(left).checked_sub(1)? as usize).copied()
}

/// The tries to be had once a link is lost, `left` having been left: all, if it `lasted`.
fn tries_once_lost(lasted: bool, left: u32) -> u32 {
    if lasted {
        TRIES
    } else {
        left
    }
}

/// Reaches the sensors, with the tries `tries` has left: those running, and of this version; else (none running, or
/// another version's, which goes as it hears this one) this version's started, and waited for. A try that ends without
/// them is followed by another, a while after: a start that took longer than it was waited for is found running then,
/// and sensors that ended are started again. The user is asked once at most (`ask`), and never again, whatever came of
/// it. None: they were not had, as the user would not, or there is no way to start them unasked, or the tries ran out.
fn reach<R: Reach>(reach: &mut R, mut ask: bool, tries: &AtomicU32) -> Option<(R::Link, R::Info)> {
    loop {
        let left = tries.load(Ordering::Relaxed);
        if left == 0 {
            reach.note("readings: the sensors' process was tried for as often as it is, and was not had".into());
            return None;
        }
        tries.store(left - 1, Ordering::Relaxed);
        if let Some(pause) = pause_before(left) {
            reach.pause(pause);
        }
        if let Ok(link) = reach.connect(RUNNING_WITHIN) {
            match reach.greet(&link) {
                Ok(info) => return Some((link, info)),
                Err(why) => {
                    reach.note(format!("readings: {why}"));
                    reach.gone(link);
                }
            }
        }
        if !reach.start(ask) {
            reach.note("readings: the sensors' process could not be started".into());
            return None;
        }
        ask = false;
        match reach.connect(STARTED_WITHIN) {
            Ok(link) => match reach.greet(&link) {
                Ok(info) => return Some((link, info)),
                Err(why) => {
                    reach.note(format!("readings: {why}"));
                    reach.gone(link);
                }
            },
            Err(why) => reach.note(format!("readings: the sensors' process was started, and {why}")),
        }
    }
}

/// `reach`, on a thread of its own, `before` (the link last had) let go of first: what it comes to arrives on what is
/// returned, whenever it does; nothing does, and that ends, if it does not come to the sensors.
fn reach_apart<R>(mut reaching: R, before: Option<R::Link>, ask: bool, tries: &'static AtomicU32) -> Receiver<(R::Link, R::Info)>
where
    R: Reach + Send + 'static,
    R::Link: Send + 'static,
    R::Info: Send + 'static,
{
    let (reached, arriving) = mpsc::channel();
    std::thread::Builder::new()
        .name("glance: reaching the sensors".into())
        .spawn(move || {
            // (Their task is run, and the user asked, through COM: see lib.rs.)
            unsafe {
                let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
            }
            if let Some(before) = before {
                reaching.gone(before);
            }
            match reach(&mut reaching, ask, tries) {
                Some(found) => {
                    let _ = reached.send(found);
                }
                None => reaching.given_up(),
            }
        })
        .expect("a thread started");
    arriving
}

/// The sensors themselves, as `reach` goes about them.
struct TheSensors;

impl Reach for TheSensors {
    type Link = Link;
    type Info = StaticInfo;

    fn connect(&mut self, patience: Duration) -> Result<Link, String> {
        Link::connect(patience)
    }

    fn greet(&mut self, link: &Link) -> Result<StaticInfo, String> {
        match crate::link::receive_within::<Told>(&link.told, HELLO_WITHIN) {
            Ok(Told::Hello { version, info, .. }) if version == env!("CARGO_PKG_VERSION") => Ok(info),
            Ok(Told::Hello { version, .. }) => Err(format!("the sensors running are of Glance {version}, not {}", env!("CARGO_PKG_VERSION"))),
            Ok(_) => Err("the sensors said something else before hello".into()),
            Err(Unheard::InTime) => Err(format!("the sensors said nothing in {} s", HELLO_WITHIN.as_secs())),
            Err(Unheard::Broke) => Err("the sensors went before they said hello".into()),
        }
    }

    fn gone(&mut self, link: Link) {
        match link.gone_within(GONE_WITHIN) {
            Gone::Ended(code) => crate::journal::note(format!("readings: the sensors' process ended, with code {code:#x}")),
            Gone::Running => crate::journal::note(format!("readings: the sensors' process let go of had not ended in {} s", GONE_WITHIN.as_secs())),
            Gone::Unknown => crate::journal::note("readings: the sensors' process let go of, and what became of it is not known"),
        }
    }

    fn start(&mut self, ask: bool) -> bool {
        if ask {
            crate::elevation::start_sensors()
        } else {
            crate::elevation::start_sensors_unasked()
        }
    }

    fn pause(&mut self, pause: Duration) {
        std::thread::sleep(pause);
    }

    fn note(&mut self, line: String) {
        crate::journal::note(line);
    }

    fn given_up(&mut self) {
        heard_what_was_left();
        crate::journal::note("readings: Glance's own, without administrator rights (the sensors' process was not had)");
    }
}

/// Notes what sensors that ended had still to say.
fn heard_what_was_left() {
    for line in crate::journal::take_left() {
        crate::journal::note(format!("sensors, as they ended: {line}"));
    }
}

/// Whether Glance's own sampler reads the hardware (see `Sampler::meanwhile`), the sensors' process being on its way
/// (`coming`: started, or being tried for) or not. Not while it is: that process looks at all of it as it starts,
/// and the two are not to go through the same drivers at once (as they did at a sign-in on a laptop). Once it is not
/// to be had (no rights, the user would not, it failed for good), all it can without it, as it always did.
fn reads_the_hardware(coming: bool) -> bool {
    !coming
}

/// Glance's own sampler, the sensors' process being on its way (`coming`) or not (see `reads_the_hardware`).
fn own_sampler(coming: bool) -> Sampler {
    if reads_the_hardware(coming) {
        Sampler::new()
    } else {
        Sampler::meanwhile()
    }
}

/// The readings' source, and what the machine is. Elevated already, Glance reads the sensors itself; else the sensors
/// are reached for (see `reach`), and if they have not answered in a while it reads what it can itself till they do.
/// (What the machine is then lists no drive till they do, or are not to be had: see `Controller::drives_listed`.)
pub fn open() -> (StaticInfo, Source) {
    let mut reaching = None;
    if !crate::elevation::is_elevated() {
        let arriving = reach_apart(TheSensors, None, true, &TRIES_LEFT);
        if let Ok((link, info)) = arriving.recv_timeout(WAITED_AT_START) {
            linked_to(&link, &info);
            crate::journal::note("readings: from the sensors' process");
            return (info, Source::Linked(Some(link)));
        }
        crate::journal::note("readings: Glance's own for now, without administrator rights");
        reaching = Some(arriving);
    }
    let sampler = own_sampler(reaching.is_some());
    crate::presents::start();
    (sampler.info.clone(), Source::Own { sampler, next: Instant::now(), tick: 0, reaching })
}

/// What was found of the hardware by what the readings now come from, where it is not what Glance started with.
pub fn found() -> Option<Vec<String>> {
    FOUND.lock().unwrap().clone()
}

/// Where each of the GPUs Glance started with (`App::info`) is among those the readings now come with, by the
/// index of each in theirs (none: not among them); none while they are the same, in the same order. (A sensors'
/// process started again, or Glance reading itself, finds the GPUs again: docked, undocked, a driver changed, they
/// may be listed otherwise, and each was shown the readings of another.)
static GPUS_AS: Mutex<Option<Vec<Option<usize>>>> = Mutex::new(None);

/// The readings come with the GPUs of `info` from now.
fn gpus_found(info: &StaticInfo) {
    let first = crate::app().info();
    let first = &first.gpus;
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

/// From now on what is asked goes over `link`, to sensors that found `info`; nothing asked of the sensors before is
/// taken to be known to them.
fn linked_to(link: &Link, info: &StaticInfo) {
    *FOUND.lock().unwrap() = Some(info.found.clone());
    heard_what_was_left();
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
    // again (else it said its own, Auto, and the user's choice went with it).
    ask(&Asked::GameMode(GameModeSent::from(crate::presents::game_mode())));
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
    /// own sampler reads at its time). None when there was none this time (a sample that could not be taken, or the
    /// source changed). The link lost, the sensors are started again through their task (see `reach`); till they
    /// answer, if it takes more than a moment, Glance reads what it can itself, and watches the edges itself, as it does
    /// wherever they are not there, and they take over as they answer. The sensors' process watches `edges` meanwhile,
    /// and tells each push it sees.
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
                            TRIES_LEFT.store(tries_once_lost(lasted, TRIES_LEFT.load(Ordering::Relaxed)), Ordering::Relaxed);
                            // The link let go of, and the process it was to gone, before they are started again: a pipe
                            // lasts while either end holds it, and the new process, finding its pipes still there, could
                            // not make them its own, and ended.
                            let arriving = reach_apart(TheSensors, held.take(), false, &TRIES_LEFT);
                            // A download the last had under way went with it: no longer waited for, here or there.
                            if let Ok((again, info)) = arriving.recv_timeout(WAITED_ONCE_LOST) {
                                crate::journal::note("readings: the sensors' process went, and was started again");
                                crate::update::link_lost();
                                linked_to(&again, &info);
                                gpus_found(&info);
                                crate::app().controller.drives_listed(&info);
                                *held = Some(again);
                                return None;
                            }
                            crate::journal::note("readings: the sensors' process went; Glance reads what it can itself meanwhile");
                            // What it said last is not so any more: the games it saw, an update it was taking.
                            crate::presents::forget_mirrored();
                            crate::update::link_lost();
                            crate::app().controller.hearing_changed();
                            // (The drives kept as the sensors listed them: a sampler of the meanwhile lists none.)
                            let sampler = own_sampler(true);
                            gpus_found(&sampler.info);
                            *FOUND.lock().unwrap() = Some(sampler.info.found.clone());
                            crate::presents::start();
                            *self = Source::Own { sampler, next: Instant::now(), tick: 0, reaching: Some(arriving) };
                            return None;
                        }
                    }
                }
            }
            Source::Own { sampler, next, tick, reaching } => {
                // The sensors, once they answer, from then on.
                match reaching.as_ref().map(Receiver::try_recv) {
                    Some(Ok((link, info))) => {
                        crate::journal::note("readings: from the sensors' process, as it answered");
                        // (Their frames from now, not any traced here; an update asked of an elevated Glance here goes on.)
                        crate::presents::stop_started();
                        linked_to(&link, &info);
                        gpus_found(&info);
                        crate::app().controller.drives_listed(&info);
                        crate::app().controller.hearing_changed();
                        *self = Source::Linked(Some(link));
                        return None;
                    }
                    // Not to be had: all Glance can read itself from now (see `reads_the_hardware`).
                    Some(Err(TryRecvError::Disconnected)) => {
                        *reaching = None;
                        *sampler = own_sampler(false);
                        gpus_found(&sampler.info);
                        crate::app().controller.drives_listed(&sampler.info);
                        *FOUND.lock().unwrap() = Some(sampler.info.found.clone());
                    }
                    Some(Err(TryRecvError::Empty)) | None => {}
                }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Arc;

    /// A way to the sensors that goes as it is told, and writes down what was done, with no clock but what it was asked
    /// to wait: how long anything takes on a test machine has no part in it.
    #[derive(Default)]
    struct Scripted {
        /// Whether each connect finds pipes, and each greet hears hello (no more: none, and none).
        connects: VecDeque<bool>,
        greets: VecDeque<bool>,
        /// Whether a start is refused, and from which start on.
        refused_from: Option<usize>,
        started: usize,
        done: Arc<Mutex<Vec<String>>>,
        /// What the first connect waits for.
        held_till: Option<Receiver<()>>,
    }

    impl Scripted {
        fn did(&self, what: impl Into<String>) {
            self.done.lock().unwrap().push(what.into());
        }
    }

    impl Reach for Scripted {
        type Link = u32;
        type Info = &'static str;

        fn connect(&mut self, patience: Duration) -> Result<u32, String> {
            if let Some(held) = self.held_till.take() {
                let _ = held.recv();
            }
            self.did(format!("connect {}", if patience == RUNNING_WITHIN { "running" } else { "started" }));
            if self.connects.pop_front().unwrap_or(false) {
                Ok(1)
            } else {
                Err("not there".into())
            }
        }

        fn greet(&mut self, _: &u32) -> Result<&'static str, String> {
            self.did("greet");
            if self.greets.pop_front().unwrap_or(false) {
                Ok("the machine")
            } else {
                Err("silent".into())
            }
        }

        fn gone(&mut self, _: u32) {
            self.did("gone");
        }

        fn start(&mut self, ask: bool) -> bool {
            self.did(if ask { "start, asking" } else { "start" });
            self.started += 1;
            self.refused_from.is_none_or(|from| self.started < from)
        }

        fn pause(&mut self, pause: Duration) {
            self.did(format!("pause {}", pause.as_secs()));
        }

        fn note(&mut self, _: String) {}

        fn given_up(&mut self) {
            self.did("given up");
        }
    }

    fn scripted(connects: &[bool], greets: &[bool]) -> Scripted {
        Scripted { connects: connects.iter().copied().collect(), greets: greets.iter().copied().collect(), ..Default::default() }
    }

    fn done(script: &Scripted) -> Vec<String> {
        script.done.lock().unwrap().clone()
    }

    /// Those running are linked to, and nothing is started.
    #[test]
    fn sensors_running_are_linked_to() {
        let mut script = scripted(&[true], &[true]);
        assert!(reach(&mut script, true, &AtomicU32::new(TRIES)).is_some());
        assert_eq!(done(&script), ["connect running", "greet"]);
    }

    /// None running: started, asking if it comes to that, and waited for.
    #[test]
    fn none_running_are_started() {
        let mut script = scripted(&[false, true], &[true]);
        assert!(reach(&mut script, true, &AtomicU32::new(TRIES)).is_some());
        assert_eq!(done(&script), ["connect running", "start, asking", "connect started", "greet"]);
    }

    /// Another version's go as they hear this one, and this version's are started once they have.
    #[test]
    fn another_versions_go_first() {
        let mut script = scripted(&[true, true], &[false, true]);
        assert!(reach(&mut script, true, &AtomicU32::new(TRIES)).is_some());
        assert_eq!(done(&script), ["connect running", "greet", "gone", "start, asking", "connect started", "greet"]);
    }

    /// The user would not: Glance neither asks again nor tries otherwise.
    #[test]
    fn the_user_is_not_asked_again() {
        let mut script = Scripted { refused_from: Some(1), ..Default::default() };
        let tries = AtomicU32::new(TRIES);
        assert!(reach(&mut script, true, &tries).is_none());
        assert_eq!(done(&script), ["connect running", "start, asking"]);
    }

    /// A start that takes longer than it was waited for (a machine just updated) is found running at the next try,
    /// and the user is not asked for it a second time.
    #[test]
    fn a_slow_start_is_found_at_the_next_try() {
        let mut script = scripted(&[false, false, true], &[true]);
        assert!(reach(&mut script, true, &AtomicU32::new(TRIES)).is_some());
        assert_eq!(done(&script), ["connect running", "start, asking", "connect started", "pause 5", "connect running", "greet"]);
    }

    /// Sensors that end before they say hello are started again, with longer between, the user never asked again; so
    /// many times, and no more.
    #[test]
    fn sensors_that_keep_ending_are_not_started_without_end() {
        let mut script = scripted(&[false, true, false, true, false, true, false, true], &[]);
        let tries = AtomicU32::new(TRIES);
        assert!(reach(&mut script, true, &tries).is_none());
        let done = done(&script);
        assert_eq!(done.iter().filter(|what| what.starts_with("start")).cloned().collect::<Vec<_>>(), ["start, asking", "start", "start", "start"]);
        assert_eq!(done.iter().filter(|what| what.starts_with("pause")).cloned().collect::<Vec<_>>(), ["pause 5", "pause 30", "pause 120"]);
        assert_eq!(tries.load(Ordering::Relaxed), 0);
        // And, with none left, nothing more is done.
        let mut after = Scripted::default();
        assert!(reach(&mut after, false, &tries).is_none());
        assert!(self::done(&after).is_empty());
    }

    /// A way to start them unasked gone by a later try (their task off, or taken away) ends the trying.
    #[test]
    fn no_way_to_start_them_ends_the_trying() {
        let mut script = scripted(&[false, false, false], &[]);
        script.refused_from = Some(2);
        assert!(reach(&mut script, true, &AtomicU32::new(TRIES)).is_none());
        assert_eq!(done(&script), ["connect running", "start, asking", "connect started", "pause 5", "connect running", "start"]);
    }

    /// A link that lasted brings all the tries back; one that did not leaves as many as there were, spaced apart: what
    /// fails as it starts, however it fails, is not started without end.
    #[test]
    fn tries_come_back_only_after_a_link_that_lasted() {
        assert_eq!(tries_once_lost(true, 0), TRIES);
        assert_eq!(tries_once_lost(false, 2), 2);
        assert_eq!(tries_once_lost(false, 0), 0);
        assert_eq!(pause_before(TRIES), None);
        assert_eq!(pause_before(1), Some(PAUSES[PAUSES.len() - 1]));
        assert_eq!(pause_before(0), None);
    }

    /// While the sensors' process is on its way, Glance's own sampler asks the hardware nothing (that process is
    /// looking at it all). Once the process is not to be had, the sampler reads all it can.
    #[test]
    fn reads_no_hardware_while_the_sensors_are_coming() {
        assert!(!reads_the_hardware(true));
        assert!(reads_the_hardware(false));
    }

    /// What is waited for on Glance's side outlasts what the sensors' side allows itself: they are given up on because
    /// they went, or are stuck, never because each side counted differently.
    #[test]
    fn the_two_sides_agree_on_time() {
        assert!(HELLO_WITHIN > crate::sensing::FIRST_LOOK_WITHIN);
        assert!(WAITED_AT_START < STARTED_WITHIN);
        assert!(WAITED_ONCE_LOST < WAITED_AT_START);
    }

    /// Sensors that answer after the start has stopped waiting still arrive, the thread that reached them having gone
    /// on: Glance is not left without them for as long as it runs.
    #[test]
    fn sensors_that_answer_late_still_arrive() {
        static TRIES_HERE: AtomicU32 = AtomicU32::new(TRIES);
        let (let_go, held_till) = mpsc::channel();
        let mut script = scripted(&[true], &[true]);
        script.held_till = Some(held_till);
        let arriving = reach_apart(script, None, true, &TRIES_HERE);
        // The start stops waiting: nothing yet, and it goes on without.
        assert!(arriving.recv_timeout(Duration::from_millis(20)).is_err());
        assert!(matches!(arriving.try_recv(), Err(TryRecvError::Empty)));
        // They answer.
        let_go.send(()).unwrap();
        let (link, info) = arriving.recv_timeout(Duration::from_secs(30)).expect("they arrive");
        assert_eq!((link, info), (1, "the machine"));
    }

    /// Given up on, nothing arrives and what is waited on ends: the readings go on as they are.
    #[test]
    fn sensors_not_had_end_the_wait() {
        static TRIES_HERE: AtomicU32 = AtomicU32::new(TRIES);
        let script = Scripted { refused_from: Some(1), ..Default::default() };
        let record = script.done.clone();
        let arriving = reach_apart(script, Some(7), true, &TRIES_HERE);
        assert!(arriving.recv_timeout(Duration::from_secs(30)).is_err());
        assert!(matches!(arriving.try_recv(), Err(TryRecvError::Disconnected)));
        // The link before let go of first, then the rest.
        assert_eq!(*record.lock().unwrap(), ["gone", "connect running", "start, asking", "given up"]);
    }
}
