//! The PC's lights, through OpenRGB: its SDK server on this machine is asked
//! to put each lighting device out, or to light it as the configuration has
//! it. OpenRGB is started for that if it is not running, and needs no
//! administrator rights for the devices it reaches over USB and the
//! graphics card's own bus.
//!
//! The protocol is OpenRGB's network protocol, spoken at version 3: a header
//! ("ORGB", device, packet, size) and a body, all little-endian.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::config::{Config, Effect, Light};

const SERVER: ([u8; 4], u16) = ([127, 0, 0, 1], 6742);
/// The protocol version asked for: modes have a brightness; zones have no
/// segments yet.
const VERSION: u32 = 3;
/// How long OpenRGB is given to find the devices after it is started.
const STARTING: Duration = Duration::from_secs(40);
/// How long any one answer may take.
const ANSWER: Duration = Duration::from_secs(8);
/// How often a device is asked to be as wanted before it is left as it
/// is, and how long it is given each time before it is read back.
const TRIES: usize = 4;
const SETTLING: Duration = Duration::from_millis(400);

const CONTROLLER_COUNT: u32 = 0;
const CONTROLLER_DATA: u32 = 1;
const PROTOCOL_VERSION: u32 = 40;
const CLIENT_NAME: u32 = 50;
const RESIZE_ZONE: u32 = 1000;
const UPDATE_LEDS: u32 = 1050;
const UPDATE_MODE: u32 = 1101;
/// A mode's colour mode that shows the colours of the device's LEDs.
const COLORS_PER_LED: u32 = 1;

/// A mode of a device, as the server describes it: its name, and all of it
/// as it was sent (handed back as it is to choose it), with where its
/// colours are in that.
#[derive(Clone, Debug, PartialEq)]
pub struct Mode {
    pub name: String,
    raw: Vec<u8>,
    /// Where in `raw` its colours are (their count, then each).
    colors_at: usize,
    /// Where in `raw` its speed is, and its range: its slowest may be the larger number.
    speed_at: usize,
    slowest: u32,
    fastest: u32,
    /// It shows the colours of the device's LEDs, not any of its own.
    per_led: bool,
}

impl Mode {
    /// The mode at `share` of its range of speeds, from its slowest (0) to its fastest (1).
    fn at_pace(&self, share: f64) -> Mode {
        let mut mode = self.clone();
        let speed = (self.slowest as f64 + (self.fastest as f64 - self.slowest as f64) * share).round() as u32;
        mode.raw[self.speed_at..self.speed_at + 4].copy_from_slice(&speed.to_le_bytes());
        mode
    }

    /// The mode in `colour` (0x00BBGGRR): each colour it has, that one. A
    /// mode with none of its own (every colour in turn, say) is as it is.
    fn in_color(&self, colour: u32) -> Mode {
        let mut mode = self.clone();
        for chunk in mode.raw[self.colors_at + 2..].chunks_exact_mut(4) {
            chunk.copy_from_slice(&colour.to_le_bytes());
        }
        mode
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Zone {
    pub name: String,
    pub leds_max: u32,
    pub leds: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    pub name: String,
    pub modes: Vec<Mode>,
    /// The mode it is in, as its place among them.
    pub active: usize,
    pub zones: Vec<Zone>,
    pub leds: usize,
    /// Each LED's colour as the server has it (0x00BBGGRR).
    pub colors: Vec<u32>,
}

impl Device {
    /// Whether it is lit: not in its "off" mode, and, in a mode that shows
    /// the colours it is sent, not all black.
    pub fn lit(&self) -> bool {
        let mode = self.modes.get(self.active).map_or("", |mode| mode.name.as_str());
        let painted = mode.eq_ignore_ascii_case("Direct") || mode.eq_ignore_ascii_case("Static");
        !mode.eq_ignore_ascii_case("Off") && !(painted && self.colors.iter().all(|color| color & 0x00FF_FFFF == 0))
    }
}

/// A reader over a packet's body.
struct Body<'a>(&'a [u8]);

impl<'a> Body<'a> {
    fn take(&mut self, count: usize) -> Option<&'a [u8]> {
        let (taken, rest) = self.0.split_at_checked(count)?;
        self.0 = rest;
        Some(taken)
    }
    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    /// A string: its length (with the zero that ends it), then its bytes.
    fn string(&mut self) -> Option<String> {
        let length = self.u16()? as usize;
        let bytes = self.take(length)?;
        Some(String::from_utf8_lossy(bytes.strip_suffix(&[0]).unwrap_or(bytes)).into_owned())
    }
}

