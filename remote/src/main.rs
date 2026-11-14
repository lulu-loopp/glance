//! Glance's remote control helper: a paired phone, on the same network, may
//! ask this PC to put its screens and lights out, to sleep, to turn the
//! sound up or down. It runs as the user, never elevated, only while remote
//! control is on; the monitor itself opens no port.
//!
//!     glance-remote            listens
//!     glance-remote pair       shows the pairing code (a QR code) for the phone
//!     glance-remote lights     lists the lighting devices OpenRGB reaches
//!     glance-remote monitors   lists the monitors and what they answer over DDC/CI
//!     glance-remote screens    says how the screens are put out (signal, kept); with one of those, sets it
//!     glance-remote ask METHOD PATH [JSON]   asks the helper that listens, from this PC, as a phone would
//!     glance-remote address    says where the page for a phone's browser is
//!     glance-remote restore    puts on again the monitors a helper since ended was keeping dark
//!     glance-remote night-key [new|none]   says the key night is put on and off by alone; makes one anew, or keeps none

// No console of its own: it runs unseen for as long as the remote is on.
// (Asked something from a terminal, it answers into that terminal: see `main`.)
#![windows_subsystem = "windows"]

/// Says a line to whoever asked (see `speak_to_the_terminal`); one who has
/// stopped listening is not said it, and that is no failure of this.
macro_rules! say {
    ($($said:tt)*) => {{
        use std::io::Write;
        let _ = writeln!(std::io::stdout(), $($said)*);
    }};
}

mod actions;
mod config;
mod cooler;
mod display;
mod fetch;
mod journal;
mod keys;
mod lights;
mod media;
mod nearby;
mod power;
mod reads;
mod sign;
mod touch;

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use config::{Config, Effect, Light, Night, Phone, Screens};
use serde_json::{json, Value};

/// The most a request may be: none of the phone's is near it.
const MOST: usize = 16 << 10;
/// How long a request may take to arrive whole.
const ARRIVING: Duration = Duration::from_secs(5);
/// How many requests are answered at once, and how many of them one
/// address may have: more are turned away till those are done.
const AT_ONCE: usize = 64;
const AT_ONCE_EACH: usize = 16;
/// How often screens this helper has put out are looked at: whether the PC
/// has been touched since.
const WATCH: Duration = Duration::from_millis(500);
/// How often lights that follow the heat are looked at again.
const FOLLOW: Duration = Duration::from_secs(2);
/// How long a maker's program started is given, and OpenRGB run by Glance's sensors to be started afresh and to find
/// the devices, before the lights are put back.
const AFRESH: Duration = Duration::from_secs(15);
/// How many of the log's latest lines go into a report.
const REPORTED: usize = 120;
/// The heat the coolest colour is for, and the warmest (°C).
const COOL: i64 = 40;
const HOT: i64 = 85;

/// The colour (RRGGBB) for how warm the PC is: blue while it is cool,
/// through green and amber, to red when it is hot.
fn heat(celsius: i64) -> String {
    let along = (celsius.clamp(COOL, HOT) - COOL) as f32 / (HOT - COOL) as f32;
    // Round the wheel of hues from blue (210 degrees) back to red (0).
    let hue = 210.0 * (1.0 - along) / 60.0;
    let channel = |n: f32| {
        let k = (n + hue) % 6.0;
        ((1.0 - k.min(4.0 - k).clamp(0.0, 1.0)) * 255.0).round() as u8
    };
    format!("{:02X}{:02X}{:02X}", channel(5.0), channel(3.0), channel(1.0))
}

#[derive(Default)]
struct State {
    seen: sign::Seen,
    /// Whether the lights are on, once this helper has set them.
    lights: Option<bool>,
    /// How many lighting devices OpenRGB had as it last set them (not set yet: not known).
    devices: Option<usize>,
    night: Option<Night>,
}

