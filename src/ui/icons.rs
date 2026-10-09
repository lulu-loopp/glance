//! The icons the panel's small layouts mark their readings with: Lucide's
//! (lucide.dev, ISC licence: licenses/Lucide-ISC.txt), as the SVG path data
//! it publishes on its 24-unit grid, read into figures any canvas can draw.

/// An icon, by what it stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    Cpu,
    Gpu,
    Memory,
    Disk,
    Network,
    Game,
    Pin,
    Close,
    Settings,
    Check,
    Pointer,
    Panel,
    Restore,
    Update,
    Quit,
    Layers,
}

impl Icon {
    /// Lucide's path data for it ("cpu", "gpu", "memory-stick", "hard-drive",
    /// "arrow-down-up", "gamepad-2", "pin", "x", "settings", "check",
    /// "mouse-pointer-2", "panel-right", "undo-2", "refresh-cw", "power", "layers";
    /// lucide-static 1.53.0), its
    /// rectangles and circles written as paths.
    fn data(self) -> &'static [&'static str] {
        match self {
            Icon::Cpu => &[
                "M12 20v2", "M12 2v2", "M17 20v2", "M17 2v2", "M2 12h2", "M2 17h2", "M2 7h2", "M20 12h2", "M20 17h2", "M20 7h2", "M7 20v2", "M7 2v2",
                "M6 4h12a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2z",
                "M9 8h6a1 1 0 0 1 1 1v6a1 1 0 0 1-1 1H9a1 1 0 0 1-1-1V9a1 1 0 0 1 1-1z",
            ],
            Icon::Gpu => &[
                "M2 17h18a2 2 0 0 0 2-2V7a2 2 0 0 0-2-2H2", "M2 21V3", "M7 17v3a1 1 0 0 0 1 1h5a1 1 0 0 0 1-1v-3",
                "M18 11a2 2 0 1 1-4 0a2 2 0 1 1 4 0z", "M10 11a2 2 0 1 1-4 0a2 2 0 1 1 4 0z",
            ],
            Icon::Memory => &[
                "M12 12v-2", "M12 18v-2", "M16 12v-2", "M16 18v-2", "M2 11h1.5", "M20 18v-2", "M20.5 11H22", "M4 18v-2", "M8 12v-2", "M8 18v-2",
                "M4 6h16a2 2 0 0 1 2 2v6a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2z",
            ],
            Icon::Disk => &[
                "M10 16h.01",
                "M2.212 11.577a2 2 0 0 0-.212.896V18a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-5.527a2 2 0 0 0-.212-.896L18.55 5.11A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z",
                "M21.946 12.013H2.054", "M6 16h.01",
            ],
            Icon::Network => &["m3 16 4 4 4-4", "M7 20V4", "m21 8-4-4-4 4", "M17 4v16"],
            Icon::Pin => &[
                "M12 17v5",
                "M9 10.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.76V7a1 1 0 0 1 1-1 2 2 0 0 0 0-4H8a2 2 0 0 0 0 4 1 1 0 0 1 1 1z",
            ],
            Icon::Close => &["M18 6 6 18", "m6 6 12 12"],
            Icon::Check => &["M20 6 9 17l-5-5"],
            Icon::Panel => &["M5 3h14a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2z", "M15 3v18"],
            Icon::Restore => &["M9 14 4 9l5-5", "M4 9h10.5a5.5 5.5 0 0 1 5.5 5.5a5.5 5.5 0 0 1-5.5 5.5H11"],
            Icon::Update => &["M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8", "M21 3v5h-5", "M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16", "M8 16H3v5"],
            Icon::Quit => &["M12 2v10", "M18.4 6.6a9 9 0 1 1-12.77.04"],
            Icon::Layers => &[
                "M12.83 2.18a2 2 0 0 0-1.66 0L2.6 6.08a1 1 0 0 0 0 1.83l8.58 3.91a2 2 0 0 0 1.66 0l8.58-3.9a1 1 0 0 0 0-1.83z",
                "M2 12a1 1 0 0 0 .58.91l8.6 3.91a2 2 0 0 0 1.65 0l8.58-3.9A1 1 0 0 0 22 12",
                "M2 17a1 1 0 0 0 .58.91l8.6 3.91a2 2 0 0 0 1.65 0l8.58-3.9A1 1 0 0 0 22 17",
            ],
            Icon::Pointer => &["M4.037 4.688a.495.495 0 0 1 .651-.651l16 6.5a.5.5 0 0 1-.063.947l-6.124 1.58a2 2 0 0 0-1.438 1.435l-1.579 6.126a.5.5 0 0 1-.947.063z"],
            Icon::Settings => &[
                "M9.671 4.136a2.34 2.34 0 0 1 4.659 0 2.34 2.34 0 0 0 3.319 1.915 2.34 2.34 0 0 1 2.33 4.033 2.34 2.34 0 0 0 0 3.831 2.34 2.34 0 0 1-2.33 4.033 2.34 2.34 0 0 0-3.319 1.915 2.34 2.34 0 0 1-4.659 0 2.34 2.34 0 0 0-3.32-1.915 2.34 2.34 0 0 1-2.33-4.033 2.34 2.34 0 0 0 0-3.831A2.34 2.34 0 0 1 6.35 6.051a2.34 2.34 0 0 0 3.319-1.915",
                "M15 12a3 3 0 1 1-6 0a3 3 0 1 1 6 0z",
            ],
            Icon::Game => &[
                "M6 11h4", "M8 9v4", "M15 12h.01", "M18 10h.01",
                "M17.32 5H6.68a4 4 0 0 0-3.978 3.59c-.006.052-.01.101-.017.152C2.604 9.416 2 14.456 2 16a3 3 0 0 0 3 3c1 0 1.5-.5 2-1l1.414-1.414A2 2 0 0 1 9.828 16h4.344a2 2 0 0 1 1.414.586L17 18c.5.5 1 1 2 1a3 3 0 0 0 3-3c0-1.545-.604-6.584-.685-7.258-.007-.05-.011-.1-.017-.151A4 4 0 0 0 17.32 5z",
            ],
        }
    }

    /// Its figures, on the 24-unit grid.
    pub fn figures(self) -> Vec<Figure> {
        self.data().iter().flat_map(|path| parse(path)).collect()
    }
}

