//! A sum typed into search, worked out: numbers with + - * / ^ %, brackets, and the signs a phone or an input method
//! types for them (× ÷ − and the full-width ones). Worked out exactly, as fractions, as long as they fit, so that what
//! ten figures are added up to is what they are on paper; else (a root, a very large power) as floating point.

/// A sum's answer.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Answer {
    /// As it is copied: its figures alone.
    pub plain: String,
    /// As it is shown: "= 1,234.5", or "≈ 0.333333333333333" where that is not all of it.
    pub shown: String,
    /// The sum, written out ("128 × 6.5 + 40").
    pub said: String,
}

/// A number: a fraction in its lowest terms (what it is over more than 0), or floating point.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Number {
    Exact(i128, i128),
    Real(f64),
}

use Number::{Exact, Real};

/// The most a fraction's two parts are: what is over them can be taken ten times over as they are written out.
const MOST: i128 = i128::MAX / 10;

fn common(a: u128, b: u128) -> u128 {
    if b == 0 { a } else { common(b, a % b) }
}

/// `n` over `d`, where it fits.
fn exact(n: i128, d: i128) -> Option<Number> {
    let by = common(n.unsigned_abs(), d.unsigned_abs());
    let (n, d) = (n.checked_div(i128::try_from(by).ok()?)?, d.checked_div(i128::try_from(by).ok()?)?);
    let (n, d) = if d < 0 { (n.checked_neg()?, d.checked_neg()?) } else { (n, d) };
    (n.unsigned_abs() <= MOST as u128 && d <= MOST).then_some(Exact(n, d))
}

impl Number {
    /// A whole number or a decimal, as it is written ("1234.5").
    fn read(figures: &str) -> Option<Number> {
        let (whole, part) = figures.split_once('.').unwrap_or((figures, ""));
        let exactly = || {
            let n: i128 = format!("{whole}{part}").parse().ok()?;
            exact(n, 10i128.checked_pow(u32::try_from(part.len()).ok()?)?)
        };
        exactly().or_else(|| figures.parse().ok().map(Real))
    }

    fn real(self) -> f64 {
        match self {
            Exact(n, d) => n as f64 / d as f64,
            Real(value) => value,
        }
    }

    /// The two worked out exactly, where both are fractions and what they come to fits; else as floating point.
    fn with(self, other: Number, exactly: impl Fn(i128, i128, i128, i128) -> Option<Number>, really: impl Fn(f64, f64) -> f64) -> Number {
        let fits = match (self, other) {
            (Exact(a, b), Exact(c, d)) => exactly(a, b, c, d),
            _ => None,
        };
        fits.unwrap_or_else(|| Real(really(self.real(), other.real())))
    }

    fn plus(self, other: Number) -> Number {
        self.with(other, |a, b, c, d| exact(a.checked_mul(d)?.checked_add(c.checked_mul(b)?)?, b.checked_mul(d)?), |a, b| a + b)
    }

    fn times(self, other: Number) -> Number {
        self.with(other, |a, b, c, d| exact(a.checked_mul(c)?, b.checked_mul(d)?), |a, b| a * b)
    }

    fn minus(self) -> Number {
        match self {
            Exact(n, d) => Exact(-n, d),
            Real(value) => Real(-value),
        }
    }

    /// None over nothing.
    fn over(self, other: Number) -> Option<Number> {
        (other.real() != 0.0).then(|| self.with(other, |a, b, c, d| exact(a.checked_mul(d)?, b.checked_mul(c)?), |a, b| a / b))
    }

    /// What is left of it over `other` (its own sign): none over nothing.
    fn left_over(self, other: Number) -> Option<Number> {
        (other.real() != 0.0).then(|| self.with(other, |a, b, c, d| exact(a.checked_mul(d)? % c.checked_mul(b)?, b.checked_mul(d)?), |a, b| a % b))
    }

    /// To the power `other`: none where that is no number (a root of less than nothing, nothing over nothing).
    fn power(self, other: Number) -> Option<Number> {
        let exactly = match (self, other) {
            (Exact(n, d), Exact(up, root)) => rooted(n, root).zip(rooted(d, root)).and_then(|(n, d)| raised(n, d, up)),
            _ => None,
        };
        let value = exactly.unwrap_or_else(|| Real(self.real().powf(other.real())));
        value.real().is_finite().then_some(value)
    }
}

