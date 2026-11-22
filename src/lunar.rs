//! The Chinese (lunar) calendar: the lunar day of a day, the solar term or traditional festival it is, as the time
//! component tells them where it is asked to (its "lunar" item). Told from the Hong Kong Observatory's conversion tables,
//! 1901 to 2100, not computed: a new moon minutes from midnight in China is a day's difference (2027's first month begins
//! on February 6, its new moon at 23:56).

use crate::todo::Day;
use crate::ui::text::Lang;

/// The first lunar year, and Gregorian year, of `YEARS`.
const FIRST: i32 = 1901;

/// Each lunar year, and the solar terms of the Gregorian year of that number.
/// Its first: which of its months, in their order, the leap one among them, have 30 days and not 29 (a bit each, from the
/// lowest, 13 of them); the number of its leap month (4 bits, 0: none); how many days after January 1 it begins.
/// Its second: how many days after `TERMS_FROM` each term falls in its month (2 bits each, from the lowest).
#[rustfmt::skip]
const YEARS: [(u32, u64); 200] = [
    (0x620752, 0x6aaaa6aa9a5a), (0x4c0ea5, 0xaaaaaabaaa6a), (0x38b64a, 0xaaabbabbafaa), (0x5c064b, 0x5aa665a65aab),
    (0x440a9b, 0x6aaaa6aa9a5a), (0x309556, 0xaaaaaaaaaa6a), (0x56056a, 0xaaabbabbafaa), (0x400b59, 0x5aa665a65aab),
    (0x2a5752, 0x6aaaa6aa9a5a), (0x500752, 0xaaaaaaaaaa6a), (0x3adb25, 0xaaabbabbafaa), (0x600b25, 0x5aa665a65aab),
    (0x480a4b, 0x6aaaa6aa9a56), (0x32b4ab, 0xaaaaaaaa9a5a), (0x5802ad, 0xaaabaabaaeaa), (0x42056b, 0x569665a65aaa),
    (0x2c4b69, 0x5aa6a6a69a56), (0x520da9, 0x6aaaaaaa9a5a), (0x3efd92, 0xaaabaabaaeaa), (0x640e92, 0x569665a65aaa),
    (0x4c0d25, 0x5aa6a6a65a56), (0x36ba4d, 0x6aaaaaaa9a5a), (0x5c0a56, 0xaaabaabaaa6a), (0x4602b6, 0x569665a65aaa),
    (0x2e95b5, 0x5aa6a6a65a56), (0x5606d4, 0x6aaaa6aa9a5a), (0x400ea9, 0xaaaaaabaaa6a), (0x2c5e92, 0x555665665aaa),
    (0x500e92, 0x5aa665a65a56), (0x3acd26, 0x6aaaa6aa9a5a), (0x5e052b, 0xaaaaaabaaa6a), (0x480a57, 0x555665665aaa),
    (0x32b2b6, 0x5aa665a65a56), (0x580b5a, 0x6aaaa6aa9a5a), (0x4406d4, 0xaaaaaaaaaa6a), (0x2e6ec9, 0x555665665aaa),
    (0x520749, 0x5aa665a65a56), (0x3cf693, 0x6aaaa6aa9a5a), (0x620a93, 0xaaaaaaaaaa6a), (0x4c052b, 0x555665665aaa),
    (0x34ca5b, 0x5aa665a65a56), (0x5a0aad, 0x6aaaa6aa9a5a), (0x46056a, 0xaaaaaaaaaa6a), (0x309b55, 0x555665655aaa),
    (0x560ba4, 0x569665a65a56), (0x400b49, 0x6aa6a6aa9a56), (0x2a5a93, 0xaaaaaaaa9a5a), (0x500a95, 0x5556556559aa),
    (0x38f52d, 0x569665a65a55), (0x5e0536, 0x6aa6a6a65a56), (0x480aad, 0xaaaaaaaa9a5a), (0x34b5aa, 0x5556556559aa),
    (0x5805b2, 0x569665a65a55), (0x420da5, 0x5aa6a6a65a56), (0x2e7d4a, 0x6aaaa6aa9a5a), (0x540d4a, 0x5556556555aa),
    (0x3d0a95, 0x569665a65a55), (0x600a97, 0x5aa665a65a56), (0x4c0556, 0x6aaaa6aa9a5a), (0x36cab5, 0x55555565556a),
    (0x5a0ad5, 0x555665665a55), (0x4606d2, 0x5aa665a65a56), (0x308ea5, 0x6aaaa6aa9a5a), (0x560ea5, 0x55555565556a),
    (0x40064a, 0x555665665a55), (0x286c97, 0x5aa665a65a56), (0x4e0a9b, 0x6aaaa6aa9a5a), (0x3af55a, 0x55555555556a),
    (0x5e056a, 0x555665665a55), (0x480b69, 0x5aa665a65a56), (0x34b752, 0x6aaaa6aa9a5a), (0x5a0b52, 0x55555555556a),
    (0x420b25, 0x555665655a55), (0x2c964b, 0x5aa665a65a56), (0x520a4b, 0x6aa6a6aa9a5a), (0x3d14ab, 0x55555555456a),
    (0x6002ad, 0x555655655a55), (0x4a056d, 0x5a9665a65a56), (0x36cb69, 0x6aa6a6a69a5a), (0x5c0da9, 0x55555555456a),
    (0x460d92, 0x555655655a55), (0x309d25, 0x569665a65a56), (0x560d25, 0x6aa6a6a65a56), (0x415a4d, 0x55555155455a),
    (0x640a56, 0x555655655955), (0x4e02b6, 0x569665a65a55), (0x38c5b5, 0x5aa6a5a65a56), (0x5e06d5, 0x15555155455a),
    (0x480ea9, 0x555555655555), (0x34be92, 0x569665665a55), (0x5a0e92, 0x5aa665a65a56), (0x440d26, 0x15555155455a),
    (0x2c6a56, 0x555555655515), (0x500a57, 0x555665665a55), (0x3d14d6, 0x5aa665a65a56), (0x62035a, 0x15555155455a),
    (0x4a06d5, 0x555555555515), (0x36b6c9, 0x555665665a55), (0x5c0749, 0x5aa665a65a56), (0x460693, 0x15555155455a),
    (0x2e952b, 0x555555555515), (0x54052b, 0x555665665a55), (0x3e0a5b, 0x5aa665a65a56), (0x2a555a, 0x15555155455a),
    (0x4e056a, 0x555555555515), (0x38fb55, 0x555655655a55), (0x600ba4, 0x5aa665a65a56), (0x4a0b49, 0x15515155455a),
    (0x32ba93, 0x555555554515), (0x580a95, 0x555655655a55), (0x42052d, 0x5a9665a65a56), (0x2c8aad, 0x15515151455a),
    (0x500ab5, 0x555551554515), (0x3d35aa, 0x555655655a55), (0x6205d2, 0x569665a65a56), (0x4c0da5, 0x155151510556),
    (0x36dd4a, 0x555551554505), (0x5c0d4a, 0x555655655955), (0x460c95, 0x569665665a55), (0x30952e, 0x155110510556),
    (0x540556, 0x155551554505), (0x3e0ab5, 0x555555655555), (0x2a55b2, 0x569665665a55), (0x5006d2, 0x055110510556),
    (0x38cea5, 0x155551554505), (0x5e0725, 0x555555555515), (0x48064b, 0x555665665a55), (0x32ac97, 0x055110510556),
    (0x560cab, 0x155551554505), (0x42055a, 0x555555555515), (0x2c6ad6, 0x555665665a55), (0x520b69, 0x055110510556),
    (0x3d7752, 0x155551554505), (0x620b52, 0x555555555515), (0x4c0b25, 0x555655655a55), (0x36da4b, 0x055110510556),
    (0x5a0a4b, 0x155551554505), (0x4404ab, 0x555555555515), (0x2ea55b, 0x555655655a55), (0x5405ad, 0x055110510556),
    (0x3e0b6a, 0x155151514505), (0x2a5b52, 0x555555554515), (0x500d92, 0x555655655a55), (0x3afd25, 0x054110510556),
    (0x5e0d25, 0x155151510505), (0x480a55, 0x555551554515), (0x32b4ad, 0x555655655a55), (0x5804b6, 0x014110110556),
    (0x4005b5, 0x155110510501), (0x2c6daa, 0x555551554505), (0x520ec9, 0x555555655555), (0x3f1e92, 0x014110110555),
    (0x620e92, 0x155110510501), (0x4c0d26, 0x555551554505), (0x36ca56, 0x555555555555), (0x5a0a57, 0x014110110555),
    (0x440556, 0x055110510501), (0x2e86d5, 0x155551554505), (0x540755, 0x555555555555), (0x400749, 0x000110110555),
    (0x286e93, 0x055110510501), (0x4e0693, 0x155551554505), (0x38f52b, 0x555555555515), (0x5e052b, 0x000110110555),
    (0x460a5b, 0x055110510501), (0x32b55a, 0x155551554505), (0x58056a, 0x555555555515), (0x420b65, 0x000100100555),
    (0x2c974a, 0x055110510501), (0x520b4a, 0x155151514505), (0x3d1a95, 0x555555555515), (0x620a95, 0x000100100555),
    (0x4a052d, 0x054110510501), (0x34caad, 0x155151514505), (0x5a0ab5, 0x555551554515), (0x4605aa, 0x000100100555),
    (0x2e8ba5, 0x054110510501), (0x540da5, 0x155150510505), (0x400d4a, 0x555551554515), (0x2a7c95, 0x000100100555),
    (0x4e0c96, 0x014110110501), (0x38f94e, 0x155110510505), (0x5e0556, 0x555551554505), (0x480ab5, 0x000000100055),
    (0x32b5b2, 0x014110110500), (0x5806d2, 0x155110510501), (0x420ea5, 0x555551554505), (0x2e8e4a, 0x000000000055),
    (0x50068b, 0x014110110500), (0x3b0c97, 0x055110510501), (0x6004ab, 0x155551554505), (0x4a055b, 0x000000000055),
    (0x34cad6, 0x000110110500), (0x5a0b6a, 0x055110510501), (0x460752, 0x155551554505), (0x309725, 0x000000000015),
    (0x540b45, 0x000100110500), (0x3e0a8b, 0x055110510501), (0x28549b, 0x155551554505), (0x4e0cab, 0x555555555515),
];

