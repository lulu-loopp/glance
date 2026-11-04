//! Words written in a box Glance draws itself: what is written (UTF-16, as Windows hands it over and DirectWrite
//! counts it), the caret and the selection, and where in a laid-out text a point or a caret is. The box, its keys and
//! its input method are its window's (see `ask`, `beside`).

use windows::core::BOOL;
use windows::Win32::Graphics::DirectWrite::{IDWriteTextLayout, DWRITE_HIT_TEST_METRICS, DWRITE_LINE_METRICS, DWRITE_TEXT_METRICS};

/// The words being written: UTF-16, as Windows hands them over and
/// DirectWrite counts them; where the caret is and where the selection
/// started (the same, with none); how far the box is scrolled; and whether
/// it is to scroll to show the caret when next drawn.
#[derive(Default)]
pub struct Typing {
    pub text: Vec<u16>,
    pub caret: usize,
    pub anchor: usize,
    pub top: f32,
    pub follow: bool,
    /// The most it holds (UTF-16 units).
    most: usize,
}

pub fn high(unit: u16) -> bool {
    (0xD800..0xDC00).contains(&unit)
}

pub fn low(unit: u16) -> bool {
    (0xDC00..0xE000).contains(&unit)
}

impl Typing {
    /// Words holding no more than `most` units.
    pub fn most(most: usize) -> Self {
        Typing { most, ..Default::default() }
    }

    pub fn selection(&self) -> std::ops::Range<usize> {
        self.caret.min(self.anchor)..self.caret.max(self.anchor)
    }

    pub fn string(&self) -> String {
        String::from_utf16_lossy(&self.text)
    }

    /// The caret at `to`, the selection kept from where it started if `extend`.
    pub fn go(&mut self, to: usize, extend: bool) {
        self.caret = to.min(self.text.len());
        if !extend {
            self.anchor = self.caret;
        }
        self.follow = true;
    }

    /// From `at`, one character on (or back): a pair of surrogates is one.
    pub fn step(&self, at: usize, forward: bool) -> usize {
        if forward {
            if at >= self.text.len() {
                return at;
            }
            if high(self.text[at]) && self.text.get(at + 1).is_some_and(|&next| low(next)) { at + 2 } else { at + 1 }
        } else {
            if at == 0 {
                return 0;
            }
            if at >= 2 && low(self.text[at - 1]) && high(self.text[at - 2]) { at - 2 } else { at - 1 }
        }
    }

    /// `put` in place of the selection, as much of it as there is room for (no pair of surrogates cut).
    pub fn put(&mut self, put: &[u16]) {
        let selection = self.selection();
        let room = self.most.saturating_sub(self.text.len() - selection.len());
        let mut taken = put.len().min(room);
        if taken > 0 && taken < put.len() && high(put[taken - 1]) {
            taken -= 1;
        }
        self.text.splice(selection.clone(), put[..taken].iter().copied());
        self.go(selection.start + taken, false);
    }

    /// The selection taken away, or with none, the character before (or after) the caret.
    pub fn erase(&mut self, forward: bool) {
        if self.caret == self.anchor {
            self.anchor = self.step(self.caret, forward);
        }
        self.put(&[]);
    }
}

/// The input method's words and choices put at a caret in `hwnd` (its client area, px: where it is, how tall), in the
/// face `face` `size` px tall, as the words written there.
pub fn place_ime(hwnd: windows::Win32::Foundation::HWND, at: windows::Win32::Foundation::POINT, tall: i32, face: &str, size: i32) {
    use windows::Win32::Foundation::{POINT, RECT};
    use windows::Win32::Graphics::Gdi::{DEFAULT_CHARSET, LOGFONTW};
    use windows::Win32::UI::Input::Ime::{ImmGetContext, ImmReleaseContext, ImmSetCandidateWindow, ImmSetCompositionFontW, ImmSetCompositionWindow, CANDIDATEFORM, CFS_EXCLUDE, CFS_POINT, COMPOSITIONFORM};
    unsafe {
        let context = ImmGetContext(hwnd);
        if context.is_invalid() {
            return;
        }
        let _ = ImmSetCompositionWindow(context, &COMPOSITIONFORM { dwStyle: CFS_POINT, ptCurrentPos: at, ..Default::default() });
        let around = RECT { left: at.x, top: at.y, right: at.x + 1, bottom: at.y + tall };
        let _ = ImmSetCandidateWindow(context, &CANDIDATEFORM { dwIndex: 0, dwStyle: CFS_EXCLUDE, ptCurrentPos: POINT { x: around.left, y: around.bottom }, rcArea: around });
        let mut font = LOGFONTW { lfHeight: -size, lfWeight: 400, lfCharSet: DEFAULT_CHARSET, ..Default::default() };
        for (unit, to) in face.encode_utf16().zip(font.lfFaceName.iter_mut()) {
            *to = unit;
        }
        let _ = ImmSetCompositionFontW(context, &font);
        let _ = ImmReleaseContext(hwnd, context);
    }
}

/// Where in `layout` the point `(x, y)` is, as the position a caret put there would have.
pub fn at_point(layout: &IDWriteTextLayout, x: f32, y: f32) -> usize {
    let (mut trailing, mut inside, mut metrics) = (BOOL(0), BOOL(0), DWRITE_HIT_TEST_METRICS::default());
    let _ = unsafe { layout.HitTestPoint(x, y, &mut trailing, &mut inside, &mut metrics) };
    (metrics.textPosition + if trailing.as_bool() { metrics.length } else { 0 }) as usize
}

/// Where a caret at `at` is drawn in `layout`: its left, its top, and how tall.
pub fn caret_at(layout: &IDWriteTextLayout, at: usize) -> (f32, f32, f32) {
    let (mut x, mut y, mut metrics) = (0.0, 0.0, DWRITE_HIT_TEST_METRICS::default());
    let _ = unsafe { layout.HitTestTextPosition(at as u32, false, &mut x, &mut y, &mut metrics) };
    (x, metrics.top, metrics.height)
}

/// The line of `layout` that `at` is on: where it starts and ends (before its line break).
pub fn line_of(layout: &IDWriteTextLayout, at: usize) -> (usize, usize) {
    let mut count = 0;
    let _ = unsafe { layout.GetLineMetrics(None, &mut count) };
    let mut lines = vec![DWRITE_LINE_METRICS::default(); count as usize];
    let _ = unsafe { layout.GetLineMetrics(Some(&mut lines), &mut count) };
    let mut start = 0;
    for (index, line) in lines.iter().enumerate() {
        let end = start + line.length as usize;
        if at < end || index + 1 == lines.len() {
            return (start, end - line.newlineLength as usize);
        }
        start = end;
    }
    (0, 0)
}

pub fn height_of(layout: &IDWriteTextLayout) -> f32 {
    let mut metrics = DWRITE_TEXT_METRICS::default();
    let _ = unsafe { layout.GetMetrics(&mut metrics) };
    metrics.height
}