/// A device from the body of a "controller data" answer (protocol 3).
pub fn device(body: &[u8]) -> Option<Device> {
    let mut b = Body(body);
    b.u32()?; // the body's size
    b.u32()?; // the device's type
    let name = b.string()?;
    // Its vendor, description, version, serial and location.
    for _ in 0..5 {
        b.string()?;
    }
    let count = b.u16()?;
    let active = b.u32()? as usize;
    let mut modes = Vec::new();
    for _ in 0..count {
        let whole = b.0;
        let name = b.string()?;
        // Its value and flags, speeds, brightnesses, colour counts, speed,
        // brightness, direction and colour mode.
        b.take(2 * 4)?;
        let (slowest, fastest) = (b.u32()?, b.u32()?);
        b.take(4 * 4)?;
        let speed_at = whole.len() - b.0.len();
        b.take(3 * 4)?;
        let per_led = b.u32()? == COLORS_PER_LED;
        let colors_at = whole.len() - b.0.len();
        let colors = b.u16()? as usize;
        b.take(colors * 4)?;
        modes.push(Mode { name, raw: whole[..whole.len() - b.0.len()].to_vec(), colors_at, speed_at, slowest, fastest, per_led });
    }
    let mut zones = Vec::new();
    for _ in 0..b.u16()? {
        let name = b.string()?;
        b.u32()?; // its type
        b.u32()?; // the fewest LEDs it may have
        let (leds_max, leds) = (b.u32()?, b.u32()?);
        let matrix = b.u16()? as usize;
        b.take(matrix)?;
        zones.push(Zone { name, leds_max, leds });
    }
    let leds = b.u16()? as usize;
    for _ in 0..leds {
        b.string()?;
        b.u32()?; // the LED's own value
    }
    let colors = (0..b.u16()?).map(|_| b.u32()).collect::<Option<Vec<u32>>>()?;
    Some(Device { name, modes, active, zones, leds, colors })
}

pub struct Client(TcpStream);

impl Client {
    /// The server on this machine, if it answers.
    fn connect() -> std::io::Result<Client> {
        let stream = TcpStream::connect_timeout(&SocketAddr::from(SERVER), Duration::from_secs(1))?;
        stream.set_read_timeout(Some(ANSWER))?;
        stream.set_write_timeout(Some(ANSWER))?;
        let mut client = Client(stream);
        client.send(0, CLIENT_NAME, b"Glance\0")?;
        client.send(0, PROTOCOL_VERSION, &VERSION.to_le_bytes())?;
        client.answer(PROTOCOL_VERSION)?;
        Ok(client)
    }

    fn send(&mut self, device: u32, packet: u32, body: &[u8]) -> std::io::Result<()> {
        let mut out = Vec::with_capacity(16 + body.len());
        out.extend_from_slice(b"ORGB");
        for word in [device, packet, body.len() as u32] {
            out.extend_from_slice(&word.to_le_bytes());
        }
        out.extend_from_slice(body);
        self.0.write_all(&out)
    }

    /// The body of the next answer to `packet` (what else the server says
    /// meanwhile, that its devices changed, is passed over).
    fn answer(&mut self, packet: u32) -> std::io::Result<Vec<u8>> {
        loop {
            let mut header = [0u8; 16];
            self.0.read_exact(&mut header)?;
            if &header[..4] != b"ORGB" {
                return Err(std::io::Error::other("not OpenRGB's answer"));
            }
            let word = |at: usize| u32::from_le_bytes(header[at..at + 4].try_into().unwrap());
            // No answer of the protocol's is near this large.
            let size = word(12) as usize;
            if size > 16 << 20 {
                return Err(std::io::Error::other("an answer too large"));
            }
            let mut body = vec![0u8; size];
            self.0.read_exact(&mut body)?;
            if word(8) == packet {
                return Ok(body);
            }
        }
    }

    fn devices(&mut self) -> std::io::Result<Vec<Device>> {
        self.send(0, CONTROLLER_COUNT, &[])?;
        let count = self.answer(CONTROLLER_COUNT)?;
        let count = u32::from_le_bytes(count.get(..4).and_then(|c| c.try_into().ok()).ok_or_else(|| std::io::Error::other("no count"))?);
        (0..count)
            .map(|index| {
                self.send(index, CONTROLLER_DATA, &VERSION.to_le_bytes())?;
                device(&self.answer(CONTROLLER_DATA)?).ok_or_else(|| std::io::Error::other("a device not understood"))
            })
            .collect()
    }