/// The earliest day of its month each solar term falls on, two in each month from January on.
const TERMS_FROM: [u32; 24] = [4, 19, 3, 18, 4, 19, 4, 19, 4, 20, 4, 20, 6, 22, 6, 22, 6, 22, 7, 22, 6, 21, 6, 21];

const TERMS: [(&str, &str); 24] = [
    ("小寒", "Minor Cold"),
    ("大寒", "Major Cold"),
    ("立春", "Start of Spring"),
    ("雨水", "Rain Water"),
    ("惊蛰", "Awakening of Insects"),
    ("春分", "Spring Equinox"),
    ("清明", "Qingming"),
    ("谷雨", "Grain Rain"),
    ("立夏", "Start of Summer"),
    ("小满", "Grain Buds"),
    ("芒种", "Grain in Ear"),
    ("夏至", "Summer Solstice"),
    ("小暑", "Minor Heat"),
    ("大暑", "Major Heat"),
    ("立秋", "Start of Autumn"),
    ("处暑", "End of Heat"),
    ("白露", "White Dew"),
    ("秋分", "Autumn Equinox"),
    ("寒露", "Cold Dew"),
    ("霜降", "Frost's Descent"),
    ("立冬", "Start of Winter"),
    ("小雪", "Minor Snow"),
    ("大雪", "Major Snow"),
    ("冬至", "Winter Solstice"),
];

