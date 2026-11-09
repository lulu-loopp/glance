//! The music full screen: over the whole of a screen, the smoke (see `ui::smoke`) flowing in the cover's colours, and
//! the line sung now large at the left, each character coming up out of a blur as it is sung, lighting and swelling
//! with the music, the whole line starting a little at each beat; the next line small under it. The controls come
//! as the pointer moves and go a while after it rests (and the pointer with them): the track short above the keys in
//! the middle, the volume and the way out at the right. Esc, or a double click, leaves it.
//!
//! Its window takes the keyboard (for Esc) and is drawn at every frame while it is up.

use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, POINT, RECT};

use crate::lyrics::Line;
use crate::playing::{Follow, Sung};
use crate::ui::canvas::{Align, Canvas, Color, Family, Font, Point};
use crate::ui::gfx::{rect, Frame, Gfx, Surface};
use crate::ui::icons::Icon;
use crate::ui::smoke::{self, Smoke};
use crate::ui::window::Window;

/// How far past either end of the track a press still takes it (DIPs): at its end, as at the end of what is drawn.
const TRACK_REACH: f32 = 8.0;

/// Its smoke's colours with no cover (as it begins, and for a track with none).
const TINT: [f32; 3] = [0.40, 0.95, 0.88];
const TINT2: [f32; 3] = [0.84, 0.97, 0.92];
/// How long the controls stay after the pointer rests.
const AWAKE: Duration = Duration::from_millis(1200);
/// How long they take to come or go.
const CONTROLS_FADE: f32 = 0.35;
/// Two presses no further apart than this are a double click.
const DOUBLE: Duration = Duration::from_millis(400);
/// How long the line before takes to leave as the next comes (seconds).
const LEAVING: f32 = 0.7;

/// Each character's size and tilt as it stands in its line, from a few of each (by its place and the line's).
const SIZES: [f32; 8] = [1.15, 0.78, 1.0, 1.32, 0.86, 1.08, 0.72, 1.22];
const TILTS: [f32; 8] = [-0.09, 0.06, -0.03, 0.08, -0.07, 0.04, -0.05, 0.07];
/// The least share of its size a line is drawn at to fit in one row, else it takes more, at most so many; and how far
/// apart the rows are, of the size they are drawn at.
const LEAST_FIT: f32 = 0.6;
const MOST_ROWS: usize = 3;
const ROW_STEP: f32 = 1.3;

/// What a press is on.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Hit {
    Back,
    Toggle,
    Next,
    Track,
    Mute,
    Volume,
    Leave,
}

/// What it asks of whoever holds it.
#[derive(PartialEq, Debug)]
pub enum Asked {
    Stay,
    Leave,
}

pub struct FullMusic {
    gfx: Rc<Gfx>,
    window: Window,
    /// (None while it could not be made on the device that draws now: made again at the next frame.)
    surface: Option<Surface>,
    smoke: Option<Smoke>,
    /// The screen it covers (physical px) and its scale (physical px to a DIP).
    screen: RECT,
    scale: f32,
    /// That screen's display, as the desktop names it (see `screens::Screen::id`).
    display: Option<String>,
    /// The music and the lines as they go.
    shown: Shown,
    /// The pointer: where it is, when it last moved; the controls, how far come.
    pointer: Option<(f32, f32)>,
    moved: Option<Instant>,
    controls: f32,
    /// Whether the volume's bar is open (the pointer over its icon, or over the bar since), and when the sound was
    /// last turned by the wheel or a key (the bar then open a moment, saying how loud).
    volume_open: bool,
    volume_turned: Option<Instant>,
    /// The wheel's turns not yet a step of the sound (a touchpad turns it by parts of a notch), toward the next (in
    /// steps of a hundred and twentieth).
    wheeled: i32,
    /// Whether the pointer over it is hidden, as last told.
    pointer_hidden: Option<bool>,
    /// What a press is on, as last drawn; the last press (for a double click); what is dragged.
    hits: Vec<((f32, f32, f32, f32), Hit)>,
    pressed: Option<Instant>,
    dragging: Option<Hit>,
}

impl FullMusic {
    /// Over the screen `screen` (physical px) at `scale`, in front with the keyboard; none where its window could
    /// not be made.
    pub fn open(gfx: Rc<Gfx>, screen: RECT, scale: f32) -> Option<Self> {
        // Its lines' serif face, taken from Glance's site the first time (YaHei till it is had).
        crate::serif::want();
        let window = Window::activating().ok()?;
        let surface = Surface::new(&gfx, window.hwnd).ok()?;
        let smoke = Smoke::new(&gfx).inspect_err(|error| crate::journal::note(format!("music full screen: the smoke could not be drawn ({error})"))).ok();
        window.place(screen);
        window.show();
        window.focus();
        let now = Instant::now();
        Some(FullMusic {
            gfx,
            window,
            surface: Some(surface),
            smoke,
            screen,
            scale,
            display: crate::screens::all().into_iter().find(|shown| shown.monitor == screen).map(|shown| shown.id),
            shown: Shown::new(now),
            pointer: None,
            moved: Some(now),
            controls: 1.0,
            pointer_hidden: None,
            volume_open: false,
            volume_turned: None,
            wheeled: 0,
            hits: Vec::new(),
            pressed: None,
            dragging: None,
        })
    }

    pub fn owns(&self, hwnd: HWND) -> bool {
        self.window.hwnd == hwnd
    }

    /// The screens changed (a resolution, a scale, a screen gone or moved): over the whole of its own screen as it is
    /// now, wherever the desktop has it (another screen may be where it was), or of the nearest, if it is gone.
    pub fn screens_changed(&mut self) {
        let own = self.display.as_ref().and_then(|display| crate::screens::all().into_iter().find(|screen| screen.id == *display));
        let at = own.map_or(self.screen, |screen| screen.monitor);
        let middle = POINT { x: (at.left + at.right) / 2, y: (at.top + at.bottom) / 2 };
        let Some(contact) = crate::mouse::monitor_at(middle) else { return };
        if (contact.monitor, contact.scale) == (self.screen, self.scale) {
            return;
        }
        self.screen = contact.monitor;
        self.scale = contact.scale;
        self.window.place(self.screen);
        // (What was laid out at the old size and scale is not hit any more: laid out again at the next frame.)
        self.hits = Vec::new();
    }