/// `n`'s root (2, its square root), where it is a whole number.
fn rooted(n: i128, root: i128) -> Option<i128> {
    if root == 1 || n == 0 || n == 1 {
        return Some(n);
    }
    if n < 0 {
        return None;
    }
    let power = u32::try_from(root).ok()?;
    let near = (n as f64).powf(1.0 / root as f64).round() as i128;
    (near - 1..=near + 1).find(|whole| whole.checked_pow(power) == Some(n))
}

/// `n` over `d`, to the whole power `up`, where it fits.
fn raised(n: i128, d: i128, up: i128) -> Option<Number> {
    let power = u32::try_from(up.unsigned_abs()).ok()?;
    let (n, d) = (n.checked_pow(power)?, d.checked_pow(power)?);
    if up < 0 {
        return (n != 0).then(|| exact(d, n))?;
    }
    exact(n, d)
}

/// How many figures of a number are shown where that is not all of it.
const FIGURES: usize = 15;

/// `value` written in figures alone, and whether that is all of it.
fn written(value: Number) -> (String, bool) {
    match value {
        Exact(n, d) => {
            let (whole, mut left) = (n.unsigned_abs() / d as u128, n.unsigned_abs() % d as u128);
            let d = d as u128;
            let mut figures = whole.to_string().into_bytes();
            let point = figures.len();
            // As many places as it has, up to so many; more, as many as make up the figures shown.
            let mut places = Vec::new();
            while left > 0 && places.len() < FIGURES {
                places.push(b'0' + (left * 10 / d) as u8);
                left = left * 10 % d;
            }
            let whole_of_it = left == 0;
            if !whole_of_it {
                let kept = if whole > 0 { FIGURES.saturating_sub(point).max(2) } else { places.iter().take_while(|&&place| place == b'0').count() + FIGURES };
                while left > 0 && places.len() <= kept {
                    places.push(b'0' + (left * 10 / d) as u8);
                    left = left * 10 % d;
                }
                let up = places.get(kept).is_some_and(|&next| next >= b'5');
                places.truncate(kept);
                figures.extend(&places);
                if up {
                    let nines = figures.iter().rev().take_while(|&&figure| figure == b'9').count();
                    let end = figures.len();
                    figures[end - nines..].fill(b'0');
                    match end.checked_sub(nines + 1) {
                        Some(at) => figures[at] += 1,
                        None => {
                            figures.insert(0, b'1');
                            return finished(figures, point + 1, n < 0, false);
                        }
                    }
                }
            } else {
                figures.extend(&places);
            }
            finished(figures, point, n < 0, whole_of_it)
        }
        Real(value) => {
            let rounded = format!("{value:.*e}", FIGURES - 1);
            let (figures, power) = rounded.split_once('e').unwrap_or((&rounded, "0"));
            let power: i32 = power.parse().unwrap_or(0);
            if (-7..21).contains(&power) {
                let places = (FIGURES as i32 - 1 - power).max(0) as usize;
                let text = format!("{value:.places$}");
                return (trimmed(&text).to_string(), false);
            }
            (format!("{}e{power}", trimmed(figures)), false)
        }
    }
}

/// A number's figures, its point after `point` of them, put together.
fn finished(figures: Vec<u8>, point: usize, less: bool, whole_of_it: bool) -> (String, bool) {
    let figures: String = figures.into_iter().map(char::from).collect();
    let (whole, places) = figures.split_at(point);
    let text = if places.is_empty() { whole.to_string() } else { format!("{whole}.{places}") };
    let text = trimmed(&text);
    (format!("{}{text}", if less && text != "0" { "-" } else { "" }), whole_of_it)
}

/// A number with no noughts at the end of its places.
fn trimmed(text: &str) -> &str {
    if text.contains('.') { text.trim_end_matches('0').trim_end_matches('.') } else { text }
}

/// Figures with their thousands apart.
fn apart(plain: &str) -> String {
    let (figures, power) = plain.split_once('e').map_or((plain, None), |(figures, power)| (figures, Some(power)));
    let (less, figures) = figures.strip_prefix('-').map_or(("", figures), |figures| ("-", figures));
    let (whole, places) = figures.split_once('.').map_or((figures, None), |(whole, places)| (whole, Some(places)));
    let mut said = less.to_string();
    for (at, figure) in whole.chars().enumerate() {
        if at > 0 && (whole.len() - at) % 3 == 0 {
            said.push(',');
        }
        said.push(figure);
    }
    if let Some(places) = places {
        said.push('.');
        said.push_str(places);
    }
    if let Some(power) = power {
        said.push_str("×10^");
        said.push_str(power);
    }
    said
}