/// The festivals of the lunar calendar, by month and day (New Year's Eve is the year's last, whichever day that is).
const FESTIVALS: [((u32, u32), (&str, &str)); 7] = [
    ((1, 1), ("春节", "Spring Festival")),
    ((1, 15), ("元宵", "Lantern Festival")),
    ((5, 5), ("端午", "Dragon Boat Festival")),
    ((7, 7), ("七夕", "Qixi")),
    ((8, 15), ("中秋", "Mid-Autumn Festival")),
    ((9, 9), ("重阳", "Double Ninth")),
    ((12, 8), ("腊八", "Laba")),
];
const EVE: (&str, &str) = ("除夕", "New Year's Eve");

const MONTHS: [&str; 12] = ["正月", "二月", "三月", "四月", "五月", "六月", "七月", "八月", "九月", "十月", "十一月", "十二月"];
const DIGITS: [&str; 9] = ["一", "二", "三", "四", "五", "六", "七", "八", "九"];

/// A day of the lunar calendar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lunar {
    /// Its month (the leap month of one, after that one).
    pub month: u32,
    pub leap: bool,
    pub day: u32,
    /// The last of its year.
    last: bool,
    /// The solar term the day is.
    term: Option<usize>,
}

/// `year`'s of `YEARS`.
fn year_of(year: i32) -> Option<(u32, u64)> {
    YEARS.get(usize::try_from(year - FIRST).ok()?).copied()
}