    fn size(&self) -> (f32, f32) {
        (((self.screen.right - self.screen.left) as f32) / self.scale, ((self.screen.bottom - self.screen.top) as f32) / self.scale)
    }

    /// Draws a frame; when the next is due (at once: it moves all the time).
    pub fn tick(&mut self, now: Instant) -> Duration {
        let dt = self.shown.last.map_or(0.0, |last| now.duration_since(last).as_secs_f32().min(0.1));
        // The controls: in while the pointer moved lately (or something is dragged), out after.
        let awake = self.dragging.is_some() || self.moved.is_some_and(|moved| now.duration_since(moved) < AWAKE);
        let to = if awake { 1.0 } else { 0.0 };
        self.controls += (to - self.controls).signum() * (dt / CONTROLS_FADE).min((to - self.controls).abs());
        // The pointer hidden with them, and back with them: told as that changes, not at every frame (where the
        // pointer was over another screen, it came and went there).
        if self.pointer_hidden != Some(!awake) {
            self.pointer_hidden = Some(!awake);
            let over = crate::mouse::cursor_position().is_some_and(|at| at.x >= self.screen.left && at.x < self.screen.right && at.y >= self.screen.top && at.y < self.screen.bottom);
            self.window.point_none(!awake, over);
        }
        // What plays, and its cover (a track with none: back toward its own colours, one not yet read: as they are).
        let heard_now = crate::playing::music();
        let cover = heard_now.as_ref().filter(|playing| !playing.title.is_empty()).and_then(|playing| match crate::cover::of(playing) {
            Some(cover) => Some(Some(cover)),
            None if crate::cover::has_none(playing) => Some(None),
            None => None,
        });
        let sung = heard_now.as_ref().and_then(|playing| playing.sung.clone());
        let (lines, sung_now) = match &sung {
            Some(Sung::Lines { lines, now, .. }) => (Some(lines.clone()), *now),
            _ => (None, None),
        };
        // With no lines to show, what plays large, and why there are none under it.
        let said = lines.is_none().then(|| {
            let lang = crate::update::language();
            let why = match (&heard_now, &sung) {
                (None, _) => lang.pick("没有正在播放的音乐", "No music is playing"),
                (Some(_), Some(Sung::Asking)) => lang.pick("正在查找歌词…", "Finding the lyrics…"),
                (Some(_), Some(Sung::Untimed { .. })) => lang.pick("播放器未提供播放进度，无法同步歌词", "The player does not tell how far into the track it is, so the lyrics cannot follow it"),
                (Some(_), Some(Sung::Instrumental)) => lang.pick("这首歌是纯音乐", "This track is instrumental"),
                _ => lang.pick("没有找到歌词", "No lyrics were found"),
            };
            let (title, artist) = heard_now.as_ref().map_or_else(Default::default, |playing| (playing.title.clone(), playing.artist.clone()));
            (title, artist, why.to_string())
        });
        self.shown.follow(now, crate::sound::heard(), cover.as_ref().map(Option::as_deref), lines, sung_now);
        let position = crate::playing::position(Follow::Music).unwrap_or(0.0);
        let playing = heard_now.as_ref().is_some_and(|playing| playing.playing);
        let length = heard_now.as_ref().and_then(|playing| playing.timeline).map(|timeline| (timeline.length, timeline.seekable));
        let (w, h) = self.size();
        let pixels = ((self.screen.right - self.screen.left).max(1) as u32, (self.screen.bottom - self.screen.top).max(1) as u32);
        // On the device for its screen, if it is not on it (a driver reset, a device lost: see `gfx::on_screen`): made
        // anew on it. (A window has one composition target: the old one goes first.)
        if let Ok(current) = crate::ui::gfx::for_area(self.screen) {
            if !Rc::ptr_eq(&current, &self.gfx) || self.surface.is_none() {
                self.surface = None;
                self.surface = Surface::new(&current, self.window.hwnd).ok();
                self.smoke = Smoke::new(&current).ok();
                self.gfx = current;
            }
        }
        if let Some(smoke) = self.smoke.as_mut() {
            if let Err(error) = self.shown.render_smoke(smoke, pixels) {
                crate::journal::note(format!("music full screen: the smoke could not be drawn ({error})"));
                self.smoke = None;
            }
        }
        let (smoke, shown) = (&self.smoke, &self.shown);
        let (pointer, controls, dragging) = (self.pointer, self.controls, self.dragging);
        let mut volume_open = self.volume_open;
        let flash = self.volume_turned.is_some_and(|turned| now.duration_since(turned) < AWAKE);
        let mut hits = Vec::new();
        let gfx = self.gfx.clone();
        let Some(surface) = self.surface.as_mut() else { return Duration::ZERO };
        let drawn = surface.draw(&gfx, pixels, self.scale, |frame| {
            frame.origin(0.0, 0.0);
            shown.paint(frame, (w, h), smoke.as_ref(), position, now);
            if let Some((title, artist, why)) = &said {
                draw_said(frame, (w, h), title, artist, why);
            }
            if controls > 0.0 {
                // (Coming up a little as they come in.)
                frame.origin(0.0, (1.0 - controls) * 6.0);
                draw_controls(frame, (w, h), controls, pointer, dragging, &mut volume_open, flash, playing, position, length, &mut hits);
                frame.origin(0.0, 0.0);
            }
        });
        // Not drawn: drawn again as it next draws; the device gone, on the one made in its place (above).
        if let Err(error) = &drawn {
            if crate::ui::gfx::failed("music full screen", &gfx, Some(error)) {
                self.surface = None;
            }
        }
        self.hits = hits;
        self.volume_open = volume_open;
        // What was laid out for lines no longer drawn, let go (with no widget or panel drawing meanwhile, it was kept
        // for the whole of a playlist).
        self.gfx.sweep();
        Duration::ZERO
    }

    /// The pointer moved to `at` (physical px, in the window).
    pub fn moved(&mut self, at: (i32, i32)) {
        let at = (at.0 as f32 / self.scale, at.1 as f32 / self.scale);
        if self.pointer.is_some_and(|was| (was.0 - at.0).abs() + (was.1 - at.1).abs() < 0.5) {
            return;
        }
        self.pointer = Some(at);
        self.moved = Some(Instant::now());
        if let Some(dragged) = self.dragging {
            self.drag(dragged, at);
        }
    }

