//! Nearby transfer as the helper runs it: its receiver on its port, finding devices, the link to Glance, and what
//! Glance asks (sending, and announcing this PC); and what the helper's configuration keeps of it, as Glance's
//! settings ask (`glance-remote nearby ...`).

use std::collections::HashMap;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde_json::json;

use super::discover::Discovery;
use super::link::{Command, Link};
use super::{send, tls, Added, Adding, Event, Info, Kept, Receiver, Told};
use crate::config::Config;

/// How often a receiving gone quiet is looked for.
const EXPIRE_EVERY: Duration = Duration::from_secs(10);
/// How often the devices added by their addresses are told of this PC (they keep it found as long again as they would
/// one that announced itself, `discover::FRESH`, and it learns whether they are there).
const REACH_ADDED_EVERY: Duration = Duration::from_secs(60);

/// The folder this user's helper keeps what comes in (`inbox`) and what came (`receipts`): its own, in the user's
/// local application data (what comes may be large: not to roam with the profile).
pub fn local_folder() -> PathBuf {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_default().join("dev.weiyi.glance.remote").join("nearby")
}

/// What nearby transfer, as it runs, has to hand.
pub struct Running {
    pub receiver: Arc<Receiver>,
    pub discovery: Arc<Discovery>,
    told: Arc<Link>,
    /// The sendings going on, each by its number.
    jobs: Mutex<HashMap<u64, Arc<send::Job>>>,
}

/// The nearby transfer this helper runs, once started.
pub static RUNNING: OnceLock<Arc<Running>> = OnceLock::new();

/// Starts nearby transfer, as this PC called `name` unless its configuration says otherwise (as it says each time: see
/// `Receiver::me`): its receiver on the first of its own ports free (another, where all of them are taken: reached
/// then from the local network alone, as the firewall lets it), finding devices, and Glance told and asked.
pub fn start(name: &str) -> std::io::Result<()> {
    let own = tls::Own::in_folder(&crate::config::folder())?;
    let listener = super::OWN_PORTS.map(listening).find_map(Result::ok).map_or_else(|| listening(0), Ok)?;
    let port = listener.local_addr()?.port();
    let info = Info {
        alias: name.to_string(),
        device_model: Some("Windows".into()),
        device_type: Some("desktop".into()),
        port: Some(port),
        protocol: Some("https".into()),
        ..Info::default()
    };
    // (Glance's commands reach what is made after the link: handed over once it is.)
    let made: Arc<OnceLock<Arc<Running>>> = Arc::new(OnceLock::new());
    let commanded = made.clone();
    let link = Link::start(move |command| {
        if let Some(running) = commanded.get() {
            running.commanded(command);
        }
    });
    let told: Arc<dyn Told> = link.clone();
    let receiver = Arc::new(Receiver::new(own, info, local_folder().join("inbox"), told, || kept_of(&Config::load()), |change| {
        let _ = Config::change(|config| change(&mut config.nearby));
    })?);
    let discovery = Discovery::start(receiver.clone())?;
    let running = Arc::new(Running { receiver: receiver.clone(), discovery: discovery.clone(), told: link, jobs: Mutex::default() });
    let _ = made.set(running.clone());
    let _ = RUNNING.set(running);
    crate::journal::note(format!("nearby: on, as \"{}\" on port {port}, fingerprint {}", receiver.me().alias, receiver.info.fingerprint));
    let serving = receiver.clone();
    std::thread::spawn(move || serving.serve(listener));
    std::thread::spawn(move || loop {
        std::thread::sleep(EXPIRE_EVERY);
        receiver.expire();
        receiver.let_go_quiet();
    });
    discovery.announce();
    let reaching = RUNNING.get().expect("nearby transfer is running").clone();
    std::thread::spawn(move || loop {
        reaching.reach_added();
        std::thread::sleep(REACH_ADDED_EVERY);
    });
    Ok(())
}

/// An address as typed (IPv4 or IPv6, with a port or not: "10.21.140.12", "10.21.140.12:53318", "2001:da8::1",
/// "[2001:da8::1]:53318"): the address and the port given, where it is one.
pub fn parse_address(typed: &str) -> Option<(std::net::IpAddr, Option<u16>)> {
    let typed = typed.trim();
    if let Ok(at) = typed.parse::<SocketAddr>() {
        return (at.port() != 0).then_some((at.ip().to_canonical(), Some(at.port())));
    }
    let bare = typed.strip_prefix('[').and_then(|rest| rest.strip_suffix(']')).unwrap_or(typed);
    bare.parse::<std::net::IpAddr>().ok().map(|ip| (ip.to_canonical(), None))
}