/// A part of a sum.
#[derive(Clone, Debug, PartialEq)]
enum Part {
    Number(String),
    Sign(char),
}

/// What was typed, taken apart: none where it holds anything else than a sum does.
fn parts(typed: &str) -> Option<Vec<Part>> {
    let mut parts = Vec::new();
    let mut chars = typed.chars().map(plain).peekable();
    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => {}
            '+' | '-' | '*' | '/' | '^' | '%' | '(' | ')' => parts.push(Part::Sign(c)),
            c if c.is_ascii_digit() || c == '.' => {
                let mut figures = String::from(c);
                while let Some(c) = chars.next_if(|c| c.is_ascii_digit() || matches!(c, '.' | ',')) {
                    figures.push(c);
                }
                parts.push(Part::Number(figures));
            }
            _ => return None,
        }
    }
    Some(parts)
}

/// The sign another typed for it stands for.
fn plain(c: char) -> char {
    match c {
        '\u{FF01}'..='\u{FF5E}' => char::from_u32(c as u32 - 0xFEE0).unwrap_or(c),
        '×' | '✕' | '·' | '∙' => '*',
        '÷' => '/',
        '−' | '–' => '-',
        c => c,
    }
}

/// Whether `figures` are a number as one is written: with one point at most, and its thousands apart or not.
fn number(figures: &str) -> Option<Number> {
    let (whole, places) = figures.split_once('.').unwrap_or((figures, ""));
    if places.contains(['.', ',']) || (whole.is_empty() && places.is_empty()) {
        return None;
    }
    let mut groups = whole.split(',');
    let first = groups.next().unwrap_or_default();
    let thousands = whole.contains(',');
    if thousands && (!(1..=3).contains(&first.len()) || groups.any(|group| group.len() != 3)) {
        return None;
    }
    Number::read(&figures.replace(',', ""))
}

/// What a part of a sum comes to: a number, or so many per cent (of what it is added to).
#[derive(Clone, Copy)]
enum Value {
    Plain(Number),
    PerCent(Number),
}

impl Value {
    fn number(self) -> Number {
        match self {
            Value::Plain(number) => number,
            Value::PerCent(number) => number.times(Exact(1, 100)),
        }
    }
}

/// A sum read from its parts.
struct Reader<'a> {
    parts: &'a [Part],
    at: usize,
    /// How many of its numbers are put together: none, it is a number alone.
    put_together: usize,
}

impl Reader<'_> {
    fn sign(&self) -> Option<char> {
        match self.parts.get(self.at) {
            Some(Part::Sign(sign)) => Some(*sign),
            _ => None,
        }
    }

    /// Numbers added up, and taken away: so many per cent, of what comes before.
    fn added(&mut self) -> Option<Value> {
        let mut value = self.multiplied()?;
        while let Some(sign @ ('+' | '-')) = self.sign() {
            self.at += 1;
            self.put_together += 1;
            let by = self.multiplied()?;
            let before = value.number();
            let by = match by {
                Value::Plain(number) => number,
                Value::PerCent(_) => before.times(by.number()),
            };
            value = Value::Plain(before.plus(if sign == '-' { by.minus() } else { by }));
        }
        Some(value)
    }

    fn multiplied(&mut self) -> Option<Value> {
        let mut value = self.signed()?;
        loop {
            let sign = match self.sign() {
                Some(sign @ ('*' | '/' | '%')) => {
                    self.at += 1;
                    sign
                }
                // (A number before a bracket, times it.)
                Some('(') => '*',
                _ => return Some(value),
            };
            self.put_together += 1;
            let (before, by) = (value.number(), self.signed()?.number());
            value = Value::Plain(match sign {
                '*' => before.times(by),
                '/' => before.over(by)?,
                _ => before.left_over(by)?,
            });
        }
    }

    fn signed(&mut self) -> Option<Value> {
        match self.sign() {
            Some('-') => {
                self.at += 1;
                Some(match self.signed()? {
                    Value::Plain(number) => Value::Plain(number.minus()),
                    Value::PerCent(number) => Value::PerCent(number.minus()),
                })
            }
            Some('+') => {
                self.at += 1;
                self.signed()
            }
            _ => self.raised(),
        }
    }

    fn raised(&mut self) -> Option<Value> {
        let value = self.per_cent()?;
        if self.sign() != Some('^') {
            return Some(value);
        }
        self.at += 1;
        self.put_together += 1;
        let by = self.signed()?.number();
        Some(Value::Plain(value.number().power(by)?))
    }

    /// A number, or so many per cent: "%" after it, with no number after that (between two, what is left over).
    fn per_cent(&mut self) -> Option<Value> {
        let value = self.alone()?;
        if self.sign() == Some('%') && !matches!(self.parts.get(self.at + 1), Some(Part::Number(_) | Part::Sign('('))) {
            self.at += 1;
            return Some(Value::PerCent(value.number()));
        }
        Some(value)
    }

    fn alone(&mut self) -> Option<Value> {
        match self.parts.get(self.at)? {
            Part::Number(figures) => {
                self.at += 1;
                Some(Value::Plain(number(figures)?))
            }
            Part::Sign('(') => {
                self.at += 1;
                let value = self.added()?;
                // (Not yet closed at the end of what is typed, closed there.)
                match self.sign() {
                    Some(')') => self.at += 1,
                    _ if self.at == self.parts.len() => {}
                    _ => return None,
                }
                Some(Value::Plain(value.number()))
            }
            Part::Sign(_) => None,
        }
    }
}