/// The day the lunar year `year` begins on.
fn begins(year: i32) -> Option<i64> {
    Some(Day { year, month: 1, day: 1 }.number() + i64::from(year_of(year)?.0 >> 17))
}

impl Lunar {
    /// What `day` is there (none before or after what the table tells).
    pub fn of(day: Day) -> Option<Lunar> {
        let (_, terms) = year_of(day.year)?;
        let half = (day.month as usize - 1) * 2;
        let term = (half..half + 2).find(|&term| TERMS_FROM[term] + (terms >> (2 * term) & 3) as u32 == day.day);
        let year = if day.number() >= begins(day.year)? { day.year } else { day.year - 1 };
        let mut left = (day.number() - begins(year)?) as u32;
        let (word, _) = year_of(year)?;
        let leap = word >> 13 & 15;
        let months = 12 + u32::from(leap > 0);
        for at in 0..months {
            let long = 29 + (word >> at & 1);
            if left < long {
                let after = leap > 0 && at >= leap;
                return Some(Lunar { month: if after { at } else { at + 1 }, leap: leap > 0 && at == leap, day: left + 1, last: at + 1 == months && left + 1 == long, term });
            }
            left -= long;
        }
        None
    }

    /// The day, said: "九月廿九", "闰六月初三", "Lunar 9/29", "Lunar 6/3 (leap)".
    pub fn told(self, lang: Lang) -> String {
        match lang {
            Lang::Zh => {
                let digit = |n: u32| DIGITS[n as usize - 1];
                let day = match self.day {
                    10 => "初十".to_string(),
                    20 => "二十".to_string(),
                    30 => "三十".to_string(),
                    n @ 1..10 => format!("初{}", digit(n)),
                    n @ 11..20 => format!("十{}", digit(n - 10)),
                    n => format!("廿{}", digit(n - 20)),
                };
                format!("{}{}{day}", if self.leap { "闰" } else { "" }, MONTHS[self.month as usize - 1])
            }
            Lang::En => format!("Lunar {}/{}{}", self.month, self.day, if self.leap { " (leap)" } else { "" }),
        }
    }

    /// The festival the day is, else the solar term.
    pub fn marked(self, lang: Lang) -> Option<&'static str> {
        let festival = || FESTIVALS.iter().find(|(at, _)| !self.leap && *at == (self.month, self.day)).map(|(_, name)| *name);
        let (zh, en) = if self.last { Some(EVE) } else { festival() }.or(self.term.map(|term| TERMS[term]))?;
        Some(lang.pick(zh, en))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn told(year: i32, month: u32, day: u32) -> (String, &'static str) {
        let lunar = Lunar::of(Day { year, month, day }).unwrap();
        (lunar.told(Lang::Zh), lunar.marked(Lang::Zh).unwrap_or(""))
    }

