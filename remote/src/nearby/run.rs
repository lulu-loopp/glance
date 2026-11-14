//! Nearby transfer as the helper runs it: its receiver on its port, finding devices, the link to Glance, and what
//! Glance asks (sending, and announcing this PC); and what the helper's configuration keeps of it, as Glance's
//! settings ask (`glance-remote nearby ...`).

use std::collections::HashMap;
use std::net::{Ipv4Addr, TcpListener};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde_json::json;

use super::discover::Discovery;
use super::link::{Command, Link};
use super::{send, tls, Event, Info, Kept, Receiver, Told};
use crate::config::Config;

/// How often a receiving gone quiet is looked for.
const EXPIRE_EVERY: Duration = Duration::from_secs(10);

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
/// `Receiver::me`): its receiver on its own port (another, where that one is taken), finding devices, and Glance told
/// and asked.
pub fn start(name: &str) -> std::io::Result<()> {
    let own = tls::Own::in_folder(&crate::config::folder())?;
    let listener = TcpListener::bind((Ipv4Addr::UNSPECIFIED, super::OWN_PORT)).or_else(|_| TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0)))?;
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
    });
    discovery.announce();
    Ok(())
}

impl Running {
    /// Does what Glance asks.
    fn commanded(self: &Arc<Self>, command: Command) {
        match command {
            Command::Announce => {
                self.discovery.announce();
                self.told.tell(Event::Devices { devices: self.receiver.devices_now() });
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
            // (Answers are the link's own.)
            Command::Answer { .. } => {}
        }
    }
}

/// What `config` keeps for nearby transfer, as it holds now: the devices of phones no longer paired not trusted.
pub fn kept_of(config: &Config) -> Kept {
    let mut kept = config.nearby.clone();
    kept.trusted.retain(|trusted| trusted.phone.as_ref().is_none_or(|id| config.phones.iter().any(|phone| &phone.id == id && phone.device.is_some())));
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
        trusted.push(super::Trusted { fingerprint: fingerprint.clone(), alias: alias.clone(), kind: Some("mobile".into()), last, since, phone: Some(phone.to_string()) });
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
        "on": running.is_some(),
        "alias": running.map(|running| running.receiver.me().alias),
        "port": running.and_then(|running| running.receiver.info.port),
        "fingerprint": running.map(|running| running.receiver.info.fingerprint.clone()),
    })
}

/// What nearby transfer is, as Glance's settings show it: whether it is on, what this PC is called (and would be by
/// default), its fingerprint, its port (where it runs), the devices trusted, whether others are turned away.
pub fn said(config: &Config) -> serde_json::Value {
    let kept = &kept_of(config);
    let fingerprint = tls::Own::in_folder(&crate::config::folder()).map(|own| own.fingerprint).ok();
    json!({
        "on": kept.on,
        "alias": kept.alias,
        "name": config.name,
        "fingerprint": fingerprint,
        "port": RUNNING.get().and_then(|running| running.receiver.info.port),
        "trusted": kept.trusted,
        "others_refused": kept.others_refused,
    })
}

/// `glance-remote nearby [on | off | alias NAME | forget FINGERPRINT | others ask|refuse | most FILE SENDING]`: what is
/// kept changed as said, then (or with nothing said) said as `said` has it.
pub fn asked(words: &[String]) -> std::io::Result<serde_json::Value> {
    let word = |at: usize| words.get(at).map(String::as_str);
    let change = |with: &dyn Fn(&mut Kept)| Config::change(|config| with(&mut config.nearby)).map(|_| ());
    match (word(0), word(1)) {
        (Some("on"), _) => change(&|kept| kept.on = true)?,
        (Some("off"), _) => change(&|kept| kept.on = false)?,
        (Some("alias"), name) => {
            let name = name.map(super::shown_name).filter(|name| name != "file" && !name.trim().is_empty());
            change(&|kept| kept.alias = name.clone())?
        }
        (Some("forget"), Some(fingerprint)) => change(&|kept| kept.trusted.retain(|trusted| !trusted.fingerprint.eq_ignore_ascii_case(fingerprint)))?,
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
        config.phones = vec![
            Phone { id: "one".into(), device: Some("phone".into()), ..Default::default() },
            Phone { id: "offered".into(), offered_till: Some(u64::MAX), ..Default::default() },
        ];
        let kept: Vec<String> = kept_of(&config).trusted.into_iter().map(|trusted| trusted.fingerprint).collect();
        assert_eq!(kept, ["AA", "BB"]);
    }
}