    /// Pressed at `at` (physical px): a control does what it does; twice in a row elsewhere, it is left.
    pub fn press(&mut self, at: (i32, i32)) -> Asked {
        let at = (at.0 as f32 / self.scale, at.1 as f32 / self.scale);
        self.pointer = Some(at);
        self.moved = Some(Instant::now());
        let hit = self.hits.iter().find(|((x, y, w, h), _)| at.0 >= *x && at.0 < x + w && at.1 >= *y && at.1 < y + h).map(|(_, hit)| *hit);
        match hit {
            Some(Hit::Back) => crate::playing::act_on(Follow::Music, crate::playing::Act::Key(crate::playing::Key::Previous)),
            Some(Hit::Toggle) => crate::playing::act_on(Follow::Music, crate::playing::Act::Key(crate::playing::Key::Toggle)),
            Some(Hit::Next) => crate::playing::act_on(Follow::Music, crate::playing::Act::Key(crate::playing::Key::Next)),
            Some(Hit::Mute) => {
                if let Some((_, muted)) = crate::playing::sound() {
                    crate::playing::set_sound(None, Some(!muted));
                }
            }
            Some(Hit::Leave) => return Asked::Leave,
            Some(dragged @ (Hit::Track | Hit::Volume)) => {
                self.dragging = Some(dragged);
                // (The pointer held till it is let go, wherever it goes: see `lost`.)
                unsafe {
                    let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetCapture(self.window.hwnd);
                }
                self.drag(dragged, at);
            }
            None => {
                let now = Instant::now();
                if self.pressed.is_some_and(|before| now.duration_since(before) <= DOUBLE) {
                    return Asked::Leave;
                }
                self.pressed = Some(now);
            }
        }
        Asked::Stay
    }

    /// Let go: what was dragged is where it was let go.
    pub fn release(&mut self) {
        if let Some(Hit::Track) = self.dragging.take() {
            crate::playing::slid(crate::playing::Slider::Seek(Follow::Music), true);
        }
        crate::ui::window::let_go_of_capture(self.window.hwnd);
    }

    /// The pointer taken from it mid-drag (another window took it, or it is being left): the drag given up, the
    /// track as it was (left dragged, what plays stayed shown where the drag had it).
    pub fn lost(&mut self) {
        if let Some(Hit::Track) = self.dragging.take() {
            crate::playing::slid(crate::playing::Slider::Seek(Follow::Music), false);
        }
    }

    /// What is dragged, to `at`: the track (shown that far in, played from there once let go) or the volume.
    fn drag(&mut self, dragged: Hit, at: (f32, f32)) {
        let Some((area, _)) = self.hits.iter().find(|(_, hit)| *hit == dragged).copied() else { return };
        match dragged {
            Hit::Track => crate::playing::slide_along(crate::playing::Slider::Seek(Follow::Music), along_track(area, at.0)),
            // (Upright: from its bottom up.)
            Hit::Volume => {
                let (top, bottom) = volume_bar(area);
                let fraction = ((bottom - at.1) / (bottom - top).max(1.0)).clamp(0.0, 1.0);
                crate::playing::set_sound(Some((fraction * 100.0).round() as u8), Some(false));
            }
            _ => {}
        }
    }

    /// The sound louder or softer by `step` (of 100; none: as it is, muted still where it is), the volume's bar shown a
    /// moment saying how loud.
    fn turn_sound(&mut self, step: i32) {
        if let Some((level, _)) = crate::playing::sound().filter(|_| step != 0) {
            crate::playing::set_sound(Some((level as i32 + step).clamp(0, 100) as u8), Some(false));
        }
        self.volume_turned = Some(Instant::now());
        self.moved = Some(Instant::now());
    }

    /// The wheel turned by `delta` (120 a notch): the sound louder or softer, five in a hundred a notch (a part of a
    /// notch kept toward the next).
    pub fn wheel(&mut self, delta: i16) {
        let step = crate::playing::wheel_steps(&mut self.wheeled, delta, 5);
        self.turn_sound(step);
    }

    /// A key pressed: Esc leaves it, the space bar pauses or plays, left and right go back and on five seconds, up
    /// and down turn the sound.
    pub fn key(&mut self, key: u16) -> Asked {
        match key {
            0x1B => return Asked::Leave,
            0x20 => crate::playing::act_on(Follow::Music, crate::playing::Act::Key(crate::playing::Key::Toggle)),
            0x25 | 0x27 if crate::playing::music().and_then(|playing| playing.timeline).is_some_and(|timeline| timeline.seekable) => {
                let position = crate::playing::position(Follow::Music).unwrap_or(0.0);
                let to = position + if key == 0x27 { 5.0 } else { -5.0 };
                crate::playing::act_on(Follow::Music, crate::playing::Act::Seek(to.max(0.0)));
                self.moved = Some(Instant::now());
            }
            0x26 => self.turn_sound(5),
            0x28 => self.turn_sound(-5),
            _ => {}
        }
        Asked::Stay
    }
}

/// What the full screen shows as the music goes, wherever it is drawn: the music followed (each band slowly, the swell
/// of the lows, how far the smoke has flowed, the beat), the smoke's colours easing toward the cover's, and the lines
/// with the one sung now and the one leaving.
pub struct Shown {
    smooth: [f32; crate::sound::BANDS],
    swell: f32,
    flow: f32,
    /// The beat, up quickly and down slowly (as it is heard, it comes and goes from one frame to the next).
    beat: f32,
    last: Option<Instant>,
    began: Instant,
    tint: [f32; 3],
    tint2: [f32; 3],
    /// The line sung now, and the one before it with when it began to leave.
    now: Option<usize>,
    leaving: Option<(usize, Instant)>,
    lines: Option<Arc<Vec<Line>>>,
}

impl Shown {
    pub fn new(now: Instant) -> Shown {
        Shown { smooth: [0.0; crate::sound::BANDS], swell: 0.0, flow: 0.0, beat: 0.0, last: None, began: now, tint: TINT, tint2: TINT2, now: None, leaving: None, lines: None }
    }