/// The ports a device at an address typed without one is looked for on: a Glance PC's own, then LocalSend's.
pub const PORTS_TRIED: [u16; 2] = [super::OWN_PORT, super::PORT];

/// A listener on `port` (none: one of the system's choosing) for devices over IPv6 and IPv4 alike (an IPv6 socket that
/// takes IPv4 too), where this PC has IPv6; else over IPv4.
fn listening(port: u16) -> std::io::Result<TcpListener> {
    use socket2::{Domain, Socket, Type};
    let both = || -> std::io::Result<TcpListener> {
        let socket = Socket::new(Domain::IPV6, Type::STREAM, None)?;
        socket.set_only_v6(false)?;
        socket.bind(&SocketAddr::from((Ipv6Addr::UNSPECIFIED, port)).into())?;
        socket.listen(128)?;
        Ok(socket.into())
    };
    both().or_else(|error| match error.kind() {
        // (Taken over IPv6 is taken: another port is asked for, as over IPv4.)
        std::io::ErrorKind::AddrInUse => Err(error),
        _ => TcpListener::bind((Ipv4Addr::UNSPECIFIED, port)),
    })
}

impl Running {
    /// Does what Glance asks.
    fn commanded(self: &Arc<Self>, command: Command) {
        match command {
            Command::Announce => {
                self.discovery.announce();
                self.receiver.tell_me();
                self.receiver.tell_devices();
                let reaching = self.clone();
                std::thread::spawn(move || reaching.reach_added());
            }
            Command::Add { job, address } => {
                let going = Arc::new(send::Job::default());
                self.jobs.lock().unwrap().insert(job, going.clone());
                let running = self.clone();
                std::thread::spawn(move || {
                    let outcome = running.receiver.add(&address, &PORTS_TRIED, &going);
                    running.jobs.lock().unwrap().remove(&job);
                    if let Some(outcome) = outcome {
                        running.told.tell(Event::Added { job, outcome });
                    }
                });
            }
            Command::Send { job, to, files } => {
                let Some(device) = self.receiver.devices_now().into_iter().find(|device| device.fingerprint.eq_ignore_ascii_case(&to)) else {
                    self.told.tell(Event::Sending { job, state: send::State::Failed { why: "the device is not nearby".into() } });
                    return;
                };
                let going = Arc::new(send::Job::default());
                self.jobs.lock().unwrap().insert(job, going.clone());
                let running = self.clone();
                std::thread::spawn(move || {
                    let told: Arc<dyn Told> = running.told.clone();
                    send::send(&running.receiver.own, &running.receiver.me(), &device, &files, job, &told, &going);
                    running.jobs.lock().unwrap().remove(&job);
                });
            }
            Command::Cancel { job } => {
                if let Some(going) = self.jobs.lock().unwrap().get(&job) {
                    going.cancel();
                }
            }
            Command::Pin { job, pin } => {
                if let Some(going) = self.jobs.lock().unwrap().get(&job) {
                    going.give_pin(pin);
                }
            }
            Command::Trust { fingerprint } => {
                let running = self.clone();
                std::thread::spawn(move || {
                    let why_not = running.trust(&fingerprint).err();
                    if let Some(why) = &why_not {
                        crate::journal::note(format!("nearby: {fingerprint} not trusted: {why}"));
                    }
                    running.told.tell(Event::Trusted { fingerprint, why_not });
                });
            }
            // (Answers are the link's own.)
            Command::Answer { .. } => {}
        }
    }
}

impl Running {
    /// Each device added by its address told of this PC (`register`), at the address that last answered first, then
    /// at each other it is known at, till one answers: heard as it answers (kept as reached there, see
    /// `Receiver::heard`). One that answers nowhere is not heard: it is not found till it does.
    pub fn reach_added(self: &Arc<Self>) {
        for added in self.receiver.added() {
            let running = self.clone();
            std::thread::spawn(move || {
                let info = Info { fingerprint: added.fingerprint.clone(), protocol: Some(if added.https { "https" } else { "http" }.into()), ..Info::default() };
                for at in &added.addresses {
                    let info = Info { port: Some(at.port()), ..info.clone() };
                    if running.receiver.tell_added(&added.fingerprint, *at, &info) {
                        break;
                    }
                }
            });
        }
    }