    /// A day's lunar day, its festival or term; a leap month's, said so, with no festival; the year's last, its eve,
    /// though it is a term too; the first day and the last the table tells, and none beyond them.
    #[test]
    fn a_day_is_told_in_the_lunar_calendar() {
        for (day, said, marked) in [
            ((2026, 11, 7), "九月廿九", "立冬"),
            ((2026, 11, 22), "十月十四", "小雪"),
            ((2026, 2, 16), "十二月廿九", "除夕"),
            ((2026, 2, 17), "正月初一", "春节"),
            // (Its new moon is at 23:56 the day before, in China.)
            ((2027, 2, 6), "正月初一", "春节"),
            ((2026, 3, 3), "正月十五", "元宵"),
            ((2026, 6, 19), "五月初五", "端午"),
            ((2026, 8, 19), "七月初七", "七夕"),
            ((2026, 9, 25), "八月十五", "中秋"),
            ((2026, 10, 18), "九月初九", "重阳"),
            ((2027, 1, 15), "十二月初八", "腊八"),
            ((2025, 7, 25), "闰六月初一", ""),
            ((2025, 8, 22), "闰六月廿九", ""),
            ((2025, 8, 23), "七月初一", "处暑"),
            // (New Year's Eve and the start of spring.)
            ((2019, 2, 4), "十二月三十", "除夕"),
            ((2033, 12, 22), "闰十一月初一", ""),
            ((2034, 2, 19), "正月初一", "春节"),
            ((2026, 4, 5), "二月十八", "清明"),
            ((1901, 2, 19), "正月初一", "春节"),
            ((2100, 12, 31), "十二月初一", ""),
        ] {
            assert_eq!(told(day.0, day.1, day.2), (said.to_string(), marked), "{day:?}");
        }
        assert_eq!(Lunar::of(Day { year: 1901, month: 2, day: 18 }), None);
        assert_eq!(Lunar::of(Day { year: 2101, month: 1, day: 1 }), None);
        let leap = Lunar::of(Day { year: 2025, month: 7, day: 27 }).unwrap();
        assert_eq!((leap.told(Lang::En).as_str(), Lunar::of(Day { year: 2026, month: 11, day: 7 }).unwrap().marked(Lang::En)), ("Lunar 6/3 (leap)", Some("Start of Winter")));
    }

    /// Every day the table tells is as the observatory's tables have it (`GLANCE_HKO`: the folder of them, T1901.txt to
    /// T2100.txt, as it gives them out).
    #[test]
    #[ignore]
    fn every_day_is_as_the_observatory_tells_it() {
        let dir = std::path::PathBuf::from(std::env::var("GLANCE_HKO").unwrap());
        let mut days = 0;
        for year in FIRST..FIRST + YEARS.len() as i32 {
            let mut text = std::fs::read_to_string(dir.join(format!("T{year}.txt"))).unwrap();
            for (as_given, as_told) in [("閏", "闰"), ("驚蟄", "惊蛰"), ("穀", "谷"), ("滿", "满"), ("種", "种"), ("處", "处")] {
                text = text.replace(as_given, as_told);
            }
            for line in text.lines() {
                let words: Vec<&str> = line.split_whitespace().collect();
                // (A day's line, not a note under them.)
                let [on, said, week, ..] = words.as_slice() else { continue };
                if !week.starts_with("星期") {
                    continue;
                }
                let parts: Vec<u32> = on.split(['年', '月', '日']).filter_map(|part| part.parse().ok()).collect();
                let &[of, month, day] = parts.as_slice() else { continue };
                let Some(lunar) = Lunar::of(Day { year: of as i32, month, day }) else {
                    assert!(year == FIRST && (month, day) < (2, 19), "{line}");
                    continue;
                };
                let (zh, first) = (lunar.told(Lang::Zh), lunar.day == 1);
                assert!(if first { zh == format!("{said}初一") } else { zh.ends_with(said) && !said.contains('月') }, "{line}: {zh}");
                assert_eq!(lunar.term.map(|term| TERMS[term].0), words.get(3).copied(), "{line}");
                days += 1;
            }
        }
        assert_eq!(days, 73_049 - 49);
    }
}