    /// Puts device `index` in `mode`: the mode handed back as the server
    /// described it.
    fn set_mode(&mut self, index: u32, at: usize, mode: &Mode) -> std::io::Result<()> {
        let mut body = Vec::new();
        body.extend_from_slice(&((8 + mode.raw.len()) as u32).to_le_bytes());
        body.extend_from_slice(&(at as u32).to_le_bytes());
        body.extend_from_slice(&mode.raw);
        self.send(index, UPDATE_MODE, &body)
    }

    /// The LEDs of device `index`, each in its colour (0x00BBGGRR).
    fn set_colors(&mut self, index: u32, colors: &[u32]) -> std::io::Result<()> {
        let mut body = Vec::new();
        body.extend_from_slice(&((4 + 2 + 4 * colors.len()) as u32).to_le_bytes());
        body.extend_from_slice(&(colors.len() as u16).to_le_bytes());
        for color in colors {
            body.extend_from_slice(&color.to_le_bytes());
        }
        self.send(index, UPDATE_LEDS, &body)
    }
}

/// Where OpenRGB is: where the configuration says, where its installer
/// puts it, or where this helper fetched it to.
pub fn program(config: &Config) -> Option<PathBuf> {
    if config.lights_left_alone {
        return None;
    }
    let installed = std::env::var_os("ProgramFiles").map(|files| Path::new(&files).join("OpenRGB").join("OpenRGB.exe"));
    config.openrgb.clone().into_iter().chain(installed).find(|path| path.is_file()).or_else(crate::fetch::fetched)
}

/// CREATE_NO_WINDOW: no console for a program started without one.
const NO_WINDOW: u32 = 0x0800_0000;

/// The server, started if it is not running: as a server alone (no window,
/// none of its warnings shown, a third of the memory), and without its
/// trying to reach a server itself.
fn server(config: &Config) -> std::io::Result<Client> {
    use std::os::windows::process::CommandExt;
    if let Ok(client) = Client::connect() {
        return Ok(client);
    }
    // (One Glance's sensors run is theirs to start: it is only waited for, as one started here is.)
    if !Config::load().lights_raised {
        let program = program(config).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "OpenRGB is not installed"))?;
        let started = std::process::Command::new(&program).args(["--server", "--noautoconnect"]).current_dir(program.parent().unwrap()).creation_flags(NO_WINDOW).spawn()?.id();
        let _ = Config::change(|config| config.openrgb_started = Some(started));
    }
    found_all()
}

/// Waits for an OpenRGB being started to have its devices: it listens before it has found them, and finds them one
/// after another. Asked until it has some, and as many twice running.
pub fn found_all() -> std::io::Result<Client> {
    let since = Instant::now();
    let mut before = 0;
    loop {
        std::thread::sleep(Duration::from_millis(500));
        if let Ok(mut client) = Client::connect() {
            let found = client.devices().map_or(0, |devices| devices.len());
            if (found > 0 && found == before) || since.elapsed() > STARTING {
                return Ok(client);
            }
            before = found;
        } else if since.elapsed() > STARTING {
            return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "OpenRGB did not start"));
        }
    }
}

/// The programs of the makers' own that take a board's lights for
/// themselves once they start: an OpenRGB running since before can set
/// those lights no more (it goes on believing it does), one started after
/// can, whether or not the maker's program is still there. (As seen on a
/// Gigabyte board, by a camera on the case: lights set before, during and
/// after Gigabyte Control Center, and after OpenRGB was started afresh.)
pub const RIVALS: [&str; 1] = ["GCC.exe"];

/// Stops the OpenRGB this helper started, so that the next asking starts
/// one afresh; whether there was one of its own to stop.
pub fn afresh() -> bool {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{OpenProcess, TerminateProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE};
    let Some(started) = Config::load().openrgb_started else { return false };
    // (A number is given again to another program in time: it is OpenRGB's only while OpenRGB has it.)
    if !crate::actions::programs().iter().any(|(number, name)| *number == started && name.eq_ignore_ascii_case("OpenRGB.exe")) {
        return false;
    }
    unsafe {
        let Ok(program) = OpenProcess(PROCESS_TERMINATE | PROCESS_SYNCHRONIZE, false, started) else { return false };
        let stopped = TerminateProcess(program, 0).is_ok();
        WaitForSingleObject(program, 5000);
        let _ = CloseHandle(program);
        stopped
    }
}