    /// Followed to `now`: `heard`, the cover's colours (`cover`: none known yet keeps them, a track with none goes back
    /// toward its own), and `lines` with the one sung now.
    pub fn follow(&mut self, now: Instant, heard: crate::sound::Heard, cover: Option<Option<&crate::cover::Cover>>, lines: Option<Arc<Vec<Line>>>, sung_now: Option<usize>) {
        let dt = self.last.map_or(0.0, |last| now.duration_since(last).as_secs_f32().min(0.1));
        self.last = Some(now);
        let k = 1.0 - (-dt / 0.22).exp();
        for (shown, band) in self.smooth.iter_mut().zip(heard.bands) {
            *shown += (band - *shown) * k;
        }
        let bass = part(0, 4, &heard.bands);
        self.swell += (bass - self.swell) * (1.0 - (-dt / 0.45).exp());
        self.flow += dt * (0.55 + self.swell * 0.9);
        let heard_beat = heard.beat.clamp(0.0, 1.0);
        let pace = if heard_beat > self.beat { 0.04 } else { 0.3 };
        self.beat += (heard_beat - self.beat) * (1.0 - (-dt / pace).exp());
        let toward = cover.map(|cover| match cover {
            Some(cover) => {
                let top = cover.base.iter().fold(0.01f32, |top, c| top.max(*c));
                let tint = cover.base.map(|c| (c / top * 0.96).min(1.0));
                (tint, tint.map(|c| c + (1.0 - c) * 0.55))
            }
            None => (TINT, TINT2),
        });
        if let Some((tint, tint2)) = toward {
            let ease = 1.0 - (-dt / 0.6).exp();
            self.tint = std::array::from_fn(|i| self.tint[i] + (tint[i] - self.tint[i]) * ease);
            self.tint2 = std::array::from_fn(|i| self.tint2[i] + (tint2[i] - self.tint2[i]) * ease);
        }
        if lines.as_ref().map(Arc::as_ptr) != self.lines.as_ref().map(Arc::as_ptr) {
            self.leaving = None;
        } else if sung_now != self.now {
            self.leaving = self.now.map(|before| (before, now));
        }
        self.lines = lines;
        self.now = sung_now;
    }

    /// The smoke made for `pixels` (the whole of what it covers) as it flows now.
    pub fn render_smoke(&self, smoke: &mut Smoke, pixels: (u32, u32)) -> windows::core::Result<()> {
        let made = ((pixels.0 as f32 * smoke::SCALE) as u32, (pixels.1 as f32 * smoke::SCALE) as u32);
        smoke.render(made, &smoke::Now { flow: self.flow, swell: self.swell, tint: self.tint, tint2: self.tint2 })
    }

    /// Black, the smoke over it (as last made), and the lines at `position` (seconds into the track), at `now`.
    pub fn paint(&self, frame: &Frame, (w, h): (f32, f32), smoke: Option<&Smoke>, position: f64, now: Instant) {
        frame.fill(Color::hex(0x000000, 1.0), 0.0, 0.0, w, h);
        if let Some(smoke) = smoke {
            smoke.paint(frame, rect(0.0, 0.0, w, h));
        }
        if let Some(lines) = &self.lines {
            let music = Music { smooth: self.smooth, swell: self.swell, beat: self.beat, high: part(14, 20, &self.smooth), bass: part(0, 4, &self.smooth) };
            let t = now.duration_since(self.began).as_secs_f32();
            draw_lines(frame, (w, h), lines, self.now, self.leaving.map(|(index, since)| (index, now.duration_since(since).as_secs_f32())), position, t, &music);
        }
    }
}

/// The mean of the bands `from` to `to` of `of`.
fn part(from: usize, to: usize, of: &[f32]) -> f32 {
    of[from..to].iter().sum::<f32>() / (to - from) as f32
}

/// The music as the lines move with it: each band followed slowly, the lows' swell, the beat, the lows and the highs.
struct Music {
    #[allow(dead_code)]
    smooth: [f32; crate::sound::BANDS],
    swell: f32,
    beat: f32,
    high: f32,
    bass: f32,
}

fn ease(x: f32) -> f32 {
    1.0 - (1.0 - x.clamp(0.0, 1.0)).powi(3)
}

/// Each character of `line` with when it is sung and for how long: by its words' times where it has them (each
/// word's characters sharing it evenly), else its time shared evenly between them all (to the next line, or four
/// seconds on).
fn timed(line: &Line, next: Option<f64>) -> Vec<(char, f64, f64)> {
    if !line.words.is_empty() {
        return line
            .words
            .iter()
            .flat_map(|word| {
                let sung = word.text.trim_end().chars().count().max(1) as f64;
                let each = (word.end - word.at).max(0.0) / sung;
                word.text.chars().enumerate().map(move |(k, ch)| (ch, word.at + each * (k as f64).min(sung - 1.0), each))
            })
            .collect();
    }
    let chars: Vec<char> = line.text.chars().collect();
    let end = line.end.or(next).unwrap_or(line.at + 4.0);
    let each = (end - line.at).max(0.0) * 0.85 / chars.len().max(1) as f64;
    chars.into_iter().enumerate().map(|(k, ch)| (ch, line.at + each * k as f64, each)).collect()
}

/// The line sung now (and the one before it as it leaves, `leaving` seconds since) large at the left of `(w, h)`,
/// and the next line small under it.
#[allow(clippy::too_many_arguments)]
fn draw_lines(frame: &Frame, (w, h): (f32, f32), lines: &[Line], now: Option<usize>, leaving: Option<(usize, f32)>, position: f64, t: f32, music: &Music) {
    let left = w * 0.081;
    let baseline = h * 0.62;
    let base = h * 0.133;
    if let Some((before, since)) = leaving.filter(|(_, since)| *since < LEAVING) {
        if let Some(line) = lines.get(before) {
            draw_line(frame, (w, h), line, lines.get(before + 1).map(|line| line.at), before, position, t, music, ease(since / LEAVING), (left, baseline, base));
        }
    }
    let Some(index) = now else { return };
    let line = &lines[index];
    let next = lines.get(index + 1);
    draw_line(frame, (w, h), line, next.map(|line| line.at), index, position, t, music, 0.0, (left, baseline, base));
    // The next line, small under it, coming in a moment after.
    if let Some(next) = next.filter(|next| !next.text.trim().is_empty()) {
        let font = Font::new(crate::serif::family(), (h * 0.029).max(14.0), 700.0);
        let shown = ((position - line.at) as f32 - 0.3).clamp(0.0, 1.0) * 0.5;
        frame.text_moving(&next.text, font, Color::hex(0xFFFFFF, shown), left + 4.0, baseline + h * 0.09, w - 2.0 * left, Align::Start);
    }
}