    /// The device found nearby by `fingerprint` trusted, once it proved it holds that certificate (see `send::proven`).
    fn trust(&self, fingerprint: &str) -> Result<(), String> {
        let device = self.receiver.devices_now().into_iter().find(|device| device.fingerprint.eq_ignore_ascii_case(fingerprint)).ok_or("the device is not nearby")?;
        let info = send::proven(&self.receiver.own, &device).map_err(|error| error.to_string())?;
        let alias = super::shown_name(&info.alias);
        Config::change(|config| config.nearby.trust_proven(&device.fingerprint, &alias, info.device_type.clone().or(device.kind.clone()), info.glance || device.glance, false)).map_err(|error| error.to_string())?;
        crate::journal::note(format!("nearby: {alias} ({}) trusted, as found", device.address));
        Ok(())
    }
}

/// Devices added by their addresses, as the receiver adds them and tells them of this PC.
impl Receiver {
    /// The device at `typed` (an address, see `parse_address`) added: reached by its address alone (see
    /// `send::reached`, on the port given, else on each of `ports_tried`), told of this PC (it then finds this PC as
    /// one that announced itself), and kept with the addresses it was reached at and tells of, where it was not known
    /// already (found nearby, or added before). Given up meanwhile (`going` cancelled), nothing: nothing is kept and no
    /// device told.
    pub fn add(&self, typed: &str, ports_tried: &[u16], going: &send::Job) -> Option<Adding> {
        let Some((ip, port)) = parse_address(typed) else { return Some(Adding::NotAnAddress) };
        let ports = port.map_or(ports_tried.to_vec(), |port| vec![port]);
        let mut reached = Err(send::Unreached::Nothing);
        for port in ports {
            if going.cancelled() {
                return None;
            }
            reached = send::reached(&self.own, SocketAddr::new(ip, port));
            // (Something answered there: what it is, it is, on any other port too.)
            if reached != Err(send::Unreached::Nothing) {
                break;
            }
        }
        // (Given up meanwhile: nothing kept, nothing told.)
        if going.cancelled() {
            return None;
        }
        let info = match reached {
            Ok(info) => info,
            Err(send::Unreached::Nothing) => return Some(Adding::Unreachable),
            Err(send::Unreached::NotLocalSend) => return Some(Adding::NotLocalSend),
        };
        if info.fingerprint.eq_ignore_ascii_case(&self.info.fingerprint) {
            return Some(Adding::Itself);
        }
        let fingerprint = info.fingerprint.clone();
        let https = info.protocol.as_deref() != Some("http");
        let at = SocketAddr::new(ip, info.port.unwrap_or(super::PORT));
        let known = self.devices_now().iter().any(|device| device.fingerprint.eq_ignore_ascii_case(&fingerprint)) || self.added().iter().any(|added| added.fingerprint.eq_ignore_ascii_case(&fingerprint));
        if !known {
            let alias = super::shown_name(&info.alias);
            let mut added = Added { fingerprint: fingerprint.clone(), alias, model: info.device_model.clone(), kind: info.device_type.clone(), glance: info.glance, https, addresses: Vec::new(), since: super::seconds() };
            added.reached_at(at, &info.addresses);
            self.change_added(&|kept| {
                kept.retain(|was| !was.fingerprint.eq_ignore_ascii_case(&added.fingerprint));
                kept.push(added.clone());
                true
            });
            crate::journal::note(format!("nearby: {} ({at}) added by its address", added.alias));
        }
        // Told of this PC, as it is now and then (`reach_added`): found by it, and heard as it answers.
        self.tell_added(&fingerprint, at, &info);
        Some(Adding::Added { fingerprint, known })
    }

    /// The device of `fingerprint`, at `at`, as `info` says it speaks, told of this PC: heard as it answers (proven,
    /// over HTTPS: pinned to its fingerprint). Whether it answered.
    pub fn tell_added(&self, fingerprint: &str, at: SocketAddr, info: &Info) -> bool {
        match send::register(&self.own, &self.me(), at.ip(), info) {
            Ok(answered) => {
                let known = Info { port: Some(at.port()), protocol: info.protocol.clone(), ..answered };
                let https = info.protocol.as_deref() != Some("http");
                self.heard(&known, at.ip(), https.then_some(fingerprint));
                true
            }
            Err(_) => false,
        }
    }
}

/// What `config` keeps for nearby transfer, as it holds now: called by this PC's name; the devices of phones no longer
/// paired, or whose files are to be asked about (see `Phone::files`), not trusted.
pub fn kept_of(config: &Config) -> Kept {
    let mut kept = config.nearby.clone();
    kept.name = config.name.clone();
    kept.trusted.retain(|trusted| trusted.phone.as_ref().is_none_or(|id| config.phones.iter().any(|phone| &phone.id == id && phone.device.is_some() && phone.files)));
    kept
}