/// A light's colour as it shows: its own, as bright as the light is (see `Light`).
fn shown(light: &Light) -> [u8; 3] {
    let rgb = u32::from_str_radix(&light.color, 16).unwrap_or(0xFFFFFF);
    [(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8].map(|part| (part as u32 * light.brightness.min(100) as u32 / 100) as u8)
}

/// Whether there are lights here the phone can set: the light under the keys, a cooler's display, or devices OpenRGB
/// sets (as many as it had as it last set them; not set yet, some, as far as is known: it is not started to be asked).
fn lights_there(keys: bool, cooler: bool, openrgb: bool, devices: Option<usize>) -> bool {
    keys || cooler || (openrgb && devices != Some(0))
}

/// The lights put on or out, as what was asked came to: the keys' light, a cooler's display, and how many devices
/// OpenRGB set (0, where it was not asked). Taken by any, they were set (what failed of the rest is noted); taken by
/// none, they were not, and the phone is told so.
fn lights_taken(on: bool, keyed: bool, cooled: bool, set: Result<usize, String>) -> Result<(), String> {
    match set {
        Err(error) if keyed || cooled => {
            journal::note(format!("the lights {}: the keys' or the cooler's were set; OpenRGB: {error}", if on { "on" } else { "out" }));
            Ok(())
        }
        Ok(0) if !keyed && !cooled => Err("no lights took it".into()),
        other => other.map(|_| ()),
    }
}

/// Whether this PC's lights are the light under its keys: a laptop whose
/// keyboard's light can be set here.
fn lights_are_keys() -> bool {
    actions::laptop() && keys::there()
}

/// How many colours are kept to choose from at most.
const COLOURS_MOST: usize = 24;

struct Helper {
    config: Config,
    /// How the lights are when they are on: as the phone last chose.
    light: Mutex<Light>,
    /// The colours they are chosen from, once kept (see `Config::colours`).
    colours: Mutex<Option<Vec<String>>>,
    /// How bright the light under the keys is with them, once chosen.
    keys: Mutex<Option<u8>>,
    /// The phones paired with, as the configuration had them when it was
    /// last kept (and when that was): read again once it is kept anew, by
    /// this or by another of this program offering a pairing.
    phones: Mutex<(Option<std::time::SystemTime>, Vec<Phone>)>,
    state: Mutex<State>,
    /// Held while the lights are being set: one at a time.
    lighting: Mutex<()>,
    /// A maker's own lighting program has started since OpenRGB was last
    /// started: OpenRGB is to be started afresh before it is asked again.
    stale: std::sync::atomic::AtomicBool,
    /// Held while night begins or ends: one such change at a time, whole.
    nighting: Mutex<()>,
    /// This run of the helper, by a number made up as it started: a request
    /// is signed for the run it is meant for, so that one overheard before
    /// the helper started again (what it had seen forgotten) is not taken.
    session: String,
    /// The addresses being answered now, and how many requests of each.
    callers: Mutex<std::collections::HashMap<std::net::IpAddr, usize>>,
}

/// A request being answered: counted for its address till it is done.
struct Caller(Arc<Helper>, std::net::IpAddr);

impl Drop for Caller {
    fn drop(&mut self) {
        let mut callers = self.0.callers.lock().unwrap();
        if let Some(count) = callers.get_mut(&self.1) {
            *count -= 1;
            if *count == 0 {
                callers.remove(&self.1);
            }
        }
    }
}

/// A request that is a paired phone's: the key it was signed with, as it
/// is kept and as it signs.
struct Admitted {
    key: String,
    secret: [u8; 32],
}

/// Whether the lights are on, as this helper starts: as OpenRGB has them if it is running, else as they were last set.
/// One run by Glance's sensors starts with the PC as this does: it says nothing yet of how they are to be, having
/// found them as the PC left them, or not found them yet, and they are put back as they were last set.
fn lit_at_start(raised: bool, last_set: Option<bool>, running: impl FnOnce() -> Option<bool>) -> Option<bool> {
    if raised {
        last_set.or_else(running)
    } else {
        running().or(last_set)
    }
}

impl Helper {
    fn lights(&self, on: bool) -> Result<(), String> {
        // (The state is not held while OpenRGB is asked: that may take a while.)
        let _one = self.lighting.lock().unwrap();
        self.refresh();
        let mut light = self.light.lock().unwrap().clone();
        if light.effect == Effect::Heat {
            if let Some(celsius) = reads::read().as_ref().and_then(reads::hottest) {
                light.color = heat(celsius);
            }
        }
        // A cooler's little display goes out and comes on with them
        // (whether or not there are other lights to set), and meanwhile:
        // each takes its own while.
        let cooler = std::thread::spawn(move || if on { cooler::lit() } else { cooler::dark() });
        // And the light under a laptop's keys: out, and on again as bright as it was chosen (not chosen yet, the
        // brightest).
        let keyed = keys::there() && keys::light(if on { self.keys.lock().unwrap().unwrap_or(keys::MOST) } else { 0 });
        // (In the lights' colour, doing what they do, where the keyboard takes a colour and its light is this PC's lights.)
        if on && keyed && lights_are_keys() && keys::coloured() {
            keys::colour(shown(&light), match light.effect {
                Effect::Breathing => keys::BREATHING,
                Effect::Rainbow => keys::RAINBOW,
                _ => keys::STEADY,
            });
        }
        // A laptop whose keys' light is set here has that for its lights: OpenRGB is not asked there (it knows
        // few laptops, and not starting on one must not stand for the keys' light not having been set). With no
        // OpenRGB on this PC, the lights it would set are left to their makers' programs: there is still the rest.
        let set = if lights_are_keys() || lights::program(&self.config).is_none() { Ok(0) } else { lights::set(&self.config, on, &light).map_err(|error| error.to_string()) };
        if let Ok(devices) = set {
            if !lights_are_keys() && lights::program(&self.config).is_some() {
                self.state.lock().unwrap().devices = Some(devices);
            }
        }
        let cooled = cooler.join().unwrap_or(false);
        lights_taken(on, keyed, cooled, set)?;
        self.state.lock().unwrap().lights = Some(on);
        let _ = Config::change(|config| config.lit = Some(on));
        // (The OpenRGB that set them stays: some devices fall back to their own light once it goes, OpenRGB having
        // kept them as it set them.)
        Ok(())
    }

    /// OpenRGB started afresh, if a maker's program has taken the lights
    /// from the one running (see `lights::RIVALS`).
    fn refresh(&self) {
        if self.stale.swap(false, std::sync::atomic::Ordering::SeqCst) {
            lights::afresh();
        }
    }

    /// The lights put back as they were last set, once OpenRGB run by Glance's sensors is there again: a maker's
    /// program sets the memory's as it starts, and those lights were left as it set them till they were next asked.
    fn put_back(&self) {
        let _one = self.lighting.lock().unwrap();
        let Some(on) = self.state.lock().unwrap().lights else { return };
        if lights_are_keys() {
            return;
        }
        if let Err(error) = lights::found_all() {
            journal::note(format!("the lights were not put back: {error}"));
            return;
        }
        let mut light = self.light.lock().unwrap().clone();
        if light.effect == Effect::Heat {
            if let Some(celsius) = reads::read().as_ref().and_then(reads::hottest) {
                light.color = heat(celsius);
            }
        }
        if let Err(error) = lights::set(&self.config, on, &light) {
            journal::note(format!("the lights were not put back: {error}"));
        }
    }

    /// Lights that follow the heat, put in the colour of how warm the PC
    /// is now, where that is another than `last` (the one they were last
    /// put in by this).
    fn follow(&self, last: &mut Option<String>) {
        let _one = self.lighting.lock().unwrap();
        let lit = {
            let state = self.state.lock().unwrap();
            state.lights == Some(true) && state.night.is_none()
        };
        if !lit || self.light.lock().unwrap().effect != Effect::Heat {
            *last = None;
            return;
        }
        let Some(color) = reads::read().as_ref().and_then(reads::hottest).map(heat) else { return };
        self.refresh();
        let warm = Light { color: color.clone(), effect: Effect::Steady, brightness: self.light.lock().unwrap().brightness };
        if last.as_ref() == Some(&color) {
            return;
        }
        let set = if lights_are_keys() { keys::colour(shown(&warm), keys::STEADY) } else { lights::set(&self.config, true, &warm).is_ok() };
        if set {
            *last = Some(color);
        }
    }

    /// The lights' colour or what they do, changed from the phone: kept,
    /// for the lights on from then on.
    fn choose(&self, color: Option<&str>, effect: Option<Effect>, brightness: Option<u64>) {
        let mut light = self.light.lock().unwrap();
        if let Some(color) = color.filter(|color| color.len() == 6 && color.bytes().all(|b| b.is_ascii_hexdigit())) {
            light.color = color.to_ascii_uppercase();
        }
        if let Some(effect) = effect {
            light.effect = effect;
        }
        // (Never none: that is the lights off, which is asked for as such.)
        if let Some(brightness) = brightness {
            light.brightness = brightness.clamp(1, 100) as u8;
        }
        let chosen = light.clone();
        let _ = Config::change(|config| config.light = chosen);
    }

    /// Whether there is room to answer one more request from `from`:
    /// then it is counted, till what is returned is let go.
    fn room(self: &Arc<Self>, from: std::net::IpAddr) -> Option<Caller> {
        let mut callers = self.callers.lock().unwrap();
        if callers.values().sum::<usize>() >= AT_ONCE || callers.get(&from).is_some_and(|count| *count >= AT_ONCE_EACH) {
            return None;
        }
        *callers.entry(from).or_default() += 1;
        Some(Caller(self.clone(), from))
    }

    /// Whether a request is a paired phone's: signed with a key kept, for
    /// this run of the helper, now, by the phone the key belongs to, and
    /// not sent before. A key not used before becomes that phone's.
    /// Refused, the key it was signed with comes too where there was one
    /// (the answer can then be signed).
    /// One asked from this PC itself (`here`), as this PC, is taken by
    /// this PC's own key (see `Config::local`), which no other address may
    /// use.
    fn admit(&self, request: &sign::Request, here: bool) -> Result<Admitted, (sign::Refused, Option<[u8; 32]>)> {
        if request.phone == LOCAL {
            let secret = Config::load().local.as_deref().and_then(sign::key).filter(|_| here).ok_or((sign::Refused::Forged, None))?;
            let at = sign::verify(&secret, request).map_err(|refused| (refused, None))?;
            let now = sign::now();
            if request.session != self.session || !sign::timely(at, now) {
                return Err((sign::Refused::Stale, Some(secret)));
            }
            self.state.lock().unwrap().seen.take(&secret, at, request.nonce, now).map_err(|why| (why, Some(secret)))?;
            return Ok(Admitted { key: String::new(), secret });
        }
        let mut phones = self.phones.lock().unwrap();
        let kept = Config::kept_at();
        if phones.0 != kept {
            *phones = (kept, Config::load().phones);
        }
        let now = sign::now();
        let signed = phones.1.iter().filter(|phone| phone.offered_till.is_none_or(|till| till > now)).find_map(|phone| {
            let secret = sign::key(&phone.key)?;
            match sign::verify(&secret, request) {
                Ok(at) => Some(Ok((phone, secret, at))),
                Err(sign::Refused::Forged) => None,
                Err(refused) => Some(Err(refused)),
            }
        });
        let Some((phone, secret, at)) = signed.transpose().map_err(|refused| (refused, None))? else { return Err((sign::Refused::Forged, None)) };
        let refused = |why| Err((why, Some(secret)));
        if request.session != self.session || !sign::timely(at, now) {
            return refused(sign::Refused::Stale);
        }
        if phone.device.as_deref().is_some_and(|owner| owner != request.phone) {
            return refused(sign::Refused::Taken);
        }
        if let Err(why) = self.state.lock().unwrap().seen.take(&secret, at, request.nonce, now) {
            return refused(why);
        }
        let key = phone.key.clone();
        // (A request that names no phone leaves the key as it is.)
        if phone.device.is_none() && !request.phone.is_empty() {
            let (device, name) = (request.phone.to_string(), unescaped(request.name));
            let changed = Config::change(|config| {
                // A phone paired anew is that phone still: what it was paired by before is gone.
                config.phones.retain(|phone| phone.device.as_deref() != Some(device.as_str()));
                if let Some(phone) = config.phones.iter_mut().find(|phone| phone.key == key) {
                    *phone = Phone { id: phone.id.clone(), key: key.clone(), name, device: Some(device), offered_till: None };
                }
            });
            if let Ok(config) = changed {
                *phones = (Config::kept_at(), config.phones);
            }
        }
        Ok(Admitted { key, secret })
    }

    /// The screens out, the way that is chosen (see `display`).
    fn screens_off(&self) -> Result<(), String> {
        match Config::load().way() {
            Screens::Signal => display::sleep().map_err(|error| error.to_string()),
            Screens::Kept => {
                display::darken();
                self.keep_held().map_err(|error| error.to_string())
            }
        }
    }

    /// The monitors this helper has put out with their signal kept, kept
    /// in its configuration as they are now: one that ends before it has
    /// put them back does so when it next starts.
    fn keep_held(&self) -> std::io::Result<()> {
        let held = display::held();
        if Config::load().darkened == held {
            return Ok(());
        }
        Config::change(|config| config.darkened = held).map(|_| ())
    }

    /// The screens on again, whichever way they were put out.
    fn screens_on(&self) {
        display::wake();
        let _ = self.keep_held();
    }

    /// Night: the lights out, the sound muted, the power-saving plan, the
    /// screens off. Day again: the plan there was, the sound as it was, and
    /// the lights on (the screens come on at the touch that ends it, or stay
    /// off till one).
    fn night(&self, on: bool) -> Result<(), String> {
        let _one = self.nighting.lock().unwrap();
        if on {
            if self.state.lock().unwrap().night.is_some() {
                return Ok(());
            }
            // The lights first: they may take OpenRGB a while to reach.
            let lit = self.lights(false);
            // Muted, if it is not already (then it stays so after: it was muted before night).
            let muted = actions::volume().is_some_and(|(_, muted)| !muted) && actions::set_volume(None, Some(true)).is_ok();
            let power = power::save();
            let dark = self.screens_off();
            let night = Night { power, muted };
            // (Kept, so that night is ended by the next helper where this one ends first.)
            let _ = Config::change(|config| config.night = Some(night.clone()));
            self.state.lock().unwrap().night = Some(night);
            journal::note(format!("night began (lights: {}; screens: {})", lit.as_ref().map_or_else(|error| error.as_str(), |_| "out"), dark.as_ref().map_or_else(|error| error.as_str(), |_| "out")));
            lit.and(dark)
        } else {
            let Some(night) = self.state.lock().unwrap().night.take() else { return Ok(()) };
            if let Some(power) = night.power {
                power::restore(power);
            }
            if night.muted {
                let _ = actions::set_volume(None, Some(false));
            }
            self.screens_on();
            let lit = self.lights(true);
            let _ = Config::change(|config| config.night = None);
            journal::note(format!("night ended (lights: {})", lit.as_ref().map_or_else(|error| error.as_str(), |_| "on")));
            lit
        }
    }

    fn state(&self) -> Value {
        // As OpenRGB has them; while it is not running, as this helper last
        // set them. A laptop whose lights are its keys' is never asked of
        // OpenRGB (see `lights`): as set here, whatever OpenRGB last had.
        let kept = self.state.lock().unwrap().lights;
        let known = if lights_are_keys() { None } else { lights::known() };
        let lights = known.as_ref().map(|devices| devices.iter().any(lights::Device::lit)).or(kept);
        let light = self.light.lock().unwrap().clone();
        let playing = media::playing();
        let state = self.state.lock().unwrap();
        let sound = actions::volume();
        json!({
            "name": self.config.name,
            "version": env!("CARGO_PKG_VERSION"),
            "lights": lights,
            // Whether there is the program the lights are set by at all (none: the phone has no lights to show).
            "lights_there": lights_there(keys::there(), cooler::there(), lights::program(&self.config).is_some(), known.map(|devices| devices.len()).or(state.devices)),
            // And whether, of those, there is the program that sets a board's and a card's lights and their colours
            // (not asked on a laptop whose lights are its keys': see `lights`).
            "colours_there": if lights_are_keys() { keys::coloured() } else { lights::program(&self.config).is_some() },
            // A laptop is drawn as one, and its lights are called what they are.
            "laptop": actions::laptop(),
            "light": light,
            "colours": *self.colours.lock().unwrap(),
            "reads": reads::read(),
            // A laptop's keys: how bright their light is set to be with the lights on (not known till chosen), of how bright it goes.
            "keys": keys::there().then(|| json!({ "level": *self.keys.lock().unwrap(), "most": keys::MOST })),
            "media": playing.map(|media| json!({ "title": media.title, "artist": media.artist, "app": media.app, "playing": media.playing })),
            "night": state.night.is_some(),
            "power": power::choices().iter().map(|choice| json!({ "id": choice.id, "name": choice.name, "on": choice.on })).collect::<Vec<_>>(),
            "screen": display::on(),
            "monitors": display::monitors().iter().map(|monitor| json!({ "name": monitor.name, "x": monitor.place.0, "y": monitor.place.1, "width": monitor.place.2, "height": monitor.place.3, "on": monitor.on, "brightness": monitor.brightness })).collect::<Vec<_>>(),
            "volume": sound.map(|(level, _)| level),
            "muted": sound.map(|(_, muted)| muted),
            "idle_seconds": actions::tick().wrapping_sub(actions::last_input()) / 1000,
            "nearby": nearby::run::told_to_phones(),
        })
    }

    /// Does what a request asks: its answer, or why not (with the status).
    fn act(self: &Arc<Self>, method: &str, path: &str, body: &[u8], key: &str) -> Result<Value, (u16, String)> {
        let failed = |error: String| (500, error);
        let asked: Value = if body.is_empty() { Value::Null } else { serde_json::from_slice(body).map_err(|_| (400, "the body is not JSON".to_string()))? };
        let on = || asked.get("on").and_then(Value::as_bool).ok_or((400, "\"on\" is missing".to_string()));
        // The remote off (this helper runs for nearby transfer alone), a phone is answered only what nearby transfer
        // has of it, and its unpairing.
        if !self.config.remote && !path.starts_with("/v1/nearby") && path != "/v1/unpair" {
            return Err((403, "the remote is off".into()));
        }
        match (method, path) {
            ("GET", "/v1/state") => {}
            // Where and as whom this PC receives nearby (its certificate's fingerprint, for the phone to pin).
            ("GET", "/v1/nearby") => return Ok(nearby::run::told_to_phones()),
            // The certificate the asking phone sends nearby by: trusted while the phone is paired (it told it signed
            // with its key, which only it has), its files taken without asking.
            ("POST", "/v1/nearby/trust") => {
                let said = |name: &str| asked.get(name).and_then(Value::as_str).unwrap_or_default();
                let phone = Config::load().phones.into_iter().find(|phone| phone.key == key).map(|phone| phone.id).ok_or((403, "not a paired phone".to_string()))?;
                if !nearby::run::phone_trusted(&phone, said("fingerprint"), said("alias")).map_err(|error| failed(error.to_string()))? {
                    return Err((400, "not a certificate's fingerprint".into()));
                }
                return Ok(nearby::run::told_to_phones());
            }
            ("POST", "/v1/screen") => {
                if on()? {
                    self.screens_on()
                } else {
                    self.screens_off().map_err(failed)?
                }
            }
            // One monitor by itself, by its place among them.
            ("POST", "/v1/monitor") => {
                let index = asked.get("index").and_then(Value::as_u64).ok_or((400, "\"index\" is missing".to_string()))? as usize;
                // On or off, or as bright as asked.
                let done = match asked.get("brightness").and_then(Value::as_u64) {
                    Some(percent) => display::dim(index, percent.min(100) as u8),
                    None => display::turn(index, on()?),
                };
                let _ = self.keep_held();
                done.map_err(|why| failed(why.into()))?;
            }
            // On or off, in another colour, doing something else: any of
            // them. A colour or an effect alone lights them so.
            ("POST", "/v1/lights") => {
                let effect = asked.get("effect").and_then(|effect| serde_json::from_value::<Effect>(effect.clone()).ok());
                self.choose(asked.get("color").and_then(Value::as_str), effect, asked.get("brightness").and_then(Value::as_u64));
                self.lights(asked.get("on").and_then(Value::as_bool).unwrap_or(true)).map_err(failed)?
            }
            // The colours to choose from, as they are from now on: each a
            // colour, once, no more than a row of them holds.
            ("POST", "/v1/colours") => {
                let said = asked.get("colours").and_then(Value::as_array).ok_or((400, "\"colours\" is missing".to_string()))?;
                let mut colours: Vec<String> = Vec::new();
                for one in said {
                    let one = one.as_str().filter(|one| one.len() == 6 && one.bytes().all(|b| b.is_ascii_hexdigit())).ok_or((400, "a colour is not RRGGBB".to_string()))?.to_ascii_uppercase();
                    if !colours.contains(&one) {
                        colours.push(one);
                    }
                }
                if colours.len() > COLOURS_MOST {
                    return Err((400, "too many colours".to_string()));
                }
                let kept = colours.clone();
                Config::change(|config| config.colours = Some(kept)).map_err(|error| failed(error.to_string()))?;
                *self.colours.lock().unwrap() = Some(colours);
            }
            // How bright the light under the keys is: set now (unless the
            // lights are out: then for when they are on again), and kept.
            ("POST", "/v1/keys") => {
                let level = asked.get("level").and_then(Value::as_u64).ok_or((400, "\"level\" is missing".to_string()))?.min(keys::MOST as u64) as u8;
                *self.keys.lock().unwrap() = Some(level);
                let _ = Config::change(|config| config.keys = Some(level));
                let out = self.state.lock().unwrap().lights == Some(false);
                if !out && !keys::light(level) {
                    return Err((500, "the keyboard did not take it".into()));
                }
            }
            ("POST", "/v1/media") => {
                match media::ask(asked.get("do").and_then(Value::as_str).unwrap_or_default()) {
                    media::Asked::Taken => {}
                    // A key pressed is a touch of the keyboard, at which night
                    // would end and the screens wake: it is none of the user's,
                    // so night goes on and the screens are put out again.
                    media::Asked::Pressed => {
                        if self.state.lock().unwrap().night.is_some() {
                            let _ = self.screens_off();
                        }
                    }
                    media::Asked::Not => return Err((500, "nothing playing took it".into())),
                }
            }
            // One of the PC's power modes (or plans), by its id as the state
            // gives it. (Night over, what is gone back to is what there was before
            // night, not one chosen during it.)
            ("POST", "/v1/power") => {
                if !power::choose(asked.get("plan").and_then(Value::as_str).unwrap_or_default()) {
                    return Err((500, "the plan was not taken".into()));
                }
            }
            // The phone that asks, paired no more (it has unpaired itself).
            ("POST", "/v1/unpair") => {
                let _ = Config::change(|config| config.phones.retain(|phone| phone.key != key));
            }
            ("POST", "/v1/night") => self.night(on()?).map_err(failed)?,
            ("POST", "/v1/volume") => {
                let level = asked.get("level").and_then(Value::as_u64).map(|level| level.min(100) as u8);
                let muted = asked.get("muted").and_then(Value::as_bool);
                actions::set_volume(level, muted).map_err(|error| failed(error.to_string()))?
            }
            // Answered first: asleep, it answers nothing.
            ("POST", "/v1/sleep") => {
                std::thread::spawn(|| {
                    std::thread::sleep(Duration::from_secs(1));
                    actions::sleep();
                });
            }
            _ => return Err((404, "no such thing to ask".into())),
        }
        Ok(if self.config.remote { self.state() } else { json!({ "name": self.config.name, "remote": false }) })
    }

    /// Reads one request from `stream`, checks it is the paired phone's,
    /// does it and answers.
    fn serve(self: &Arc<Self>, mut stream: TcpStream) -> std::io::Result<()> {
        stream.set_write_timeout(Some(ARRIVING))?;
        // The whole of it is to have come by then, however it trickles in.
        let by = std::time::Instant::now() + ARRIVING;
        let mut chunk = [0u8; 2048];
        let mut more = |stream: &mut TcpStream| -> std::io::Result<Vec<u8>> {
            let left = by.checked_duration_since(std::time::Instant::now()).filter(|left| !left.is_zero()).ok_or(std::io::ErrorKind::TimedOut)?;
            stream.set_read_timeout(Some(left))?;
            let read = stream.read(&mut chunk)?;
            Ok(chunk[..read].to_vec())
        };
        let mut taken = Vec::new();
        // The head, then as much body as it says.
        let (head, mut body) = loop {
            let came = more(&mut stream)?;
            taken.extend_from_slice(&came);
            if let Some(end) = taken.windows(4).position(|four| four == b"\r\n\r\n") {
                break (String::from_utf8_lossy(&taken[..end]).into_owned(), taken[end + 4..].to_vec());
            }
            if came.is_empty() || taken.len() > MOST {
                return Ok(());
            }
        };
        let mut lines = head.split("\r\n");
        let mut request = lines.next().unwrap_or_default().split(' ');
        let (method, path) = (request.next().unwrap_or_default(), request.next().unwrap_or_default());
        let header = |name: &str| lines.clone().filter_map(|line| line.split_once(':')).find(|(key, _)| key.eq_ignore_ascii_case(name)).map_or("", |(_, value)| value.trim());
        let length: usize = header("content-length").parse().unwrap_or(0);
        if length > MOST {
            return Ok(());
        }
        while body.len() < length {
            let came = more(&mut stream)?;
            if came.is_empty() {
                return Ok(());
            }
            body.extend_from_slice(&came);
        }
        body.truncate(length);
        // Night, asked by whoever has the key kept for that alone (see `Config::night_key`): a phone's shortcut,
        // which cannot sign. Nothing else is done by it, and nothing is answered of the PC but whether it is night.
        if path == NIGHT_BY_KEY {
            let given = header("x-glance-key");
            let kept = Config::load().night_key;
            let (status, answer) = if !self.config.remote {
                (403, json!({ "error": "the remote is off" }))
            } else if method != "POST" || !kept.as_deref().is_some_and(|kept| sign::is_key(given, kept)) {
                journal::note(format!("refused {method} {path} from {}: not the night key", stream.peer_addr().map_or_else(|_| "?".into(), |from| from.ip().to_string())));
                (401, json!({ "error": "forged" }))
            } else {
                let on = serde_json::from_slice::<Value>(&body).ok().and_then(|asked| asked.get("on").and_then(Value::as_bool));
                match on {
                    None => (400, json!({ "error": "\"on\" is missing" })),
                    Some(on) => {
                        let done = self.night(on);
                        journal::note(format!("asked {path} by the night key, on {on}: {}", done.as_ref().map_or_else(|error| error.as_str(), |_| "done")));
                        match done {
                            Ok(()) => (200, json!({ "name": self.config.name, "night": on })),
                            Err(error) => (500, json!({ "error": error })),
                        }
                    }
                }
            };
            let answer = answer.to_string();
            return write!(stream, "HTTP/1.1 {status} {}
Content-Type: application/json
Content-Length: {}
Connection: close

{answer}", if status == 200 { "OK" } else { "Error" }, answer.len());
        }
        // The page a phone's browser controls this PC from (a phone with no
        // app for it): anyone's to fetch, for it holds nothing; what it
        // then asks is signed as the app's is, with a key it was paired by.
        if method == "GET" && path == "/" {
            let page = include_str!("web.html");
            return write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}", page.len());
        }
        let nonce = header("x-glance-nonce");
        let request = sign::Request { method, path, time: header("x-glance-time"), nonce, sign: header("x-glance-sign"), phone: header("x-glance-phone"), name: header("x-glance-name"), session: header("x-glance-session"), body: &body };
        let here = stream.peer_addr().is_ok_and(|from| from.ip().is_loopback());
        let (status, answer, secret) = match self.admit(&request, here) {
            // (With which run this is, and what time it has: a phone that had them wrong asks again by them.)
            Err((refused, secret)) => {
                // (Not of now is how every phone's first asking of a run is refused: nothing to note.)
                if refused != sign::Refused::Stale {
                    journal::note(format!("refused {method} {} from {}: {refused:?}", path.chars().take(40).collect::<String>(), stream.peer_addr().map_or_else(|_| "?".into(), |from| from.ip().to_string())));
                }
                (401, json!({ "error": format!("{refused:?}").to_lowercase(), "time": sign::now(), "session": self.session }), secret)
            }
            Ok(admitted) => {
                let done = self.act(method, path, &body, &admitted.key);
                // What was asked to be done, and how it went (not each look, nor each step of a slider: how loud, how bright).
                let slid = path == "/v1/volume" || (path == "/v1/monitor" && body.windows(10).any(|part| part == b"brightness"));
                if method == "POST" && !(slid && done.is_ok()) {
                    journal::note(format!("asked {path} {}: {}", String::from_utf8_lossy(&body), done.as_ref().map_or_else(|(_, error)| error.as_str(), |_| "done")));
                }
                match done {
                    Ok(answer) => (200, answer, Some(admitted.secret)),
                    Err((status, error)) => (status, json!({ "error": error }), Some(admitted.secret)),
                }
            }
        };
        let answer = answer.to_string();
        let reason = if status == 200 { "OK" } else { "Error" };
        // Signed, to whoever signed the request: the answer is this PC's, and to that asking.
        let signed = secret.map(|secret| format!("X-Glance-Sign: {}\r\n", sign::answer(&secret, nonce, status, answer.as_bytes()))).unwrap_or_default();
        write!(stream, "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\n{signed}Content-Length: {}\r\nConnection: close\r\n\r\n{answer}", answer.len())
    }
}

/// This PC's address on the network it reaches others by.
fn address() -> Option<String> {
    // Nothing is sent: a socket "connected" to an address beyond the
    // network takes the address of the way there.
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:9").ok()?;
    Some(socket.local_addr().ok()?.ip().to_string())
}

/// A text's QR code as rows of 1 (dark) and 0, a line each, with no margin.
fn cells(text: &str) -> std::io::Result<String> {
    let code = qrcode::QrCode::new(text.as_bytes()).map_err(std::io::Error::other)?;
    let width = code.width();
    let dark: Vec<bool> = code.to_colors().into_iter().map(|cell| cell == qrcode::Color::Dark).collect();
    Ok(dark.chunks(width).map(|row| row.iter().map(|cell| if *cell { '1' } else { '0' }).collect::<String>() + "\n").collect())
}

/// Text as a link carries it: all but letters, figures and hyphens by
/// their bytes' numbers.
fn escaped(text: &str) -> String {
    text.bytes().map(|b| if b.is_ascii_alphanumeric() || b == b'-' { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// The text a link carried (see `escaped`).
fn unescaped(text: &str) -> String {
    let mut bytes = Vec::new();
    let mut rest = text.as_bytes();
    while let Some((first, after)) = rest.split_first() {
        let number = (*first == b'%' && after.len() >= 2).then(|| std::str::from_utf8(&after[..2]).ok().and_then(|hex| u8::from_str_radix(hex, 16).ok())).flatten();
        match number {
            Some(byte) => {
                bytes.push(byte);
                rest = &after[2..];
            }
            None => {
                bytes.push(*first);
                rest = after;
            }
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Where the page for a phone's browser is: none while this PC is on no network.
fn page(config: &Config) -> Option<String> {
    Some(format!("http://{}:{}/", address()?, config.port))
}

/// What a phone needs to pair, as the link its QR code holds: where this
/// PC is, and a key offered to the phone that takes it.
/// The same for a phone's browser: the page's address, the key after the
/// `#` (which a browser keeps to itself: it is never sent).
fn pairing_web(config: &Config, key: &str) -> String {
    format!("{}#k={key}", page(config).unwrap_or_default())
}

fn pairing(config: &Config, key: &str) -> String {
    format!("glance://pair?h={}&p={}&k={key}&n={}", address().unwrap_or_default(), config.port, escaped(&config.name))
}

/// What this PC calls itself, asking its own helper (see `Helper::admit`).
const LOCAL: &str = "this-pc";

/// Where night is asked for by the night key (see `Config::night_key`).
const NIGHT_BY_KEY: &str = "/v1/night/key";

/// Asks the helper that listens on this PC, as this PC: its answer's
/// status and what it said. Asked again once, for the run the helper says
/// it is, where it says the asking was not of now (as every first asking
/// of a run is).
fn ask_here(config: &Config, method: &str, path: &str, body: &[u8]) -> std::io::Result<(u16, String)> {
    let other = |what: &str| std::io::Error::other(what.to_string());
    let secret = sign::key(&Config::local()?).ok_or_else(|| other("no key of this PC's own"))?;
    let mut session = String::new();
    for _ in 0..2 {
        let mut nonce = [0u8; 8];
        getrandom::getrandom(&mut nonce).map_err(|error| other(&error.to_string()))?;
        let (nonce, time) = (sign::hex(&nonce), sign::now().to_string());
        let sign = sign::sign(&secret, &sign::Request { method, path, time: &time, nonce: &nonce, sign: "", phone: LOCAL, name: "", session: &session, body });
        let mut stream = TcpStream::connect_timeout(&std::net::SocketAddr::from(([127, 0, 0, 1], config.port)), Duration::from_secs(3))?;
        stream.set_read_timeout(Some(Duration::from_secs(90)))?;
        write!(stream, "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nX-Glance-Time: {time}\r\nX-Glance-Nonce: {nonce}\r\nX-Glance-Sign: {sign}\r\nX-Glance-Session: {session}\r\nX-Glance-Phone: {LOCAL}\r\nX-Glance-Name: \r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len())?;
        stream.write_all(body)?;
        let mut answer = Vec::new();
        stream.read_to_end(&mut answer)?;
        let end = answer.windows(4).position(|four| four == b"\r\n\r\n").ok_or_else(|| other("no answer"))?;
        let head = String::from_utf8_lossy(&answer[..end]).into_owned();
        let said = &answer[end + 4..];
        let status: u16 = head.split(' ').nth(1).and_then(|status| status.parse().ok()).ok_or_else(|| other("no answer"))?;
        let signed = head.split("\r\n").filter_map(|line| line.split_once(':')).find(|(name, _)| name.eq_ignore_ascii_case("x-glance-sign")).map_or("", |(_, value)| value.trim());
        // Only what the holder of the key signed is its answer.
        if signed != sign::answer(&secret, &nonce, status, said) {
            return Err(other("an answer not signed by this PC's helper"));
        }
        let stale = status == 401 && serde_json::from_slice::<Value>(said).ok().is_some_and(|said| said["error"] == "stale");
        match serde_json::from_slice::<Value>(said).ok().and_then(|said| said["session"].as_str().map(str::to_string)).filter(|_| stale && session.is_empty()) {
            Some(told) => session = told,
            None => return Ok((status, String::from_utf8_lossy(said).into_owned())),
        }
    }
    Err(other("the helper would not say which run it is"))
}

/// What this says goes to whoever started it and gave it somewhere to say
/// it (Glance, reading it); started from a terminal with nowhere given, to
/// that terminal.
fn speak_to_the_terminal() {
    use windows::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE};
    use windows::Win32::Storage::FileSystem::{CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
    use windows::Win32::System::Console::{AttachConsole, GetStdHandle, SetStdHandle, ATTACH_PARENT_PROCESS, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE};
    unsafe {
        if GetStdHandle(STD_OUTPUT_HANDLE).is_ok_and(|given| !given.is_invalid() && !given.0.is_null()) {
            return;
        }
        if AttachConsole(ATTACH_PARENT_PROCESS).is_ok() {
            if let Ok(terminal) = CreateFileW(windows::core::w!("CONOUT$"), (GENERIC_READ | GENERIC_WRITE).0, FILE_SHARE_READ | FILE_SHARE_WRITE, None, OPEN_EXISTING, Default::default(), None) {
                let _ = SetStdHandle(STD_OUTPUT_HANDLE, terminal);
                let _ = SetStdHandle(STD_ERROR_HANDLE, terminal);
            }
        }
    }
}

fn main() -> std::io::Result<()> {
    speak_to_the_terminal();
    // The monitors' sizes in their own pixels, whatever each one's scale.
    let _ = unsafe { windows::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    let config = Config::load();
    match std::env::args().nth(1).as_deref() {
        // glance-remote pair [web] [cells]: a pairing offered, as its link
        // (for a phone's browser rather than the app, with "web"). With
        // "cells", the link's QR code follows as rows of 1 and 0 (for
        // Glance to draw); without, it follows drawn in the console's
        // characters, to be scanned off the screen (written to a file, the
        // key in it was left on the disk).
        Some("pair") => {
            let asked = |word: &str| std::env::args().skip(2).any(|given| given == word);
            let key = Config::offer()?;
            let link = if asked("web") { pairing_web(&config, &key) } else { pairing(&config, &key) };
            say!("{link}");
            if asked("cells") {
                say!("{}", cells(&link)?.trim_end());
            } else {
                let code = qrcode::QrCode::new(link.as_bytes()).map_err(std::io::Error::other)?;
                say!("{}", code.render::<qrcode::render::unicode::Dense1x2>().dark_color(qrcode::render::unicode::Dense1x2::Light).light_color(qrcode::render::unicode::Dense1x2::Dark).quiet_zone(true).build());
            }
            return Ok(());
        }
        // glance-remote qr TEXT: any text's QR code, as rows of 1 and 0.
        Some("qr") => {
            say!("{}", cells(&std::env::args().nth(2).unwrap_or_default())?.trim_end());
            return Ok(());
        }
        // glance-remote lights there | fetch | forget: whether there is an
        // OpenRGB to set the lights by; one fetched for it; that one taken
        // away again (the lights then left to their makers' own programs).
        Some("lights") if std::env::args().nth(2).is_some() => {
            match std::env::args().nth(2).as_deref() {
                // (One the user has put there themselves does as well as one fetched.)
                Some("fetch") => {
                    let config = Config::change(|config| config.lights_left_alone = false)?;
                    if lights::program(&config).is_none() {
                        fetch::fetch()?;
                    }
                }
                Some("forget") => {
                    lights::afresh();
                    Config::change(|config| {
                        config.lights_left_alone = true;
                        config.lights_raised = false;
                    })?;
                    fetch::forget()?;
                }
                // Whether Glance's sensors run OpenRGB; that they are to (the release they take it from fetched, where
                // none was put with Glance, and the one this helper started stopped: it holds the port); that they no
                // longer do.
                Some("raised") => {
                    say!("{}", if Config::load().lights_raised { "raised" } else { "plain" });
                    return Ok(());
                }
                Some("raise") => {
                    // (The sensors run the one put with Glance, where there is one: see `lights::with_glance`.)
                    if lights::with_glance().is_none() {
                        fetch::fetch()?;
                    }
                    // (Told first: lights asked for as this one is stopped start no other.)
                    Config::change(|config| {
                        config.lights_left_alone = false;
                        config.lights_raised = true;
                    })?;
                    lights::afresh();
                    Config::change(|config| config.openrgb_started = None)?;
                    say!("raised");
                    return Ok(());
                }
                Some("lower") => {
                    Config::change(|config| config.lights_raised = false)?;
                    say!("plain");
                    return Ok(());
                }
                _ => {}
            }
            say!("{}", if lights::program(&Config::load()).is_some() { "there" } else { "none" });
            return Ok(());
        }
        Some("lights") => {
            for device in lights::devices(&config)? {
                say!("{} now: {} ({}), first colours {:06X?}", device.name, device.modes.get(device.active).map_or("?", |mode| mode.name.as_str()), if device.lit() { "lit" } else { "dark" }, device.colors.iter().take(3).collect::<Vec<_>>());
                say!("{}\n  modes: {}\n  zones: {}", device.name, device.modes.iter().map(|mode| mode.name.as_str()).collect::<Vec<_>>().join(", "), device.zones.iter().map(|zone| format!("{} ({} LEDs)", zone.name, zone.leds)).collect::<Vec<_>>().join(", "));
            }
            return Ok(());
        }
        // glance-remote nearby [on | off | alias NAME | forget FINGERPRINT | others ask|refuse | most FILE SENDING]:
        // nearby transfer as kept changed so (taken by the helper as it next starts), then said as JSON.
        Some("nearby") => {
            say!("{}", nearby::run::asked(&std::env::args().skip(2).collect::<Vec<_>>())?);
            return Ok(());
        }
        // glance-remote remote [on | off]: whether phones control this PC (taken by the helper as it next starts:
        // off, it runs for nearby transfer alone), changed so, then said.
        Some("remote") => {
            let config = match std::env::args().nth(2).as_deref() {
                Some(word @ ("on" | "off")) => Config::change(|config| config.remote = word == "on")?,
                _ => config,
            };
            say!("{}", json!({ "remote": config.remote }));
            return Ok(());
        }
        // The phones paired with (and the pairings offered), in their order.
        Some("phones") => {
            for phone in &config.phones {
                let what = match (&phone.device, phone.offered_till) {
                    (Some(_), _) => "paired",
                    (None, Some(_)) => "offered",
                    (None, None) => "paired before phones were told apart",
                };
                say!("{}: {} ({what})", phone.id, if phone.name.is_empty() { "?" } else { &phone.name });
            }
            return Ok(());
        }
        // glance-remote unpair ID: the phone of that name (as `phones` lists them), paired no more.
        Some("unpair") => {
            if let Some(id) = std::env::args().nth(2) {
                Config::change(|config| config.phones.retain(|phone| phone.id != id))?;
            }
            return Ok(());
        }
        // glance-remote cooler off|on: the cooler's display alone.
        Some("cooler") => {
            let off = std::env::args().nth(2).as_deref() == Some("off");
            say!("{}", if off { cooler::dark() } else { cooler::lit() });
            return Ok(());
        }
        Some("media") => {
            say!("{:?}", media::sounding());
            if let Some(playing) = media::playing() {
                say!("{:?} by {:?} in {}: playing {}", playing.title, playing.artist, playing.app, playing.playing);
            }
            return Ok(());
        }
        Some("power") => {
            for choice in power::choices() {
                say!("{} {}{}", choice.id, choice.name, if choice.on { " *" } else { "" });
            }
            return Ok(());
        }
        // glance-remote ask METHOD PATH [JSON]: the helper that listens asked from this PC, as a phone asks it;
        // its answer's status, then what it said (0 and why, where it could not be asked).
        Some("ask") => {
            let argument = |at: usize| std::env::args().nth(at).unwrap_or_default();
            match ask_here(&config, &argument(2), &argument(3), argument(4).as_bytes()) {
                Ok((status, said)) => {
                    say!("{status}");
                    say!("{said}");
                }
                Err(error) => {
                    say!("0");
                    say!("{error}");
                }
            }
            return Ok(());
        }
        // glance-remote address: where the page is that a phone's browser, paired by it before, opens again.
        Some("address") => {
            say!("{}", page(&config).unwrap_or_default());
            return Ok(());
        }
        // glance-remote restore: the helper that listens has been ended (the remote turned off, Glance being
        // removed); monitors it was keeping dark are put on again, as that helper would at its next start.
        Some("restore") => {
            if !config.darkened.is_empty() {
                journal::note(format!("{} monitors were left dark by a helper that was ended: put on again", config.darkened.len()));
                // (Those that did not come on are kept, for the next helper to try again.)
                let dark = display::restore(&config.darkened);
                Config::change(|config| config.darkened = dark)?;
            }
            // Night it had on: the power plan and the sound as they were before it (the lights are no longer this
            // helper's to set).
            if let Some(night) = &config.night {
                journal::note("night was left on by a helper that was ended: its power plan and sound put back");
                if let Some(power) = night.power {
                    power::restore(power);
                }
                if night.muted {
                    let _ = actions::set_volume(None, Some(false));
                }
                Config::change(|config| config.night = None)?;
            }
            return Ok(());
        }
        // glance-remote night-key [new|none]: the key night alone is put on and off by, said; made anew first (any
        // shortcut made with the one before asks in vain from then on), or none kept.
        Some("night-key") => {
            let config = match std::env::args().nth(2).as_deref() {
                Some("new") => {
                    let mut key = [0u8; 32];
                    getrandom::getrandom(&mut key).map_err(|error| std::io::Error::other(error.to_string()))?;
                    Config::change(|config| config.night_key = Some(sign::hex(&key)))?
                }
                Some("none") => Config::change(|config| config.night_key = None)?,
                _ => config,
            };
            say!("{}", config.night_key.as_deref().unwrap_or("none"));
            return Ok(());
        }
        // glance-remote screens [signal|kept]: how the screens are put out, set (with one of the two) and said.
        Some("screens") => {
            let way = match std::env::args().nth(2).as_deref() {
                Some("signal") => Some(Screens::Signal),
                Some("kept") => Some(Screens::Kept),
                _ => None,
            };
            let config = match way {
                Some(way) => {
                    journal::note(format!("the screens are to be put out the {way:?} way"));
                    Config::change(|config| (config.screens, config.screens_chosen) = (way, true))?
                }
                None => config,
            };
            say!("{}", format!("{:?}", config.way()).to_lowercase());
            return Ok(());
        }
        Some("monitors") => {
            for monitor in display::monitors() {
                say!("{} ({}x{} at {},{}): on {:?}", monitor.name, monitor.place.2, monitor.place.3, monitor.place.0, monitor.place.1, monitor.on);
                say!("  brightness {:?}", monitor.brightness);
            }
            return Ok(());
        }
        // glance-remote report: what there is on this PC for the remote to
        // set, for a report of what is not right (nothing of the phones paired).
        Some("report") => {
            say!("version {}", env!("CARGO_PKG_VERSION"));
            if !Config::readable() {
                say!("configuration: kept, but not to be read");
            }
            say!("OpenRGB: {}", lights::program(&config).map_or("none".to_string(), |path| path.display().to_string()));
            // (Only an OpenRGB running already is asked: one is not started for a report.)
            match lights::known() {
                Some(devices) => {
                    for device in devices {
                        say!("light: {} ({}), modes {}, zones {}", device.name, device.modes.get(device.active).map_or("?", |mode| mode.name.as_str()), device.modes.iter().map(|mode| mode.name.as_str()).collect::<Vec<_>>().join("/"), device.zones.iter().map(|zone| format!("{}:{}", zone.name, zone.leds)).collect::<Vec<_>>().join("/"));
                    }
                }
                None => say!("lights: OpenRGB not running"),
            }
            for monitor in display::monitors() {
                say!("monitor: {} {}x{} at {},{}; on {:?}; brightness {:?}", monitor.name, monitor.place.2, monitor.place.3, monitor.place.0, monitor.place.1, monitor.on, monitor.brightness);
            }
            say!("cooler display: {}", if cooler::there() { "there" } else { "none" });
            say!("keyboard light: {}", if keys::there() { "there" } else { "none" });
            say!("power: {}", power::choices().iter().map(|choice| format!("{}{}", if choice.name.is_empty() { &choice.id } else { &choice.name }, if choice.on { "*" } else { "" })).collect::<Vec<_>>().join(", "));
            say!("screens put out: {}{}", format!("{:?}", config.way()).to_lowercase(), if config.screens_chosen { " (chosen)" } else { "" });
            say!("a laptop: {}; stands by with its screens: {}", actions::laptop(), actions::stands_by_with_its_screens());
            for line in journal::recent(REPORTED) {
                say!("log: {line}");
            }
            return Ok(());
        }
        // glance-remote keys LEVEL: the light under a laptop's keys.
        Some("keys") => {
            // glance-remote keys [LEVEL | colour RRGGBB | parts]: the light under the keys, set or said.
            let colour = |text: String| u32::from_str_radix(&text, 16).ok().filter(|_| text.len() == 6).map(|rgb| [(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8]);
            match (std::env::args().nth(2).as_deref(), std::env::args().nth(3)) {
                // (And what it does, by the keyboard's own number for it: steady, unless said.)
                (Some("colour"), Some(text)) => match colour(text) {
                    Some(rgb) => say!("{}", if keys::colour(rgb, std::env::args().nth(4).and_then(|does| does.parse().ok()).unwrap_or(keys::STEADY)) { "taken" } else { "not taken" }),
                    None => say!("a colour is RRGGBB"),
                },
                (Some("parts"), _) => {
                    keys::parts().iter().for_each(|part| say!("{part}"));
                    say!("its light takes a colour: {}", keys::coloured());
                }
                (Some(level), _) if level.parse::<u8>().is_ok() => say!("{}", if keys::light(level.parse().unwrap_or(0)) { "taken" } else { "not taken" }),
                _ => say!("{}", if keys::there() { "a keyboard whose light can be set" } else { "none" }),
            }
            return Ok(());
        }
        // glance-remote dim MONITOR PERCENT: one monitor as bright as that.
        Some("dim") => {
            let number = |at: usize| std::env::args().nth(at).and_then(|text| text.parse::<usize>().ok());
            if let (Some(monitor), Some(percent)) = (number(2), number(3)) {
                say!("{}", display::dim(monitor, percent.min(100) as u8).map_or_else(|why| why, |_| "taken"));
            }
            return Ok(());
        }
        // glance-remote screen off|on: every screen.
        Some("screen") => {
            if std::env::args().nth(2).as_deref() == Some("off") {
                say!("{:?}", display::sleep());
            } else {
                display::wake();
            }
            return Ok(());
        }
        // glance-remote vcp MONITOR CODE VALUE: one feature of one monitor, set.
        Some("vcp") => {
            let number = |at: usize, radix: u32| std::env::args().nth(at).and_then(|text| u32::from_str_radix(&text, radix).ok());
            if let (Some(monitor), Some(code), Some(value)) = (number(2, 10), number(3, 16), number(4, 10)) {
                say!("{}", if display::tell(monitor as usize, code as u8, value) { "told" } else { "not taken" });
            }
            return Ok(());
        }
        _ => {}
    }
    journal::note_crashes();
    journal::note(format!("started: version {}, port {}, the screens put out the {:?} way, phones control this PC: {}, nearby transfer: {}", env!("CARGO_PKG_VERSION"), config.port, config.way(), config.remote, config.nearby.on));
    for monitor in display::monitors() {
        journal::note(format!("monitor: {} {}x{} at {},{}; says it is on: {:?}; brightness: {:?}", monitor.name, monitor.place.2, monitor.place.3, monitor.place.0, monitor.place.1, monitor.on, monitor.brightness));
    }
    // Monitors a helper before this one kept dark and did not put back (it was ended first): put back now.
    if !config.darkened.is_empty() {
        journal::note(format!("{} monitors were left dark by a helper that ended: put on again", config.darkened.len()));
        // (Those that did not come on are held as off still: see `display::hold`.)
        let dark = display::restore(&config.darkened);
        display::hold(dark.clone());
        Config::change(|config| config.darkened = dark)?;
    }
    display::watch();
    let listener = TcpListener::bind(("0.0.0.0", config.port)).inspect_err(|error| journal::note(format!("port {} cannot be listened on: {error}", config.port)))?;
    // A phone that has lost this PC (its address changed) asks the whole
    // network for it by its key's name: answered with proof of the key,
    // from the address this PC has now.
    if let Ok(asked) = UdpSocket::bind(("0.0.0.0", config.port)) {
        std::thread::spawn(move || {
            let mut packet = [0u8; 256];
            loop {
                let Ok((length, from)) = asked.recv_from(&mut packet) else { continue };
                let text = String::from_utf8_lossy(&packet[..length]);
                let mut words = text.split(' ');
                let (Some("glance-find"), Some(id), Some(nonce)) = (words.next(), words.next(), words.next()) else { continue };
                if nonce.is_empty() || nonce.len() > 64 || !nonce.bytes().all(|b| b.is_ascii_hexdigit()) {
                    continue;
                }
                let now = sign::now();
                let phones = Config::load().phones;
                let key = phones.iter().filter(|phone| phone.offered_till.is_none_or(|till| till > now)).filter_map(|phone| sign::key(&phone.key)).find(|key| sign::key_id(key) == id);
                if let Some(key) = key {
                    let _ = asked.send_to(format!("glance-here {nonce} {}", sign::here(&key, nonce)).as_bytes(), from);
                }
            }
        });
    }
    let mut session = [0u8; 8];
    getrandom::getrandom(&mut session).map_err(|error| std::io::Error::other(error.to_string()))?;
    let session = sign::hex(&session);
    // (Kept once as this version keeps it: each phone with what it is told from the others by.)
    let _ = Config::change(|_| {});
    if config.nearby.on {
        if let Err(error) = nearby::run::start(&config.name) {
            journal::note(format!("nearby transfer could not start: {error}"));
        }
    }
    let state = State { lights: lit_at_start(config.lights_raised, config.lit, lights::lit), ..State::default() };
    let helper = Arc::new(Helper { light: Mutex::new(config.light.clone()), colours: Mutex::new(config.colours.clone()), keys: Mutex::new(config.keys), phones: Mutex::new((None, Vec::new())), config, state: Mutex::new(state), lighting: Mutex::new(()), stale: std::sync::atomic::AtomicBool::new(false), nighting: Mutex::new(()), session, callers: Mutex::default() });
    // Night a helper before this one put on and did not end (it was ended first): ended now, as the monitors it kept
    // dark are put on again (above), the power plan, the sound and the lights as they were before it.
    if let Some(left) = helper.config.night.clone() {
        journal::note("night was left on by a helper that ended: ended");
        helper.state.lock().unwrap().night = Some(left);
        let _ = helper.night(false);
    }
    // The lights following the heat, and night and dark screens ended at a touch: what phones control, with the
    // remote off none of it (this helper runs for nearby transfer alone).
    if helper.config.remote {
        let following = helper.clone();
        std::thread::spawn(move || {
            let mut last = None;
            // The makers' lighting programs seen running, by their numbers: others than last time, one has started or gone.
            let mut rivals: Vec<u32> = Vec::new();
            // When the lights are to be put back (see `put_back`): first as this helper starts with the PC.
            let mut put_back_at = Config::load().lights_raised.then(|| std::time::Instant::now() + AFRESH);
            loop {
                std::thread::sleep(FOLLOW);
                let now: Vec<u32> = actions::programs().into_iter().filter(|(_, name)| lights::RIVALS.iter().any(|rival| name.eq_ignore_ascii_case(rival))).map(|(number, _)| number).collect();
                // (One gone counts too: it does not give the lights back as it goes.)
                if now != rivals {
                    following.stale.store(true, std::sync::atomic::Ordering::SeqCst);
                    // (Lights that follow the heat are put in their colour again by the OpenRGB to come.)
                    last = None;
                    if Config::load().lights_raised {
                        put_back_at = Some(std::time::Instant::now() + AFRESH);
                    }
                }
                rivals = now;
                if put_back_at.is_some_and(|at| std::time::Instant::now() >= at) {
                    put_back_at = None;
                    following.put_back();
                }
                following.follow(&mut last);
            }
        });
        // Night ends at the first touch of the mouse or the keyboard; and so do
        // screens this helper keeps dark with their signal kept (Windows, which
        // has them as on, wakes nothing).
        let watching = helper.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(WATCH);
            let woken = display::woken();
            match woken {
                display::Woken::No => continue,
                // A report that said nothing is no touch: the screens Windows put on at it are put out again.
                display::Woken::ByNothing => {
                    journal::note(format!("the screens came on with nothing touched ({}): put out again", display::last_touch()));
                    let _ = display::sleep();
                    continue;
                }
                display::Woken::Touched => journal::note(format!("touched while the screens were out ({})", display::last_touch())),
                display::Woken::Unasked => journal::note(format!("the screens came on unasked, nothing heard from ({}): left on", display::last_touch())),
            }
            let night = watching.state.lock().unwrap().night.is_some();
            if night {
                let _ = watching.night(false);
            } else {
                watching.screens_on();
            }
        });
    }
    for stream in listener.incoming().flatten() {
        // (One for whom there is no room, or no thread to be had, is hung up on.)
        let Some(caller) = stream.peer_addr().ok().and_then(|from| helper.room(from.ip())) else { continue };
        let helper = helper.clone();
        let _ = std::thread::Builder::new().spawn(move || {
            let _caller = caller;
            let _ = helper.serve(stream);
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    /// OpenRGB run by the sensors starts with this helper: that it has its lights dark, or none yet, is not how they
    /// are to be. One running as this helper starts otherwise is the user's, and says how they are.
    #[test]
    fn lights_at_start_are_as_last_set_when_the_sensors_run_openrgb() {
        assert_eq!(super::lit_at_start(true, Some(true), || Some(false)), Some(true));
        assert_eq!(super::lit_at_start(true, None, || Some(false)), Some(false));
        assert_eq!(super::lit_at_start(false, Some(true), || Some(false)), Some(false));
        assert_eq!(super::lit_at_start(false, Some(true), || None), Some(true));
    }

    /// Lights there only where something would take them: OpenRGB with no devices (as it last set them) is none,
    /// not yet asked, it may have some.
    #[test]
    fn lights_there_only_where_something_takes_them() {
        assert!(!super::lights_there(false, false, true, Some(0)));
        assert!(super::lights_there(false, false, true, Some(2)));
        assert!(super::lights_there(false, false, true, None));
        assert!(super::lights_there(true, false, true, Some(0)));
        assert!(super::lights_there(false, true, false, None));
        assert!(!super::lights_there(false, false, false, None));
    }

    /// Lights taken by none of what sets them are not set: the phone is told, not shown them as changed.
    #[test]
    fn lights_taken_by_none_are_not_set() {
        assert!(super::lights_taken(true, false, false, Ok(0)).is_err());
        assert!(super::lights_taken(true, false, false, Ok(3)).is_ok());
        assert!(super::lights_taken(false, true, false, Ok(0)).is_ok());
        assert!(super::lights_taken(true, false, true, Err("no server".into())).is_ok());
        assert_eq!(super::lights_taken(true, false, false, Err("no server".into())), Err("no server".into()));
    }

    #[test]
    fn text_goes_through_a_link_and_back() {
        for text in ["DESKTOP-9950X", "书房 的电脑", "SM-F971U1", "100%"] {
            assert_eq!(super::unescaped(&super::escaped(text)), text);
        }
        assert_eq!(super::escaped("a b"), "a%20b");
        // What is no number stays as it is.
        assert_eq!(super::unescaped("%zz%4"), "%zz%4");
    }

    #[test]
    fn the_heat_goes_from_blue_to_red() {
        assert_eq!(super::heat(20), "0080FF");
        assert_eq!(super::heat(40), "0080FF");
        assert_eq!(super::heat(85), "FF0000");
        assert_eq!(super::heat(120), "FF0000");
        // About half way: green.
        assert_eq!(super::heat(62), "36FF00");
    }
}