/// The fewest rows a line whose characters (each word, of a script that writes words) take `rooms` (and are spaces
/// where `blank` says) is drawn in, `wide` across: one, unless it would be made smaller than `LEAST_FIT` of its size
/// to fit (as a long line on a screen stood on end would), then as many more as keep it at least that large, at most
/// `MOST_ROWS`. Each row as long as the others as near as may be, broken between characters (between words), a space
/// where it breaks left at the end of the row before; and how much smaller than its size it is drawn to fit.
fn rows_of(rooms: &[f32], blank: &[bool], wide: f32) -> (Vec<std::ops::Range<usize>>, f32) {
    // How long a row of these is: without the spaces at its ends.
    let long = |row: &std::ops::Range<usize>| {
        let (mut from, mut to) = (row.start, row.end);
        while from < to && blank[from] {
            from += 1;
        }
        while to > from && blank[to - 1] {
            to -= 1;
        }
        rooms[from..to].iter().sum::<f32>()
    };
    let total: f32 = rooms.iter().sum();
    let mut chosen = (vec![0..rooms.len()], 1.0);
    for count in 1..=MOST_ROWS.min(rooms.len().max(1)) {
        // Each break where the room before it comes nearest to an even share of the whole.
        let mut rows = Vec::new();
        let (mut start, mut before) = (0, 0.0);
        for row in 1..count {
            let share = total * row as f32 / count as f32;
            let mut end = start + 1;
            while end < rooms.len() - (count - row) && (before + rooms[start..end].iter().sum::<f32>() + rooms[end] / 2.0) < share {
                end += 1;
            }
            before += rooms[start..end].iter().sum::<f32>();
            rows.push(start..end);
            start = end;
        }
        rows.push(start..rooms.len());
        let fit = (wide / rows.iter().map(long).fold(1.0, f32::max)).min(1.0);
        chosen = (rows, fit);
        if fit >= LEAST_FIT {
            break;
        }
    }
    chosen
}

/// A line's characters (see `timed`) as they move together: each of Chinese, Japanese and Korean script on its own,
/// and of the others each word whole (its letters one, as they are read: moved each its own way, a word in English
/// came apart into letters strewn along the wave); each with when it is sung and for how long, and spaces as they
/// are.
fn units(chars: Vec<(char, f64, f64)>) -> Vec<(String, f64, f64)> {
    let alone = |ch: char| ch.is_whitespace() || is_cjk(ch);
    let mut units: Vec<(String, f64, f64)> = Vec::new();
    let mut joining = false;
    for (ch, at, each) in chars {
        match units.last_mut() {
            Some((said, _, lasts)) if joining && !alone(ch) => {
                said.push(ch);
                *lasts += each;
            }
            _ => units.push((ch.to_string(), at, each)),
        }
        joining = !alone(ch);
    }
    units
}

/// Whether `ch` is of a script whose characters stand each on its own as they are read (Chinese, Japanese, Korean,
/// and their marks and full-width forms).
fn is_cjk(ch: char) -> bool {
    matches!(ch as u32, 0x1100..=0x11FF | 0x2E80..=0x2FDF | 0x3000..=0x303F | 0x3040..=0x30FF | 0x3100..=0x31FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0xFE30..=0xFE4F | 0xFF00..=0xFFEF | 0x20000..=0x2FFFF)
}

