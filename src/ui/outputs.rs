//! The outputs and the programs' sound, opened beside the panel (or a widget) from the output its volume row names (see
//! `beside::Page::Sound`): the outputs the sound may play on, the one it plays on now ticked (another pressed, it plays
//! there from then on); then each program that plays on that one or did lately, how loud it is there, dragged along
//! its track as the panel's volume is, muted by its button, a line under its track as loud as it sounds this moment.
//! A program that does not sound now is drawn greyed, and may be set all the same. Windows' own sounds come last.
//!
//! The programs stay in the order they were first listed in while it is up (one that starts or stops sounding is not
//! moved from under the pointer); one come since goes after them, before Windows' own sounds.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::canvas::{Align, Canvas, Point};
use super::icons::Icon;
use super::text::Lang;
use super::theme::{Skin, Theme};
use super::view::{knob_reach, slider, LABEL, LABEL_GAP, MEDIA_GAP, MEDIA_ROW};
use crate::audio::{Form, Mixer, Via};

/// How wide it is (as a column of the panel), and the room in from its sides, over its first line and under its last.
pub const WIDTH: f32 = super::view::COLUMN_WIDTH;
const SIDE: f32 = 18.0;
const TOP: f32 = 10.0;
const BOTTOM: f32 = 14.0;
/// A group's heading, an output's row, a program's (its gap over it and its row), and the room between the groups.
const HEADING: f32 = 24.0;
const OUTPUT_ROW: f32 = 40.0;
const APP_ROW: f32 = MEDIA_GAP + MEDIA_ROW;
const GROUP_GAP: f32 = 4.0;
/// An output's round mark, and how far its row's plate reaches past the sides of what is listed.
const MARK: f32 = 28.0;
/// The ring of a wireless output's charge round its mark: across, and its line (clear of the mark).
const CHARGE_RING: (f32, f32) = (34.0, 2.2);
const ROW_OUT: f32 = 6.0;
/// A program's picture, and the room after it.
const FACE: f32 = 16.0;
const FACE_GAP: f32 = 8.0;
/// How far a program's level moves for each notch of the wheel over its track.
const WHEEL_STEP: f32 = 2.0;

/// What is pressed on it: an output (by its place among them), a program's track or its mute (by its place as shown).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Hit {
    Output(u16),
    Level(u16),
    Mute(u16),
}

/// What is kept of it while it is up: the programs' order (see the top) and the output it is of, a program's track
/// held (by its key), and its tracks' knobs as they ease (see `thumb::Dots`).
#[derive(Default)]
pub struct Sound {
    order: Vec<String>,
    order_of: Option<String>,
    held: Option<String>,
    pub dots: super::thumb::Dots<Hit>,
    /// What was drawn last (what is read, and how loud each program sounded, to the hundredth): drawn again only as
    /// that changes, or a knob eases.
    drawn: Option<Drawn>,
}

/// How tall it is with `outputs` outputs and `apps` programs listed (DIPs): none listed, a line saying so in their room.
pub fn height(outputs: usize, apps: usize) -> f32 {
    TOP + HEADING + OUTPUT_ROW * outputs as f32 + GROUP_GAP + HEADING + APP_ROW * apps.max(1) as f32 + BOTTOM
}

/// How tall it is for `mixer` (see `height`).
pub fn height_of(mixer: &Mixer) -> f32 {
    height(mixer.outputs.len(), mixer.apps.len())
}

/// The output's name rested on and left (see `Rest`): how long the pointer rests on it before the page opens (as a
/// card beside the dock does), and how long it may be off the name and the page and their way between before it closes.
pub const REST: Duration = Duration::from_millis(260);
pub const LEAVE: Duration = Duration::from_millis(300);

/// What the pointer on the output's name asks of the sound's page (see `Rest::follow`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Act {
    Open,
    Close,
    /// Nothing yet: looked at again then.
    Wait(Instant),
    Nothing,
}