/// How an icon is stroked: this wide on its 24-unit grid, with round ends
/// and joins.
pub const STROKE: f32 = 2.0;

/// A run of segments from a starting point, closed back to it or not.
#[derive(Clone, Debug, PartialEq)]
pub struct Figure {
    pub start: (f32, f32),
    pub segments: Vec<Segment>,
    pub closed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Segment {
    Line((f32, f32)),
    /// An elliptical arc to `to`, as SVG's: its radii, the ellipse's
    /// rotation (degrees), whether it takes the larger arc, and whether it
    /// turns clockwise (in a y-down space).
    Arc { to: (f32, f32), radii: (f32, f32), rotation: f32, large: bool, clockwise: bool },
    Cubic((f32, f32), (f32, f32), (f32, f32)),
}

/// SVG path data read into figures: the commands M, L, H, V, A, C and Z,
/// each absolute or relative (lower case), a command's arguments repeated
/// for as many segments as they make.
pub fn parse(data: &str) -> Vec<Figure> {
    let mut tokens = Tokens { rest: data };
    let mut figures: Vec<Figure> = Vec::new();
    let mut at = (0.0f32, 0.0f32);
    let mut start = at;
    let mut command = None;
    while let Some(next) = tokens.command().or(command) {
        let relative = next.is_ascii_lowercase();
        let base = if relative { at } else { (0.0, 0.0) };
        let point = |tokens: &mut Tokens| Some((base.0 + tokens.number()?, base.1 + tokens.number()?));
        match next.to_ascii_uppercase() {
            'M' => {
                let Some(to) = point(&mut tokens) else { break };
                at = to;
                start = to;
                figures.push(Figure { start: to, segments: Vec::new(), closed: false });
                // Pairs after a move are lines.
                command = Some(if relative { 'l' } else { 'L' });
                continue;
            }
            'L' => {
                let Some(to) = point(&mut tokens) else { break };
                at = to;
                push(&mut figures, start, Segment::Line(to));
            }
            'H' => {
                let Some(x) = tokens.number() else { break };
                at = (base.0 + x, at.1);
                push(&mut figures, start, Segment::Line(at));
            }
            'V' => {
                let Some(y) = tokens.number() else { break };
                at = (at.0, base.1 + y);
                push(&mut figures, start, Segment::Line(at));
            }
            'A' => {
                let (Some(rx), Some(ry), Some(rotation), Some(large), Some(clockwise)) = (tokens.number(), tokens.number(), tokens.number(), tokens.flag(), tokens.flag()) else { break };
                let Some(to) = point(&mut tokens) else { break };
                at = to;
                push(&mut figures, start, Segment::Arc { to, radii: (rx, ry), rotation, large, clockwise });
            }
            'C' => {
                let (Some(c1), Some(c2), Some(to)) = (point(&mut tokens), point(&mut tokens), point(&mut tokens)) else { break };
                at = to;
                push(&mut figures, start, Segment::Cubic(c1, c2, to));
            }
            'Z' => {
                if let Some(figure) = figures.last_mut() {
                    figure.closed = true;
                }
                at = start;
                command = None;
                continue;
            }
            _ => break,
        }
        command = Some(next);
    }
    figures
}

/// Adds a segment to the figure under way, or begins one where the last
/// ended (a segment after a close, with no move).
fn push(figures: &mut Vec<Figure>, start: (f32, f32), segment: Segment) {
    match figures.last_mut().filter(|figure| !figure.closed) {
        Some(figure) => figure.segments.push(segment),
        None => figures.push(Figure { start, segments: vec![segment], closed: false }),
    }
}

/// SVG path data, read a command letter, a number or a flag at a time.
struct Tokens<'a> {
    rest: &'a str,
}