/// One line, each character (each word, of a script that writes words) its size and tilt along a slowly drifting wave, coming up out of a blur as it is sung,
/// lit and swelling with the music while it is; `out` of the way left (rising and fading).
#[allow(clippy::too_many_arguments)]
fn draw_line(frame: &Frame, (w, _h): (f32, f32), line: &Line, next: Option<f64>, which: usize, position: f64, t: f32, music: &Music, out: f32, (left, baseline, base): (f32, f32, f32)) {
    let chars = units(timed(line, next));
    if chars.iter().all(|(said, ..)| said.trim().is_empty()) {
        return;
    }
    // Each character's size and room, in as few rows as keep them large enough (see `rows_of`), the whole made smaller
    // where it would not fit; the last row on the baseline, those before it above it.
    let mut sizes: Vec<f32> = (0..chars.len()).map(|k| base * SIZES[(k + which * 3) % 8]).collect();
    let mut rooms: Vec<f32> = chars.iter().zip(&sizes).map(|((said, ..), size)| if said.trim().is_empty() { size * 0.4 } else { frame.measure(said, Font::new(crate::serif::family(), *size, 700.0)) + base * 0.08 }).collect();
    let blank: Vec<bool> = chars.iter().map(|(said, ..)| said.trim().is_empty()).collect();
    let (rows, fit) = rows_of(&rooms, &blank, w - 2.0 * left);
    sizes.iter_mut().for_each(|size| *size *= fit);
    rooms.iter_mut().for_each(|room| *room *= fit);
    let row_of = |k: usize| rows.iter().position(|row| row.contains(&k)).unwrap_or(0);
    let row_baseline = |row: usize| baseline - (rows.len() - 1 - row) as f32 * base * fit * ROW_STEP;
    let sway = (t * 0.4).sin() * 0.04;
    // Each character as it stands now: what it says, its face, its middle and top, the point it turns and swells
    // about, its tilt and swell, how far it has come, how lit it is, how much of it shows, its room.
    struct Glyph {
        said: String,
        font: Font,
        middle: f32,
        top: f32,
        pivot: f32,
        tilt: f32,
        swell: f32,
        came: f32,
        lit: f32,
        alpha: f32,
        room: f32,
    }
    let mut glyphs = Vec::new();
    let mut x = left;
    for (k, ((said, at, each), (size, room))) in chars.iter().zip(sizes.iter().zip(&rooms)).enumerate() {
        // (A row begins at the left; a space at its start is not drawn, and takes no room.)
        if rows.iter().any(|row| row.start == k) {
            x = left;
        }
        if said.trim().is_empty() {
            x += if x == left { 0.0 } else { *room };
            continue;
        }
        let middle = x + room / 2.0;
        x += room;
        let age = (position - at) as f32;
        if age < -0.25 {
            continue;
        }
        let font = Font::new(crate::serif::family(), *size, 700.0);
        let y = row_baseline(row_of(k)) + (k as f32 * 0.9 + t * 0.8 + which as f32).sin() * base * fit * 0.18 + (middle - w / 2.0) * sway;
        let came = ease((age + 0.25) / 0.55);
        // How lit it is: up as it is sung, down after.
        let lit = ease((age + 0.08) / 0.18) * (1.0 - ease((age - *each as f32 - 0.1) / 0.5)) * (1.0 - out);
        let breathe = 1.0 + 0.02 * (t * 1.7 + k as f32 * 1.3).sin() * (0.5 + music.swell);
        let rise = (1.0 - came) * 26.0 - out * 60.0 - music.beat * 3.0;
        let (ascent, _) = frame.gfx.baseline(font);
        glyphs.push(Glyph {
            said: said.clone(),
            font,
            middle,
            top: y - ascent + rise,
            pivot: y - size * 0.35 + rise,
            tilt: (TILTS[(k + which) % 8] + (t * 0.6 + k as f32).sin() * 0.025) * (0.4 + 0.6 * came),
            swell: breathe * (1.0 + lit * (0.14 * music.bass + 0.04 * music.beat)),
            came,
            lit,
            alpha: (came * 1.4).min(1.0) * (1.0 - out),
            room: *room,
        });
    }
    // Each drawn in `colour`, so much of it as `of` says.
    let draw = |frame: &Frame, colour: [f32; 3], of: &dyn Fn(&Glyph) -> f32| {
        for glyph in &glyphs {
            let a = of(glyph);
            if a <= 0.004 {
                continue;
            }
            frame.turned(glyph.middle, glyph.pivot, glyph.tilt, (glyph.swell, glyph.swell), |frame| {
                frame.text_moving(&glyph.said, glyph.font, Color { r: colour[0], g: colour[1], b: colour[2], a }, glyph.middle - glyph.room / 2.0, glyph.top, glyph.room * 2.0, Align::Start)
            });
        }
    };
    // The glows, each for the whole line at once (one blur for each, not one for each character: so many a frame,
    // the frames came late and the line moved by jumps): every character's, softly; the one being sung's, wider and
    // stronger with the highs; and those still coming, out of a blur.
    let white = [1.0, 1.0, 1.0];
    frame.glow(Color::hex(0xFFFFFF, 0.85), (10.0 + 8.0 * music.swell) / 2.0, |frame| draw(frame, white, &|glyph| glyph.alpha));
    if glyphs.iter().any(|glyph| glyph.lit > 0.01) {
        frame.glow(Color::hex(0xFFFFFF, 0.85), (24.0 + 30.0 * music.high) / 2.0, |frame| draw(frame, white, &|glyph| glyph.alpha * glyph.lit));
    }
    if glyphs.iter().any(|glyph| glyph.came < 1.0) {
        frame.glow(Color::hex(0xFFFFFF, 1.0), 4.0, |frame| draw(frame, white, &|glyph| glyph.alpha * (1.0 - glyph.came)));
    }
    for glyph in &glyphs {
        let ink = 0.92 + 0.08 * glyph.lit;
        let a = glyph.alpha * glyph.came * glyph.came;
        if a > 0.004 {
            frame.turned(glyph.middle, glyph.pivot, glyph.tilt, (glyph.swell, glyph.swell), |frame| {
                frame.text_moving(&glyph.said, glyph.font, Color { r: ink, g: ink * 0.985, b: ink * 0.965, a }, glyph.middle - glyph.room / 2.0, glyph.top, glyph.room * 2.0, Align::Start)
            });
        }
    }
}

/// What plays, large at the left where the lines would be (its title, its artist under it), and why no lines show
/// under that.
fn draw_said(frame: &Frame, (w, h): (f32, f32), title: &str, artist: &str, why: &str) {
    let left = w * 0.081;
    let baseline = h * 0.62;
    let big = Font::new(crate::serif::family(), h * 0.09, 700.0);
    let small = Font::new(crate::serif::family(), (h * 0.029).max(14.0), 700.0);
    let (ascent, _) = frame.gfx.baseline(big);
    let room = w - 2.0 * left;
    if !title.is_empty() {
        frame.glow(Color::hex(0xFFFFFF, 0.6), 8.0, |frame| frame.text_moving(title, big, Color::hex(0xFFFFFF, 1.0), left, baseline - ascent, room, Align::Start));
        frame.text_moving(title, big, Color::hex(0xF4F1EC, 0.95), left, baseline - ascent, room, Align::Start);
    }
    let mut y = baseline + h * 0.03;
    if !artist.is_empty() {
        frame.text_moving(artist, small, Color::hex(0xFFFFFF, 0.75), left + 4.0, y, room, Align::Start);
        y += small.size * 1.6;
    }
    frame.text_moving(why, small, Color::hex(0xFFFFFF, 0.5), left + 4.0, y, room, Align::Start);
}