fn rgb(hex: &str) -> Option<(u8, u8, u8)> {
    let value = u32::from_str_radix(hex, 16).ok().filter(|_| hex.len() == 6)?;
    Some(((value >> 16) as u8, (value >> 8) as u8, value as u8))
}

/// A device's modes that go through every colour by themselves, by their
/// names as OpenRGB's drivers have them, the likeliest first.
const RAINBOWS: [&str; 6] = ["Rainbow Wave", "Rainbow", "Spectrum Cycle", "Color Cycle", "Wave", "Wave 2"];

/// The lights put out, or lit as `light` has them: every device in that
/// colour and doing that, as nearly as it can (one with no such mode of
/// its own, steady in the colour). Returns how many devices there are.
pub fn set(config: &Config, on: bool, light: &Light) -> std::io::Result<usize> {
    // (Begun again, a few times at most, should the devices there are change under it.)
    for _ in 0..TRIES {
        if let Some(devices) = set_those_there(config, on, light)? {
            return Ok(devices);
        }
    }
    Err(std::io::Error::other("the lighting devices keep changing"))
}

/// The breathing of a device among others, as near theirs as it can be: a mode showing its LEDs' colours (the memory's)
/// breathes far more slowly than the rest even at its fastest, so it goes at that, and the rest, where there is one
/// such among them, at their slowest. With none, each goes as it would.
fn paced(mode: &Mode, one_is_slow: bool) -> Mode {
    match (mode.per_led, one_is_slow) {
        (true, _) => mode.at_pace(1.0),
        (false, true) => mode.at_pace(0.0),
        (false, false) => mode.clone(),
    }
}

/// `set`, for the devices there are as it begins: none, if by the time
/// they are read back they are others (OpenRGB still finding them, or one
/// plugged in or out).
fn set_those_there(config: &Config, on: bool, light: &Light) -> std::io::Result<Option<usize>> {
    let mut client = server(config)?;
    let mut devices = client.devices()?;
    // A header for addressable strips that OpenRGB has no LEDs for sends
    // nothing: given some, so that what is plugged into it is reached.
    let mut resized = false;
    for (index, device) in devices.iter().enumerate() {
        for (zone, of) in device.zones.iter().enumerate() {
            if of.leds == 0 && of.leds_max > 0 {
                let mut body = (zone as u32).to_le_bytes().to_vec();
                body.extend_from_slice(&config.strip_leds.min(of.leds_max).to_le_bytes());
                client.send(index as u32, RESIZE_ZONE, &body)?;
                resized = true;
            }
        }
    }
    if resized {
        devices = client.devices()?;
    }
    let (r, g, b) = rgb(&light.color).unwrap_or((255, 255, 255));
    // As bright as asked: each of its three lights that share of itself.
    let dimmed = |part: u8| ((part as u32 * light.brightness.min(100) as u32 + 50) / 100) as u8;
    let (r, g, b) = (dimmed(r), dimmed(g), dimmed(b));
    let colour = u32::from_le_bytes([r, g, b, 0]);
    let one_is_slow = devices.iter().any(|device| device.modes.iter().any(|mode| mode.per_led && mode.name.eq_ignore_ascii_case("Breathing")));
    // What each device is to be: a mode (as it is sent), and, in the mode
    // that shows the colours it is sent, those colours.
    let wanted: Vec<(Option<(usize, Mode)>, Option<Vec<u32>>)> = devices
        .iter()
        .map(|device| {
            let named = |name: &str| device.modes.iter().position(|mode| mode.name.eq_ignore_ascii_case(name)).map(|at| (at, device.modes[at].clone()));
            let paint = |colour: u32| (named("Direct").or_else(|| named("Static")), Some(vec![colour; device.leds]));
            if !on {
                // Out: the device's own "off", where it has one.
                return named("Off").map_or_else(|| paint(0), |off| (Some(off), None));
            }
            let own = match light.effect {
                // (Following the heat, the colour is what changes: whoever
                // asks gives the colour of the moment.)
                Effect::Steady | Effect::Heat => None,
                Effect::Breathing => named("Breathing").map(|(at, mode)| (at, paced(&mode, one_is_slow).in_color(colour))),
                Effect::Rainbow => RAINBOWS.iter().find_map(|name| named(name)),
            };
            // (One that shows its LEDs' colours is sent them too: else it shows what they were last.)
            own.map_or_else(|| paint(colour), |(at, mode)| {
                let colors = mode.per_led.then(|| vec![colour; device.leds]);
                (Some((at, mode)), colors)
            })
        })
        .collect();
    // Asked, then read back: a device that is not in the mode asked for
    // (what is sent over its bus does not always arrive) is asked again.
    for attempt in 0..TRIES {
        let behind: Vec<usize> = (0..devices.len()).filter(|at| attempt == 0 || wanted[*at].0.as_ref().is_some_and(|(mode, _)| devices[*at].active != *mode)).collect();
        // (Putting out, one asking is enough: what has not gone out by its
        // own "off" is painted dark below, which it takes at once.)
        if behind.is_empty() || (!on && attempt > 0) {
            break;
        }
        for at in behind {
            let (mode, colors) = &wanted[at];
            if let Some((index, mode)) = mode {
                client.set_mode(at as u32, *index, mode)?;
            }
            if let Some(colors) = colors {
                client.set_colors(at as u32, colors)?;
            }
        }
        std::thread::sleep(SETTLING);
        let named: Vec<String> = devices.iter().map(|device| device.name.clone()).collect();
        devices = client.devices()?;
        if devices.iter().map(|device| &device.name).ne(named.iter()) {
            return Ok(None);
        }
    }
    // Put out, a device that would not go to its own "off" (some take a
    // colour but not a change of mode) is painted with no light instead.
    if !on {
        let mut painted = false;
        for (at, device) in devices.iter().enumerate().filter(|(_, device)| device.lit()) {
            let paintable = ["Direct", "Static"].iter().find_map(|name| device.modes.iter().position(|mode| mode.name.eq_ignore_ascii_case(name)));
            if let Some(mode) = paintable {
                client.set_mode(at as u32, mode, &device.modes[mode])?;
                client.set_colors(at as u32, &vec![0; device.leds])?;
                painted = true;
            }
        }
        if painted {
            std::thread::sleep(SETTLING);
        }
    }
    Ok(Some(devices.len()))
}