/// The pointer on the output's name of a panel or a widget, as it rests there and leaves: the page opened as it has
/// rested, closed as it has been off the name, the page and their way between a moment (a hand wavering does not close
/// it). Pressed, the name opens or closes it at once, and resting on it then opens it no more till the pointer has left.
#[derive(Default)]
pub struct Rest {
    on_since: Option<Instant>,
    away_since: Option<Instant>,
    /// Opened or closed while the pointer is on the name: not opened by its resting there again till it leaves.
    held_off: bool,
}

impl Rest {
    /// The pointer is on the name (`on_name`) or not, where it keeps the page up (`holds`: see `beside::sound_holds`),
    /// the page `up` from here or not: what is to be done.
    pub fn follow(&mut self, on_name: bool, holds: bool, up: bool, now: Instant) -> Act {
        if !on_name {
            self.on_since = None;
            self.held_off = false;
        }
        if up {
            self.on_since = None;
            if on_name || holds {
                self.away_since = None;
                return Act::Nothing;
            }
            let since = *self.away_since.get_or_insert(now);
            if now >= since + LEAVE {
                self.away_since = None;
                return Act::Close;
            }
            return Act::Wait(since + LEAVE);
        }
        self.away_since = None;
        if !on_name || self.held_off {
            return Act::Nothing;
        }
        let since = *self.on_since.get_or_insert(now);
        if now >= since + REST {
            self.on_since = None;
            self.held_off = true;
            return Act::Open;
        }
        Act::Wait(since + REST)
    }

    /// When the page is due to open or close, the pointer staying where it is: looked at again then.
    pub fn due(&self) -> Option<Instant> {
        self.on_since.map(|since| since + REST).or(self.away_since.map(|since| since + LEAVE))
    }

    /// The name pressed: the page opened or closed by it, not by resting there again till the pointer leaves.
    pub fn pressed(&mut self) {
        self.held_off = true;
        self.on_since = None;
        self.away_since = None;
    }
}

/// How far down its first line is from its top, and how far in from its side what it lists is: what it is placed by,
/// level with the row it opened from (beside the top edge's panel, under it, in line with it).
pub const FIRST_LINE: f32 = TOP;
pub const FIRST_COLUMN: f32 = SIDE;