/// The controls, `shown` of them, along the bottom of `(w, h)`: the track short in the middle with its times at its
/// ends, the keys under it, the volume and the way out at the right; each lit under the pointer. What they are is
/// put in `hits`.
#[allow(clippy::too_many_arguments)]
fn draw_controls(frame: &Frame, (w, h): (f32, f32), shown: f32, pointer: Option<(f32, f32)>, dragging: Option<Hit>, volume_open: &mut bool, flash: bool, playing: bool, position: f64, length: Option<(f64, bool)>, hits: &mut Vec<((f32, f32, f32, f32), Hit)>) {
    let under = |x: f32, y: f32, wide: f32, tall: f32| pointer.is_some_and(|(px, py)| px >= x && px < x + wide && py >= y && py < y + tall);
    // A darkening along the bottom for them to be read on.
    frame.fill_shape(
        &[Point { x: 0.0, y: h - 140.0 }, Point { x: w, y: h - 140.0 }, Point { x: w, y: h }, Point { x: 0.0, y: h }],
        crate::ui::canvas::Fill::Down { top: h - 140.0, from: Color::hex(0x000000, 0.0), bottom: h, to: Color::hex(0x000000, 0.42 * shown) },
    );
    let ink = |lit: bool| Color::hex(0xFFFFFF, if lit { shown } else { 0.82 * shown });
    // The keys.
    let cell = 44.0;
    let row = h - 22.0 - cell;
    let mut x = w / 2.0 - cell * 1.5 - 18.0;
    for (hit, icon, side) in [(Hit::Back, Icon::SkipBack, 20.0), (Hit::Toggle, if playing { Icon::Pause } else { Icon::Play }, 30.0), (Hit::Next, Icon::SkipForward, 20.0)] {
        let lit = under(x, row, cell, cell);
        frame.icon(icon, Point { x: x + cell / 2.0, y: row + cell / 2.0 }, side, ink(lit));
        hits.push(((x, row, cell, cell), hit));
        x += cell + 18.0;
    }
    // The track, short, above them; its times at its ends. Under the pointer (or dragged), brighter, and over the
    // pointer the time it is at: where its player lets it be moved along (QQ Music does not), else only
    // how far it is.
    if let Some((length, seekable)) = length.filter(|(length, _)| *length > 0.0) {
        let time = Font::new(Family::Segoe, 12.0, 500.0);
        let wide = (w * 0.42).min(420.0);
        let (from, y) = (w / 2.0 - wide / 2.0, row - 22.0);
        let lit = seekable && (under(from - 8.0, y - 10.0, wide + 16.0, 20.0) || dragging == Some(Hit::Track));
        let fraction = (position / length).clamp(0.0, 1.0) as f32;
        slider(frame, (from, y, wide), fraction, lit, shown);
        let clock = |seconds: f64| crate::ui::text::clock(seconds.max(0.0).round() as u32);
        frame.text(&clock(position), time, Color::hex(0xFFFFFF, 0.6 * shown), from - 70.0, y - 9.0, 60.0, Align::End);
        frame.text(&clock(length), time, Color::hex(0xFFFFFF, 0.6 * shown), from + wide + 10.0, y - 9.0, 60.0, Align::Start);
        if let Some((px, _)) = pointer.filter(|_| lit) {
            let at = ((px - from) / wide).clamp(0.0, 1.0);
            let said = clock(at as f64 * length);
            let pill = frame.measure(&said, time);
            let x = (from + wide * at - pill / 2.0).clamp(from, from + wide - pill);
            said_bare(frame, &said, time, x, y - 30.0, shown);
        }
        if seekable {
            hits.push(((from - TRACK_REACH, y - 10.0, wide + 2.0 * TRACK_REACH, 20.0), Hit::Track));
        }
    }
    // At the right, the way out, and the sound's icon beside it; over the icon, while the pointer is over it or the
    // bar (or the sound was just turned by the wheel or a key: `flash`), a bar standing up, filled as loud as the sound
    // is, its level said at its top. (Upright, it takes no room along the bottom.)
    // (As last seen, not asked of the sound's service at each frame; none where it could not be read: said so.)
    let sound = crate::playing::sound();
    let (level, muted) = sound.unwrap_or((0, false));
    let leave_x = w - 30.0 - cell;
    let middle = row + cell / 2.0;
    let lit = under(leave_x, row, cell, cell);
    frame.icon(Icon::Minimize, Point { x: leave_x + cell / 2.0, y: middle }, 20.0, ink(lit));
    hits.push(((leave_x, row, cell, cell), Hit::Leave));
    let mute_x = leave_x - 8.0 - cell;
    let on_icon = under(mute_x, row, cell, cell);
    let (pop_w, pop_h) = (VOLUME_POP.0, VOLUME_POP.1);
    let (pop_x, pop_y) = (mute_x + (cell - pop_w) / 2.0, row - 6.0 - pop_h);
    *volume_open = on_icon || flash || dragging == Some(Hit::Volume) || (*volume_open && under(pop_x - 6.0, pop_y - 6.0, pop_w + 12.0, row + cell - pop_y + 6.0));
    frame.icon(if muted || level == 0 { Icon::Muted } else { Icon::Volume }, Point { x: mute_x + cell / 2.0, y: middle }, 20.0, ink(on_icon));
    hits.push(((mute_x, row, cell, cell), Hit::Mute));
    if *volume_open {
        // On nothing but the picture, as the track is: the line alone, and, the pointer over it, beside the pointer
        // how loud that place along it is.
        let fraction = if muted { 0.0 } else { level as f32 / 100.0 };
        let small = Font::new(Family::Segoe, 12.0, 600.0);
        let (top, bottom) = volume_bar((pop_x, pop_y, pop_w, pop_h));
        let over = under(pop_x, pop_y, pop_w, pop_h);
        let lit = over || dragging == Some(Hit::Volume);
        let thick = 3.0;
        let x = pop_x + pop_w / 2.0 - thick / 2.0;
        // The track's box put on whole pixels once, the level standing in it from its bottom (put on pixels alone,
        // the level's foot moved by a pixel as it rose).
        let whole = frame.on_pixels(x, top, thick, bottom - top);
        frame.fill_rounded(Color::hex(0xFFFFFF, if lit { 0.3 } else { 0.22 } * shown), whole.0, whole.1, whole.2, whole.3, thick / 2.0);
        let (lx, ly, lw, lh) = crate::ui::view::filled_part(frame, whole, fraction, true);
        if lh > 0.0 {
            frame.fill_rounded(Color::hex(0xFFFFFF, if lit { 1.0 } else { 0.85 } * shown), lx, ly, lw, lh, thick / 2.0);
        }
        if let Some((_, py)) = pointer.filter(|_| over) {
            let at = ((bottom - py) / (bottom - top)).clamp(0.0, 1.0);
            let said = format!("{}%", (at * 100.0).round() as u32);
            let wide = frame.measure(&said, small);
            said_bare(frame, &said, small, pop_x + pop_w / 2.0 - 12.0 - wide, py.clamp(top, bottom) - 8.0, shown);
        }
        hits.push(((pop_x, pop_y, pop_w, pop_h), Hit::Volume));
    }
}

/// The volume's bar standing up over its icon: how wide and tall it is, round its line.
const VOLUME_POP: (f32, f32) = (40.0, 156.0);

/// Where the volume's line runs in its bar `(x, y, w, h)`: from near its top to near its bottom.
fn volume_bar((_, y, _, h): (f32, f32, f32, f32)) -> (f32, f32) {
    (y + 16.0, y + h - 16.0)
}

/// Words said over the picture with nothing under them: white, a soft dark glow round them to be read on any part of
/// the smoke.
fn said_bare(frame: &Frame, said: &str, font: Font, x: f32, y: f32, shown: f32) {
    frame.glow(Color::hex(0x000000, 0.55 * shown), 3.0, |frame| frame.text(said, font, Color::hex(0x000000, 1.0), x, y, 200.0, Align::Start));
    frame.text(said, font, Color::hex(0xFFFFFF, 0.95 * shown), x, y, 200.0, Align::Start);
}