/// Whether the lights are on: any device lit. None while OpenRGB is not
/// running (it is not started to be asked: how they are is then as they
/// were left).
pub fn lit() -> Option<bool> {
    Some(Client::connect().ok()?.devices().ok()?.iter().any(Device::lit))
}

/// The lighting devices as an OpenRGB running already has them: none while none runs.
pub fn known() -> Option<Vec<Device>> {
    Client::connect().ok()?.devices().ok()
}

/// The lighting devices, for choosing how each is lit.
pub fn devices(config: &Config) -> std::io::Result<Vec<Device>> {
    server(config)?.devices()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn string(text: &str) -> Vec<u8> {
        let mut out = ((text.len() + 1) as u16).to_le_bytes().to_vec();
        out.extend_from_slice(text.as_bytes());
        out.push(0);
        out
    }

    #[test]
    fn reads_a_device_as_the_server_describes_it() {
        let mode = |name: &str, colors: u16| {
            let mut out = string(name);
            out.extend_from_slice(&[0u8; 48]);
            out.extend_from_slice(&colors.to_le_bytes());
            out.extend(std::iter::repeat_n(0u8, colors as usize * 4));
            out
        };
        let zone = |name: &str, max: u32, leds: u32| {
            let mut out = string(name);
            for word in [1u32, 0, max, leds] {
                out.extend_from_slice(&word.to_le_bytes());
            }
            out.extend_from_slice(&0u16.to_le_bytes());
            out
        };
        let mut body = vec![0u8; 4];
        body.extend_from_slice(&2u32.to_le_bytes());
        for text in ["A card", "Its maker", "", "1.0", "", "I2C"] {
            body.extend(string(text));
        }
        body.extend_from_slice(&2u16.to_le_bytes());
        body.extend_from_slice(&0u32.to_le_bytes());
        let (off, wave) = (mode("Off", 0), mode("Rainbow Wave", 2));
        body.extend(off.iter().chain(&wave));
        body.extend_from_slice(&2u16.to_le_bytes());
        body.extend(zone("GPU", 1, 1));
        body.extend(zone("Strip", 300, 0));
        body.extend_from_slice(&1u16.to_le_bytes());
        body.extend(string("LED 1"));
        body.extend_from_slice(&0u32.to_le_bytes());
        body.extend_from_slice(&1u16.to_le_bytes());
        body.extend_from_slice(&0x0000_FF00u32.to_le_bytes());
        let read = device(&body).unwrap();
        assert_eq!(read.name, "A card");
        let plain = |name: &str, raw: Vec<u8>, colors_at: usize| Mode { name: name.into(), speed_at: colors_at - 16, slowest: 0, fastest: 0, per_led: false, colors_at, raw };
        assert_eq!(read.modes, [plain("Off", off.clone(), off.len() - 2), plain("Rainbow Wave", wave.clone(), wave.len() - 10)]);
        // A mode's colours, each made the one asked for; the rest of it as it was.
        let red = read.modes[1].in_color(0xFF);
        assert_eq!(&red.raw[wave.len() - 8..], [0xFF, 0, 0, 0, 0xFF, 0, 0, 0]);
        assert_eq!(red.raw[..wave.len() - 8], wave[..wave.len() - 8]);
        assert_eq!(read.modes[0].in_color(0xFF), read.modes[0]);
        assert_eq!(read.zones, [Zone { name: "GPU".into(), leds_max: 1, leds: 1 }, Zone { name: "Strip".into(), leds_max: 300, leds: 0 }]);
        assert_eq!((read.leds, read.active, read.colors.as_slice()), (1, 0, &[0x0000_FF00u32][..]));
        // In its "off" mode it is out, whatever its colours; in a mode of
        // its own, lit; painted black, out.
        assert!(!read.lit());
        assert!(Device { active: 1, ..read.clone() }.lit());
        let painted = Device { modes: vec![Mode { name: "Direct".into(), raw: Vec::new(), colors_at: 0, speed_at: 0, slowest: 0, fastest: 0, per_led: false }], active: 0, ..read.clone() };
        assert!(painted.lit() && !Device { colors: vec![0], ..painted }.lit());
        // Cut short, it is none.
        assert_eq!(device(&body[..body.len() - 3]), None);
        assert_eq!(rgb("ff8000"), Some((255, 128, 0)));
        assert_eq!(rgb("fff"), None);
    }

    /// A mode as the server describes it: `speeds` from slowest to fastest, at `speed`, showing `colour_mode`.
    fn breathing(speeds: (u32, u32), speed: u32, colour_mode: u32) -> Mode {
        let mut body = vec![0u8; 8];
        for _ in 0..6 {
            body.extend(string(""));
        }
        body.extend_from_slice(&1u16.to_le_bytes());
        body.extend_from_slice(&0u32.to_le_bytes());
        body.extend(string("Breathing"));
        for word in [0, 0, speeds.0, speeds.1, 0, 0, 0, 0, speed, 0, 0, colour_mode] {
            body.extend_from_slice(&word.to_le_bytes());
        }
        body.extend_from_slice(&0u16.to_le_bytes());
        // No zones, LEDs or colours.
        body.extend_from_slice(&[0u8; 6]);
        device(&body).unwrap().modes.remove(0)
    }

    fn speed(mode: &Mode) -> u32 {
        u32::from_le_bytes(mode.raw[mode.speed_at..mode.speed_at + 4].try_into().unwrap())
    }

    /// Its slowest may be the larger number (the memory's and the board's are, the card's is not).
    #[test]
    fn a_mode_goes_at_a_share_of_its_speeds_whichever_way_they_run() {
        assert_eq!(speed(&breathing((4, 0), 2, 1).at_pace(1.0)), 0);
        assert_eq!(speed(&breathing((9, 0), 4, 2).at_pace(0.0)), 9);
        assert_eq!(speed(&breathing((0, 2), 1, 2).at_pace(0.0)), 0);
        assert_eq!(speed(&breathing((0, 2), 1, 2).at_pace(1.0)), 2);
    }

    #[test]
    fn breathing_slows_the_rest_only_for_a_device_that_cannot_keep_up() {
        let (memory, board) = (breathing((4, 0), 2, COLORS_PER_LED), breathing((9, 0), 4, 2));
        assert!(memory.per_led && !board.per_led);
        assert_eq!(speed(&paced(&memory, true)), 0);
        assert_eq!(speed(&paced(&board, true)), 9);
        assert_eq!(speed(&paced(&board, false)), 4);
    }
}