impl Tokens<'_> {
    fn skip(&mut self) {
        self.rest = self.rest.trim_start_matches(|c: char| c.is_whitespace() || c == ',');
    }

    fn command(&mut self) -> Option<char> {
        self.skip();
        let c = self.rest.chars().next().filter(char::is_ascii_alphabetic)?;
        self.rest = &self.rest[1..];
        Some(c)
    }

    /// A number: a sign, digits, one point at most, an exponent; another
    /// number may follow at once (".5.5" is two, "2-2" is two).
    fn number(&mut self) -> Option<f32> {
        self.skip();
        let bytes = self.rest.as_bytes();
        let mut end = 0;
        if matches!(bytes.first(), Some(b'+' | b'-')) {
            end = 1;
        }
        let mut point = false;
        while let Some(&b) = bytes.get(end) {
            match b {
                b'0'..=b'9' => end += 1,
                b'.' if !point => {
                    point = true;
                    end += 1
                }
                b'e' | b'E' if end > 0 => {
                    end += 1;
                    if matches!(bytes.get(end), Some(b'+' | b'-')) {
                        end += 1;
                    }
                }
                _ => break,
            }
        }
        let value = self.rest[..end].parse().ok()?;
        self.rest = &self.rest[end..];
        Some(value)
    }

    /// An arc's flag: one digit, 0 or 1, which nothing need separate from
    /// what follows.
    fn flag(&mut self) -> Option<bool> {
        self.skip();
        let flag = match self.rest.as_bytes().first()? {
            b'0' => false,
            b'1' => true,
            _ => return None,
        };
        self.rest = &self.rest[1..];
        Some(flag)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_moves_lines_and_closes() {
        let figures = parse("M6 4h12v2H6z M2 12h2");
        assert_eq!(figures.len(), 2);
        assert_eq!(figures[0].start, (6.0, 4.0));
        assert_eq!(figures[0].segments, vec![Segment::Line((18.0, 4.0)), Segment::Line((18.0, 6.0)), Segment::Line((6.0, 6.0))]);
        assert!(figures[0].closed);
        assert_eq!(figures[1].segments, vec![Segment::Line((4.0, 12.0))]);
        assert!(!figures[1].closed);
    }

    #[test]
    fn reads_relative_commands_and_packed_numbers() {
        // After a relative move, pairs are relative lines; "-7 7-7-7" is
        // four numbers.
        let figures = parse("m19 12-7 7-7-7");
        assert_eq!(figures[0].start, (19.0, 12.0));
        assert_eq!(figures[0].segments, vec![Segment::Line((12.0, 19.0)), Segment::Line((5.0, 12.0))]);
        assert_eq!(parse("M10 16h.01")[0].segments, vec![Segment::Line((10.01, 16.0))]);
    }

    #[test]
    fn reads_arcs() {
        let figures = parse("M14 4a2 2 0 0 1-4 0");
        assert_eq!(figures[0].segments, vec![Segment::Arc { to: (10.0, 4.0), radii: (2.0, 2.0), rotation: 0.0, large: false, clockwise: true }]);
        // Flags packed against what follows.
        let packed = parse("M14 4a4 4 0 1 1-4 0");
        assert!(matches!(packed[0].segments[0], Segment::Arc { large: true, clockwise: true, .. }));
    }

    #[test]
    fn every_icon_reads_whole() {
        for icon in [Icon::Cpu, Icon::Gpu, Icon::Memory, Icon::Disk, Icon::Network, Icon::Game, Icon::Pin, Icon::Close, Icon::Settings, Icon::Check, Icon::Pointer, Icon::Panel, Icon::Restore, Icon::Update, Icon::Quit, Icon::Layers] {
            let figures = icon.figures();
            assert!(!figures.is_empty(), "{icon:?}");
            // Every point on the grid.
            for figure in &figures {
                let (x, y) = figure.start;
                assert!((0.0..=24.0).contains(&x) && (0.0..=24.0).contains(&y), "{icon:?} {figure:?}");
            }
        }
        assert_eq!(Icon::Cpu.figures().len(), 14);
        assert_eq!(Icon::Gpu.figures().len(), 5);
        // The gear's twelve arcs, one command letter for all of them.
        assert_eq!(Icon::Settings.figures()[0].segments.len(), 12);
    }
}