/// The certificate the paired phone `phone` (by its id) presents when it sends, as it told it over what it is paired
/// by, trusted for as long as it is paired, as it calls itself (`alias`): whether `fingerprint` is one (a SHA-256, in
/// hex).
pub fn phone_trusted(phone: &str, fingerprint: &str, alias: &str) -> std::io::Result<bool> {
    if fingerprint.len() != 64 || !fingerprint.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Ok(false);
    }
    let fingerprint = fingerprint.to_ascii_uppercase();
    let alias = super::shown_name(alias);
    Config::change(|config| {
        let trusted = &mut config.nearby.trusted;
        // (One phone, one certificate: what it told before goes. Told again, it is trusted since it first was, and
        // last sent when it did.)
        let now = std::time::SystemTime::now().duration_since(std::time::SystemTime::UNIX_EPOCH).unwrap_or_default().as_secs();
        let (last, since) = trusted.iter().find(|trusted| trusted.phone.as_deref() == Some(phone) && trusted.fingerprint.eq_ignore_ascii_case(&fingerprint)).map_or((0, now), |was| (was.last, was.since));
        trusted.retain(|trusted| trusted.phone.as_deref() != Some(phone) && !trusted.fingerprint.eq_ignore_ascii_case(&fingerprint));
        trusted.push(super::Trusted { fingerprint: fingerprint.clone(), alias: alias.clone(), kind: Some("mobile".into()), last, since, phone: Some(phone.to_string()), glance: true });
    })?;
    Ok(true)
}

/// What a paired phone is told of nearby transfer: whether it is on, where and as whom this PC receives (its
/// certificate's fingerprint, to be pinned), and the most it takes (`Receiver::limits`: one file, all of one sending;
/// and how many files one sending may hold).
pub fn told_to_phones() -> serde_json::Value {
    let running = RUNNING.get();
    let limits = running.map(|running| running.receiver.limits());
    json!({
        "most_file": limits.map(|(file, _)| file),
        "most_sending": limits.and_then(|(_, sending)| sending),
        "most_files": super::MOST_FILES,
        "on": running.is_some() && kept_of(&Config::load()).on,
        "alias": running.map(|running| running.receiver.me().alias),
        "port": running.and_then(|running| running.receiver.info.port),
        "fingerprint": running.map(|running| running.receiver.info.fingerprint.clone()),
    })
}

/// What nearby transfer is, as Glance's settings show it: whether it receives, whether it runs to send, what this PC
/// is called, its fingerprint, its port (where it runs), the devices trusted, whether others are turned away.
pub fn said(config: &Config) -> serde_json::Value {
    let kept = &kept_of(config);
    let fingerprint = tls::Own::in_folder(&crate::config::folder()).map(|own| own.fingerprint).ok();
    json!({
        "on": kept.on,
        "send": kept.send,
        "name": config.name,
        "fingerprint": fingerprint,
        "port": RUNNING.get().and_then(|running| running.receiver.info.port),
        "trusted": kept.trusted,
        "others_refused": kept.others_refused,
        "added": kept.added,
    })
}

/// `glance-remote nearby [on | off | send on|off | name NAME | forget FINGERPRINT | drop FINGERPRINT | others ask|refuse
/// | most FILE SENDING]` (`drop`: a device added by its address no longer kept): what is kept changed as said, then (or with nothing said) said as `said` has it. A name that is none (or
/// nothing at all) names this PC as its computer is.
pub fn asked(words: &[String]) -> std::io::Result<serde_json::Value> {
    let word = |at: usize| words.get(at).map(String::as_str);
    let change = |with: &dyn Fn(&mut Kept)| Config::change(|config| with(&mut config.nearby)).map(|_| ());
    match (word(0), word(1)) {
        (Some("on"), _) => change(&|kept| kept.on = true)?,
        (Some("off"), _) => change(&|kept| kept.on = false)?,
        (Some("send"), Some(how)) => {
            let wanted = how == "on";
            change(&|kept| kept.send = wanted)?
        }
        (Some("name"), name) => {
            let name = name.filter(|name| !name.trim().is_empty()).map(super::shown_name).unwrap_or_else(computer_name);
            Config::change(|config| config.name = name.clone())?;
        }
        (Some("forget"), Some(fingerprint)) => change(&|kept| kept.trusted.retain(|trusted| !trusted.fingerprint.eq_ignore_ascii_case(fingerprint)))?,
        (Some("drop"), Some(fingerprint)) => change(&|kept| kept.added.retain(|added| !added.fingerprint.eq_ignore_ascii_case(fingerprint)))?,
        (Some("others"), Some(how)) => {
            let refused = how == "refuse";
            change(&|kept| kept.others_refused = refused)?
        }
        (Some("most"), Some(file)) => {
            let (file, sending) = (file.parse().ok(), word(2).and_then(|sending| sending.parse().ok()));
            change(&|kept| {
                kept.most_file = file;
                kept.most_sending = sending;
            })?
        }
        _ => {}
    }
    Ok(said(&Config::load()))
}