/// Whether `typed` is a date ("2026-11-22", "2026/11/22"), not a sum.
fn a_date(typed: &str) -> bool {
    ['-', '/'].into_iter().any(|apart| {
        let figures: Vec<&str> = typed.split(apart).map(str::trim).collect();
        let [year, month, day] = figures.as_slice() else { return false };
        let number = |text: &str| text.parse::<u32>().ok().filter(|_| text.bytes().all(|c| c.is_ascii_digit()));
        year.len() == 4 && number(year).is_some() && number(month).is_some_and(|month| (1..=12).contains(&month)) && number(day).is_some_and(|day| (1..=31).contains(&day))
    })
}

/// Whether `figures` are a number someone has (a telephone's, a file's), not one to work with: a nought before others.
fn called(figures: &str) -> bool {
    figures.len() > 1 && figures.starts_with('0') && !figures.starts_with("0.")
}

/// What `typed` comes to, where it is a sum: numbers put together, not a number alone, a date, or numbers things are
/// called by. After "=", whatever it is of those. A sum not yet typed to its end is worked out as far as it goes.
pub fn worked_out(typed: &str) -> Option<Answer> {
    let typed = typed.trim();
    let asked = typed.strip_prefix(['=', '＝']);
    let typed = asked.unwrap_or(typed).trim();
    let mut parts = parts(typed)?;
    while matches!(parts.last(), Some(Part::Sign('+' | '-' | '*' | '/' | '^' | '('))) {
        parts.pop();
    }
    if asked.is_none() && (a_date(typed) || parts.iter().any(|part| matches!(part, Part::Number(figures) if called(figures)))) {
        return None;
    }
    let mut reader = Reader { parts: &parts, at: 0, put_together: 0 };
    let value = reader.added()?.number();
    let sum = asked.is_some() || reader.put_together > 0;
    if !sum || reader.at != parts.len() || !value.real().is_finite() {
        return None;
    }
    let (plain, all) = written(value);
    Some(Answer { shown: format!("{} {}", if all { "=" } else { "≈" }, apart(&plain)), plain, said: said(&parts) })
}