/// A slider along `(x, y, wide)` (its middle line), `fraction` of it filled, `shown` of it drawn: a thin line,
/// brighter where it is `lit` (the pointer over it, or dragged), never thicker; no knob (the screen kept spare).
fn slider(frame: &Frame, (x, y, wide): (f32, f32, f32), fraction: f32, lit: bool, shown: f32) {
    let thick = 3.0;
    frame.fill_rounded(Color::hex(0xFFFFFF, if lit { 0.3 } else { 0.22 } * shown), x, y - thick / 2.0, wide, thick, thick / 2.0);
    frame.fill_rounded(Color::hex(0xFFFFFF, if lit { 1.0 } else { 0.85 } * shown), x, y - thick / 2.0, wide * fraction, thick, thick / 2.0);
}

#[cfg(all(test, feature = "studio"))]
mod pictures {
    use super::*;

    /// A frame of it, a line half sung with the next under it and the controls up, to a PNG in the target folder:
    /// `cargo test --features studio -- --ignored draws_the_full_screen`.
    #[test]
    #[ignore = "draws on this PC's graphics card, to a picture to be looked at"]
    fn draws_the_full_screen() {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        // (In the serif face, where its file is named: else in YaHei, as till it is had.)
        if let Ok(face) = std::env::var("GLANCE_SERIF") {
            crate::serif::have(std::fs::read(face).unwrap());
        }
        let gfx = crate::ui::gfx::held().unwrap();
        let (w, h) = (1280.0f32, 720.0f32);
        let mut smoke = Smoke::new(&gfx).unwrap();
        smoke.render(((w * smoke::SCALE) as u32, (h * smoke::SCALE) as u32), &smoke::Now { flow: 14.0, swell: 0.6, tint: [0.95, 0.66, 0.58], tint2: [0.98, 0.85, 0.81] }).unwrap();
        let line = |at: f64, text: &str| Line { at, end: None, text: text.into(), words: Vec::new(), backing: None, translation: None };
        let lines = vec![line(10.0, "如果你因为我而诚实"), line(14.5, "如果你看我的电影")];
        let music = Music { smooth: [0.5; crate::sound::BANDS], swell: 0.6, beat: 0.4, high: 0.5, bass: 0.6 };
        let pixels = gfx
            .draw_offscreen((w as u32, h as u32), 1.0, |frame| {
                frame.fill(Color::hex(0x000000, 1.0), 0.0, 0.0, w, h);
                smoke.paint(frame, rect(0.0, 0.0, w, h));
                draw_lines(frame, (w, h), &lines, Some(0), None, 12.6, 3.0, &music);
                let mut hits = Vec::new();
                draw_controls(frame, (w, h), 1.0, Some((w / 2.0, h - 88.0)), None, &mut true, true, true, 80.0, Some((226.0, true)), &mut hits);
            })
            .unwrap();
        let out = format!("{}/target/music", env!("CARGO_MANIFEST_DIR"));
        std::fs::create_dir_all(&out).unwrap();
        crate::studio::save_png(&std::path::PathBuf::from(format!("{out}/full.png")), w as u32, h as u32, &pixels).unwrap();
    }
}

/// How far along the track `x` is, the track pressed in `area` (its reach on either side with it): along it as it is
/// drawn, its ends where it is drawn to.
fn along_track(area: (f32, f32, f32, f32), x: f32) -> f32 {
    ((x - area.0 - TRACK_REACH) / (area.2 - 2.0 * TRACK_REACH).max(1.0)).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    /// A press on the track goes as far along what plays as the time said over it: none at its drawn start, all at its
    /// drawn end, as much past either (in its reach).
    #[test]
    fn the_track_is_pressed_where_it_is_drawn() {
        let (from, wide) = (100.0, 420.0);
        let area = (from - super::TRACK_REACH, 0.0, wide + 2.0 * super::TRACK_REACH, 20.0);
        assert_eq!(super::along_track(area, from), 0.0);
        assert_eq!(super::along_track(area, from + wide), 1.0);
        assert_eq!(super::along_track(area, from + wide / 2.0), 0.5);
        assert_eq!((super::along_track(area, from - 5.0), super::along_track(area, from + wide + 5.0)), (0.0, 1.0));
    }

    /// A line too long to be drawn large in one row (on a screen stood on end) takes two, each about half of it; one
    /// that fits keeps one; spaces where it breaks do not count toward a row's length.
    #[test]
    fn a_long_line_takes_more_rows_rather_than_shrinking() {
        let rooms = [100.0; 10];
        let blank = [false; 10];
        let (rows, fit) = super::rows_of(&rooms, &blank, 1200.0);
        assert_eq!((rows, fit), (vec![0..10], 1.0));
        let (rows, fit) = super::rows_of(&rooms, &blank, 500.0);
        assert_eq!((rows, fit), (vec![0..5, 5..10], 1.0));
        // Words with spaces between them: broken at a space, which takes no room at either end.
        let rooms = [300.0, 40.0, 300.0, 40.0, 300.0, 40.0, 300.0];
        let blank = [false, true, false, true, false, true, false];
        let (rows, fit) = super::rows_of(&rooms, &blank, 640.0);
        assert_eq!(rows.len(), 2);
        assert_eq!(fit, 640.0 / 640.0);
        // Too long for even the most rows: as many, and smaller.
        let (rows, fit) = super::rows_of(&[100.0; 30], &[false; 30], 300.0);
        assert_eq!(rows.len(), super::MOST_ROWS);
        assert!(fit < super::LEAST_FIT);
    }

    /// Chinese moves a character at a time; a word in English whole, its letters' times summed.
    #[test]
    fn words_move_whole_and_characters_alone() {
        let timed = |text: &str| text.chars().enumerate().map(|(k, ch)| (ch, k as f64, 0.5)).collect::<Vec<_>>();
        let said = |units: Vec<(String, f64, f64)>| units.into_iter().map(|(said, ..)| said).collect::<Vec<_>>();
        assert_eq!(said(super::units(timed("Come on, rain"))), ["Come", " ", "on,", " ", "rain"]);
        assert_eq!(said(super::units(timed("明天去找"))), ["明", "天", "去", "找"]);
        assert_eq!(said(super::units(timed("I love 你"))), ["I", " ", "love", " ", "你"]);
        let word = super::units(timed("rain")).remove(0);
        assert_eq!((word.1, word.2), (0.0, 2.0));
    }
}