impl Sound {
    /// The programs of `mixer` in the order they show in (see the top), the order kept updated: on another output,
    /// begun again; at first, those sounding first.
    fn ordered<'a>(&mut self, mixer: &'a Mixer) -> Vec<&'a crate::audio::App> {
        if self.order_of != mixer.current {
            self.order_of = mixer.current.clone();
            self.order.clear();
        }
        if self.order.is_empty() {
            let mut first: Vec<&crate::audio::App> = mixer.apps.iter().collect();
            first.sort_by_key(|app| (app.system, !app.sounding));
            self.order = first.iter().map(|app| app.key.clone()).collect();
        }
        for app in &mixer.apps {
            if !self.order.contains(&app.key) {
                let at = if app.system { self.order.len() } else { self.order.iter().position(|key| key == crate::audio::SYSTEM).unwrap_or(self.order.len()) };
                self.order.insert(at, app.key.clone());
            }
        }
        self.order.iter().filter_map(|key| mixer.apps.iter().find(|app| app.key == *key)).collect()
    }

    /// Whether it is to be drawn again: what is read or how loud the programs sound changed since it was drawn, or a
    /// knob eases (stepped on to `now`, the pointer over `over`).
    pub fn changed(&mut self, now: Instant, over: Option<Hit>) -> bool {
        let held = self.held.as_ref().and_then(|key| self.order.iter().position(|of| of == key)).map(|at| Hit::Level(at as u16));
        let easing = self.dots.step(now, over.filter(|hit| matches!(hit, Hit::Level(_))), held);
        let now = (crate::audio::mixer(), sounded(&crate::audio::peaks()), crate::devices::latest());
        let changed = self.drawn.as_ref() != Some(&now);
        easing || changed
    }

    /// Drawn in `frame`, `width` wide, in `theme` and `lang`, the pointer at `pointer` (DIPs on it): what it shows, as
    /// now read; where each part is pressed. Whether a program's picture is not ready yet (drawn again soon).
    #[allow(clippy::too_many_arguments)]
    pub fn paint(&mut self, frame: &dyn Canvas, theme: &Theme, lang: Lang, width: f32, pointer: Option<(f32, f32)>, accent: super::canvas::Color, hits: &mut Vec<(f32, f32, f32, f32, Hit)>) -> bool {
        let mixer = crate::audio::mixer();
        let peaks = crate::audio::peaks();
        // (A wireless output's charge, where its device's is read: see `devices`.)
        let devices = crate::devices::latest();
        let inner = width - 2.0 * SIDE;
        let over = |(x, y, w, h): (f32, f32, f32, f32)| pointer.is_some_and(|(px, py)| px >= x && px < x + w && py >= y && py < y + h);
        // (Its plates as the page beside draws its own: round-ended in glass.)
        let round = |tall: f32| if theme.skin == Skin::Glass { tall / 2.0 } else { theme.control_radius.max(4.0).min(tall / 2.0) };
        let mut y = TOP;
        let heading = |text: &str, y: f32| frame.text(text, theme.small, theme.text3, SIDE, frame.line_in(theme.small, y + 4.0, HEADING - 4.0), inner, Align::Start);

        // The outputs, the one played on ticked, on its round mark in the accent.
        heading(lang.pick("输出设备", "Output devices"), y);
        y += HEADING;
        for (at, output) in mixer.outputs.iter().enumerate() {
            let place = (SIDE - ROW_OUT, y, inner + 2.0 * ROW_OUT, OUTPUT_ROW);
            let current = mixer.current.as_deref() == Some(output.id.as_str());
            if over(place) {
                frame.fill_rounded(theme.hover, place.0, place.1, place.2, place.3, round(OUTPUT_ROW));
            }
            let centre = Point { x: SIDE + MARK / 2.0, y: y + OUTPUT_ROW / 2.0 };
            frame.fill_circle(if current { accent } else { theme.hover }, centre, MARK / 2.0);
            let charged = crate::devices::in_container(&devices, output.container.as_deref());
            if let Some(device) = charged {
                super::view::device_charge(frame, theme, theme.ink("devices"), device, centre, CHARGE_RING);
            }
            let on_mark = if current { if theme.skin == Skin::Paper { theme.paper } else { super::canvas::Color::hex(0xFFFFFF, 1.0) } } else { theme.text2 };
            frame.icon(icon_of(output.form), centre, 15.0, on_mark);
            let words_x = SIDE + MARK + 10.0;
            let tick = 15.0;
            let words_w = SIDE + inner - tick - 8.0 - words_x;
            frame.text(&output.name, theme.body, theme.text, words_x, frame.line_in(theme.body, y + 4.0, 16.0), words_w, Align::Start);
            let via = match &output.via {
                Via::Bluetooth => lang.pick("蓝牙", "Bluetooth").to_string(),
                Via::Usb => "USB".to_string(),
                Via::Card(card) => card.clone(),
            };
            let small_y = frame.line_in(theme.small, y + 20.0, 16.0);
            frame.text(&via, theme.small, theme.text3, words_x, small_y, words_w, Align::Start);
            // Its charge after it.
            if let Some(device) = charged {
                let after = words_x + frame.measure(&via, theme.small) + 8.0;
                frame.text(&format!("{}%", device.level), theme.small, theme.text2, after, small_y, (words_x + words_w - after).max(0.0), Align::Start);
            }
            if current {
                frame.icon(Icon::Check, Point { x: SIDE + inner - tick / 2.0, y: centre.y }, tick, accent);
            }
            hits.push((place.0, place.1, place.2, place.3, Hit::Output(at as u16)));
            y += OUTPUT_ROW;
        }

        // The programs on it, each as the panel's volume is: its name, its track, its figure, its mute.
        y += GROUP_GAP;
        heading(lang.pick("应用音量", "App volume"), y);
        y += HEADING;
        let apps = self.ordered(&mixer);
        if apps.is_empty() {
            let said = if mixer.read { lang.pick("暂无应用播放声音", "No apps are playing sound") } else { lang.pick("正在读取", "Reading") };
            frame.text(said, theme.small, theme.text3, SIDE, frame.line_top(theme.small, y + MEDIA_GAP + MEDIA_ROW / 2.0), inner, Align::Start);
        }
        let ink = theme.ink("time").trace;
        let said_width = frame.measure("100%", theme.small);
        let mute_x = SIDE + inner - MEDIA_ROW;
        let plot_left = SIDE + LABEL + LABEL_GAP;
        let track = (mute_x - 6.0 - said_width - LABEL_GAP - knob_reach(theme) - plot_left).max(0.0);
        let mut waiting = false;
        for (at, app) in apps.iter().enumerate() {
            let (row, middle) = (y + MEDIA_GAP, y + MEDIA_GAP + MEDIA_ROW / 2.0);
            let quiet = !app.sounding && !app.system;
            // Its picture (Windows' own sounds: the volume's mark), greyed where it does not sound now; its name.
            let face = (SIDE, middle - FACE / 2.0);
            let faded = if quiet { 0.6 } else { 1.0 };
            match app.file.as_ref().filter(|_| !app.system) {
                Some(file) => match crate::station::thumbs::icon(file, super::view::APP_PIXELS) {
                    Some(picture) => frame.picture(&picture, face.0, face.1, FACE, FACE, 4.0, faded),
                    None => {
                        waiting = true;
                        frame.icon(Icon::AppWindow, Point { x: face.0 + FACE / 2.0, y: middle }, 13.0, theme.text3);
                    }
                },
                None => frame.icon(if app.system { Icon::Volume } else { Icon::AppWindow }, Point { x: face.0 + FACE / 2.0, y: middle }, 13.0, if quiet { theme.text3 } else { theme.text2 }),
            }
            let name = if app.system { lang.pick("系统声音", "System sounds") } else { app.name.as_str() };
            frame.text(name, theme.small, if quiet { theme.text3 } else { theme.text2 }, SIDE + FACE + FACE_GAP, frame.line_top(theme.small, middle), LABEL - FACE - FACE_GAP, Align::Start);
            // Its track (in the panel's ink for the volume, grey where it is muted or does not sound), and under it how
            // loud it sounds this moment.
            // (Every program's track as it can be moved, as Windows' own mixer draws it: grey only where it is muted.)
            let colour = if app.muted { theme.text3 } else { ink };
            let level_hit = Hit::Level(at as u16);
            slider(frame, theme, colour, plot_left, middle, track, app.level as f32 / 100.0, Some(self.dots.of(level_hit)));
            if let Some(peak) = peaks.get(&app.key).filter(|_| app.sounding && !app.muted) {
                let under = middle + theme.meter_height / 2.0 + 3.0;
                frame.fill_rounded(colour.alpha(0.5), plot_left, under, track * peak.clamp(0.0, 1.0), 2.0, 1.0);
            }
            hits.push((plot_left, row, track, MEDIA_ROW, level_hit));
            let said = format!("{}%", app.level);
            frame.text(&said, theme.small, if app.muted { theme.text3 } else { theme.text2 }, mute_x - 6.0 - said_width, frame.line_top(theme.small, middle), said_width, Align::End);
            // Its mute, as the panel's: on a plate under the pointer, or while it is muted.
            let mute = (mute_x, row, MEDIA_ROW, MEDIA_ROW);
            let lit = app.muted || over(mute);
            if lit {
                frame.fill_rounded(theme.hover, mute.0, mute.1, mute.2, mute.3, round(MEDIA_ROW));
            }
            frame.icon(if app.muted { Icon::Muted } else { Icon::Volume }, Point { x: mute_x + MEDIA_ROW / 2.0, y: middle }, 15.0, if lit { theme.text } else { theme.text2 });
            hits.push((mute.0, mute.1, mute.2, mute.3, Hit::Mute(at as u16)));
            y += APP_ROW;
        }
        self.drawn = Some((mixer, sounded(&peaks), devices));
        waiting
    }

    /// A press on `hit` at `x` (DIPs) begun: a track held, set to where it is pressed (see `drag`).
    pub fn hold(&mut self, hit: Hit, x: f32, hits: &[(f32, f32, f32, f32, Hit)]) {
        if let Hit::Level(at) = hit {
            self.held = self.order_key(at);
            self.drag(x, hits);
        }
    }

    /// The track held dragged to `x` (DIPs): its program as loud as that is along it.
    pub fn drag(&mut self, x: f32, hits: &[(f32, f32, f32, f32, Hit)]) {
        let Some(key) = self.held.clone() else { return };
        let Some(at) = self.order.iter().position(|of| *of == key) else { return };
        let Some((from, _, track, _, _)) = hits.iter().find(|hit| hit.4 == Hit::Level(at as u16)) else { return };
        let level = if *track > 0.0 { ((x - from) / track).clamp(0.0, 1.0) * 100.0 } else { 0.0 };
        crate::audio::set_app(&key, Some(level.round() as u8), None);
    }

    /// Whether a track is held.
    pub fn holding(&self) -> bool {
        self.held.is_some()
    }

    /// The track held let go.
    pub fn let_go(&mut self) {
        self.held = None;
    }

    /// A press on `hit` let go on it: done.
    pub fn pressed(&mut self, hit: Hit) {
        match hit {
            Hit::Output(at) => {
                if let Some(output) = crate::audio::mixer().outputs.get(at as usize) {
                    crate::audio::choose(&output.id);
                }
            }
            Hit::Mute(at) => {
                if let Some(key) = self.order_key(at) {
                    let muted = crate::audio::mixer().apps.iter().find(|app| app.key == key).is_some_and(|app| app.muted);
                    crate::audio::set_app(&key, None, Some(!muted));
                }
            }
            Hit::Level(_) => {}
        }
    }

    /// The wheel turned `turns` notches over `hit`: a program's level moved by it.
    pub fn wheel(&mut self, hit: Hit, turns: f32) {
        let Hit::Level(at) = hit else { return };
        let Some(key) = self.order_key(at) else { return };
        let Some(level) = crate::audio::mixer().apps.iter().find(|app| app.key == key).map(|app| app.level) else { return };
        let level = (level as f32 + turns * WHEEL_STEP).round().clamp(0.0, 100.0) as u8;
        crate::audio::set_app(&key, Some(level), None);
    }

    /// The program shown at `at`, by its key.
    fn order_key(&self, at: u16) -> Option<String> {
        self.order.get(at as usize).cloned()
    }
}