/// A sum's parts, written out.
fn said(parts: &[Part]) -> String {
    let mut said = String::new();
    // (Whether what comes before ends a number: a sign after it then puts two together.)
    let mut ended = false;
    for (at, part) in parts.iter().enumerate() {
        match part {
            Part::Number(figures) => {
                said.push_str(figures);
                ended = true;
            }
            Part::Sign('(') => {
                said.push_str(if ended { " × (" } else { "(" });
                ended = false;
            }
            Part::Sign(')') => {
                said.push(')');
                ended = true;
            }
            Part::Sign('%') if !matches!(parts.get(at + 1), Some(Part::Number(_) | Part::Sign('('))) => said.push('%'),
            Part::Sign('^') => {
                said.push('^');
                ended = false;
            }
            Part::Sign(sign) => {
                let sign = match sign {
                    '*' => '×',
                    '/' => '÷',
                    '-' => '−',
                    sign => *sign,
                };
                if ended {
                    said.extend([' ', sign, ' ']);
                } else {
                    said.push(sign);
                }
                ended = false;
            }
        }
    }
    said
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shown(typed: &str) -> String {
        worked_out(typed).map_or_else(String::new, |answer| answer.shown)
    }

    #[test]
    fn sums_are_worked_out() {
        for (typed, answer) in [
            ("128*6.5+40", "= 872"),
            ("1+2*3", "= 7"),
            ("(1+2)*3", "= 9"),
            ("2^3^2", "= 512"),
            ("-2^2", "= -4"),
            ("2^-1", "= 0.5"),
            ("4^0.5", "= 2"),
            ("0.25^0.5", "= 0.5"),
            ("-4/2", "= -2"),
            ("1 - -2", "= 3"),
            ("2(3+4)", "= 14"),
            ("(1+2)(3+4)", "= 21"),
            ("（1234+5678）×9", "= 62,208"),
            ("10÷4", "= 2.5"),
            ("1,234+1", "= 1,235"),
            // Exactly: no binary rounding shows.
            ("0.1+0.2", "= 0.3"),
            ("0.3-0.1*3", "= 0"),
            ("1/3*3", "= 1"),
            ("2^100", "= 1,267,650,600,228,229,401,496,703,205,376"),
            // Not all of it.
            ("10/3", "≈ 3.33333333333333"),
            ("1/3", "≈ 0.333333333333333"),
            ("2/3", "≈ 0.666666666666667"),
            ("10/3*10^18", "≈ 3,333,333,333,333,333,333.33"),
            ("999999999999999999/3.0000001", "≈ 333,333,322,222,222,592.26"),
            ("2^0.5", "≈ 1.4142135623731"),
            ("10^60", "≈ 1×10^60"),
            ("(1-1/10^20)*10", "≈ 10"),
        ] {
            assert_eq!(shown(typed), answer, "{typed}");
        }
        assert_eq!(worked_out("(1234+5678)*9").unwrap().plain, "62208");
        assert_eq!(worked_out("1234 × -5").unwrap().plain, "-6170");
    }

    /// Per cent, of what it is added to or taken from, and by itself; between two numbers, what is left over.
    #[test]
    fn per_cent_is_of_what_comes_before() {
        for (typed, answer) in [("200+15%", "= 230"), ("200-15%", "= 170"), ("200*15%", "= 30"), ("50%+1", "= 1.5"), ("100+10%+10%", "= 121"), ("5 % 2", "= 1"), ("-5 % 3", "= -2"), ("7.5%2", "= 1.5"), ("10%(4)", "= 2"), ("=15%", "= 0.15")] {
            assert_eq!(shown(typed), answer, "{typed}");
        }
    }

    /// What is not a sum is not taken for one: a number alone, a date, a number called, words, nothing over nothing;
    /// after "=", any of those numbers is worked out.
    #[test]
    fn what_is_not_a_sum_is_left() {
        for typed in ["", "42", "-5", "(5)", "50%", "v1.2", "1.2.3-4", "2026-11-22", "2026/1/2", "0571-8888", "010-12345678", "1/0", "5 % 0", "0^-1", "(-8)^0.5", "1 2", "1+*2", ")1+2", "1,2+3", "c++", "1920x1080", "+", "..+1", "1+2)"] {
            assert_eq!(worked_out(typed), None, "{typed}");
        }
        assert_eq!(shown("=42"), "= 42");
        assert_eq!(shown("=2026-11-22"), "= 1,993");
        assert_eq!(shown("＝007+1"), "= 8");
        assert_eq!(shown("2026-13-22"), "= 1,991");
        assert_eq!(shown("0.5+1"), "= 1.5");
    }

    /// Typed on, a sum is worked out as far as it goes: a sign with nothing after it yet, brackets not yet closed.
    #[test]
    fn a_sum_is_worked_out_as_it_is_typed() {
        assert_eq!(shown("128*6.5+"), "= 832");
        assert_eq!(shown("128*(6.5+"), "= 832");
        assert_eq!(shown("2*(3+4"), "= 14");
        assert_eq!(shown("128*"), "");
        assert_eq!(worked_out("2*(3+4").unwrap().said, "2 × (3 + 4");
    }

    #[test]
    fn a_sum_is_written_out() {
        for (typed, out) in [("128*6.5+40", "128 × 6.5 + 40"), ("200+15%", "200 + 15%"), ("-2^2-1", "−2^2 − 1"), ("2(3+4)", "2 × (3 + 4)"), ("5%2", "5 % 2"), ("3*-2", "3 × −2")] {
            assert_eq!(worked_out(typed).unwrap().said, out, "{typed}");
        }
    }
}