/// This computer's name, as Windows has it.
pub fn computer_name() -> String {
    std::env::var("COMPUTERNAME").unwrap_or_else(|_| "PC".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Phone;

    /// A device a paired phone told of is trusted while that phone is paired, and no longer; one trusted as it sent
    /// stays trusted.
    #[test]
    fn a_phones_certificate_is_trusted_while_it_is_paired() {
        let trusted = |fingerprint: &str, phone: Option<&str>| super::super::Trusted { fingerprint: fingerprint.into(), phone: phone.map(str::to_string), ..Default::default() };
        let mut config = Config::default();
        config.nearby.trusted = vec![trusted("AA", None), trusted("BB", Some("one")), trusted("CC", Some("two")), trusted("DD", Some("offered"))];
        config.nearby.trusted.push(trusted("EE", Some("asked")));
        config.phones = vec![
            Phone { id: "one".into(), device: Some("phone".into()), ..Default::default() },
            Phone { id: "offered".into(), offered_till: Some(u64::MAX), ..Default::default() },
            // (Paired, but what it sends is asked about: see `Phone::files`.)
            Phone { id: "asked".into(), device: Some("other".into()), files: false, ..Default::default() },
        ];
        let kept: Vec<String> = kept_of(&config).trusted.into_iter().map(|trusted| trusted.fingerprint).collect();
        assert_eq!(kept, ["AA", "BB"]);
    }

    /// An address is read as typed: IPv4 or IPv6, with a port or not, in brackets or not, spaces around it aside; an
    /// IPv4 address written as IPv6 maps it is that IPv4 address; what is no address, or port 0, is none.
    #[test]
    fn an_address_is_read_as_typed() {
        let read = |typed: &str| parse_address(typed).map(|(ip, port)| (ip.to_string(), port));
        assert_eq!(read("10.21.140.12"), Some(("10.21.140.12".into(), None)));
        assert_eq!(read(" 10.21.140.12:53318 "), Some(("10.21.140.12".into(), Some(53318))));
        assert_eq!(read("2001:da8:215:4a07::1c"), Some(("2001:da8:215:4a07::1c".into(), None)));
        assert_eq!(read("[2001:da8::1]"), Some(("2001:da8::1".into(), None)));
        assert_eq!(read("[2001:da8::1]:53318"), Some(("2001:da8::1".into(), Some(53318))));
        assert_eq!(read("::ffff:10.21.140.12"), Some(("10.21.140.12".into(), None)));
        assert_eq!(read("10.21.140"), None);
        assert_eq!(read("10.21.140.12:0"), None);
        assert_eq!(read("实验室电脑"), None);
        assert_eq!(read(""), None);
    }

    /// A device added is kept at the address it last answered at first, then those it told of, then those known
    /// before, each once and no more than eight; whether that changed is said.
    #[test]
    fn a_device_added_keeps_the_address_it_answered_at_first() {
        let at = |text: &str| -> SocketAddr { text.parse().unwrap() };
        let mut added = Added { addresses: vec![at("10.0.0.1:53320"), at("[2001:da8::1]:53320")], ..Added::default() };
        assert!(added.reached_at(at("[2001:da8::1]:53320"), &[at("10.0.0.2:53320"), at("10.0.0.1:53320")]));
        assert_eq!(added.addresses, [at("[2001:da8::1]:53320"), at("10.0.0.2:53320"), at("10.0.0.1:53320")]);
        assert!(!added.reached_at(at("[2001:da8::1]:53320"), &[]));
        let many: Vec<SocketAddr> = (1..=20).map(|n| at(&format!("10.0.1.{n}:53320"))).collect();
        added.reached_at(at("10.0.2.1:53320"), &many);
        assert_eq!(added.addresses.len(), 8);
        assert_eq!(added.addresses[0], at("10.0.2.1:53320"));
    }

    /// The PC's name is the one the helper keeps, nearby too.
    #[test]
    fn nearby_is_called_by_the_pcs_name() {
        let mut config = Config::default();
        config.name = "书房".into();
        assert_eq!(kept_of(&config).name, "书房");
    }
}