/// How loud each program sounds, to the hundredth (as it is drawn: what changes it, it is drawn again for).
fn sounded(peaks: &HashMap<String, f32>) -> Vec<(String, u8)> {
    let mut sounded: Vec<(String, u8)> = peaks.iter().map(|(key, peak)| (key.clone(), (peak.clamp(0.0, 1.0) * 100.0).round() as u8)).collect();
    sounded.sort();
    sounded
}

/// What the plate was last drawn from: what is read, how loud each program sounded, the wireless devices' charges.
type Drawn = (Mixer, Vec<(String, u8)>, Vec<crate::reading::DeviceSample>);

/// What an output is drawn as.
pub fn icon_of(form: Form) -> Icon {
    match form {
        Form::Speakers => Icon::Speaker,
        Form::Headphones => Icon::Headphones,
        Form::Headset => Icon::Headset,
        Form::Handset => Icon::Smartphone,
        Form::Screen => Icon::Monitor,
        Form::Line => Icon::AudioOut,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The page opens once the pointer has rested on the output's name, and not as it only passes; it stays while the
    /// pointer is on the name, on the page or on its way between, and closes once it has been off them a moment (back
    /// within that, it stays). Pressed, the name opens or closes it at once, and resting there then opens it no more
    /// till the pointer has left the name.
    #[test]
    fn the_page_opens_as_the_name_is_rested_on_and_closes_as_it_is_left() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut rest = Rest::default();
        // (Passing over it.)
        assert_eq!(rest.follow(true, false, false, at(0)), Act::Wait(at(0) + REST));
        assert_eq!(rest.follow(false, false, false, at(100)), Act::Nothing);
        assert_eq!(rest.follow(true, false, false, at(200)), Act::Wait(at(200) + REST));
        // (Rested on.)
        assert_eq!(rest.follow(true, false, false, at(200) + REST), Act::Open);
        // (Off the name, on its way and on the page: kept; off them all a while: closed, unless it came back.)
        assert_eq!(rest.follow(false, true, true, at(600)), Act::Nothing);
        assert_eq!(rest.follow(false, false, true, at(700)), Act::Wait(at(700) + LEAVE));
        assert_eq!(rest.follow(false, true, true, at(800)), Act::Nothing);
        assert_eq!(rest.follow(false, false, true, at(900)), Act::Wait(at(900) + LEAVE));
        assert_eq!(rest.follow(false, false, true, at(900) + LEAVE), Act::Close);
        // (Pressed to close as it rests there: not opened again by resting, till it has left and rested again.)
        let mut rest = Rest::default();
        rest.pressed();
        assert_eq!(rest.follow(true, false, false, at(0)), Act::Nothing);
        assert_eq!(rest.follow(true, false, false, at(1000)), Act::Nothing);
        assert_eq!(rest.follow(false, false, false, at(1100)), Act::Nothing);
        assert_eq!(rest.follow(true, false, false, at(1200)), Act::Wait(at(1200) + REST));
        assert_eq!(rest.follow(true, false, false, at(1200) + REST), Act::Open);
    }
    use crate::audio::App;

    fn app(key: &str, sounding: bool, system: bool) -> App {
        App { key: key.into(), name: key.into(), file: None, level: 50, muted: false, sounding, system }
    }

    #[test]
    fn what_sounds_comes_first_and_stays_in_place() {
        let mut sound = Sound::default();
        let mut mixer = Mixer { outputs: Vec::new(), current: Some("out".into()), apps: vec![app("a", false, false), app(crate::audio::SYSTEM, false, true), app("b", true, false)], read: true };
        let keys = |apps: Vec<&App>| apps.iter().map(|app| app.key.clone()).collect::<Vec<_>>();
        assert_eq!(keys(sound.ordered(&mixer)), ["b", "a", "system"]);
        // (One stops sounding, another starts: nothing moves.)
        mixer.apps = vec![app("a", true, false), app(crate::audio::SYSTEM, false, true), app("b", false, false)];
        assert_eq!(keys(sound.ordered(&mixer)), ["b", "a", "system"]);
        // (One come since: after them, before Windows' own sounds.)
        mixer.apps.push(app("c", true, false));
        assert_eq!(keys(sound.ordered(&mixer)), ["b", "a", "c", "system"]);
        // (One gone: not shown.)
        mixer.apps.retain(|app| app.key != "a");
        assert_eq!(keys(sound.ordered(&mixer)), ["b", "c", "system"]);
        // (Another output: begun again.)
        mixer.current = Some("other".into());
        mixer.apps = vec![app("c", false, false), app("d", true, false)];
        assert_eq!(keys(sound.ordered(&mixer)), ["d", "c"]);
    }

    #[test]
    fn it_is_as_tall_as_what_it_lists() {
        assert_eq!(height(2, 3) - height(1, 3), OUTPUT_ROW);
        assert_eq!(height(2, 3) - height(2, 2), APP_ROW);
        // (None listed: a line saying so in a program's room.)
        assert_eq!(height(2, 0), height(2, 1));
    }
}
