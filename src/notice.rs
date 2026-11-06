//! What Glance's site has to tell the users of Glance (a notice: when an update cannot be taken by Glance itself, as
//! after a change of certificate, that it is had from the site), fetched with the asking for a newer release (see
//! `update::watch`), nothing more sent for it. Told once in a notification, and shown at the top of the settings'
//! updates till it is read. Its words are plain text, in Chinese and in English, and its link, if it has one, goes to
//! Glance's site and nowhere else: a notice neither downloads nor runs anything.
//!
//! The site holds one notice at a time (website/notice.json), with the versions it is for (those before `below`):
//! one taken down there is gone from the settings as the site next answers.

use std::sync::Mutex;
use std::time::Instant;

use serde_json::Value;

use crate::ui::text::Lang;

#[derive(Clone, Debug, PartialEq)]
pub struct Notice {
    /// What tells it from any other, for telling it once and for its being read.
    pub id: String,
    /// Its title and its text, in Chinese and in English.
    zh: (String, String),
    en: (String, String),
    /// A page of Glance's site that says more.
    pub link: Option<String>,
}

impl Notice {
    /// Its title and its text in `lang`.
    pub fn words(&self, lang: Lang) -> (&str, &str) {
        let (title, text) = match lang {
            Lang::Zh => &self.zh,
            Lang::En => &self.en,
        };
        (title, text)
    }
}

/// The notice Glance's site gave as it last answered.
static HEARD: Mutex<Option<Notice>> = Mutex::new(None);
/// When a notice was last told in a notification.
static TOLD_AT: Mutex<Option<Instant>> = Mutex::new(None);

/// What Glance's site answered (`body`) as it was asked for the latest release: the notice it holds for this version,
/// or none, which takes down the one heard before. What is not an answer of the site's (not an object) changes
/// nothing. A notice not told before is told as it comes (the site may answer after the asking has gone on without
/// it: see `update::latest`).
pub fn heard(body: &[u8]) {
    let Ok(answer) = serde_json::from_slice::<Value>(body) else { return };
    if !answer.is_object() {
        return;
    }
    let notice = notice_in(&answer, env!("CARGO_PKG_VERSION"), &crate::update::own_site());
    let mut heard = HEARD.lock().unwrap();
    if *heard != notice {
        match &notice {
            Some(notice) => crate::journal::note(format!("notice from the site: {}", notice.id)),
            None if heard.is_some() => crate::journal::note("notice from the site: none now".to_string()),
            None => {}
        }
        *heard = notice;
    }
    drop(heard);
    tell(crate::update::language());
}

/// The notice in what the site answered, for a Glance of version `own`, whose site is `site`: none where it holds none,
/// where it is for other versions, or where it is not whole (an id, and a title and a text in each language) or its
/// link goes elsewhere than `site`.
fn notice_in(answer: &Value, own: &str, site: &str) -> Option<Notice> {
    let notice = answer.get("notice").filter(|notice| !notice.is_null())?;
    let text = |value: &Value| value.as_str().map(str::trim).filter(|text| !text.is_empty()).map(str::to_string);
    let words = |lang: &str| Some((text(&notice[lang]["title"])?, text(&notice[lang]["text"])?));
    let id = text(&notice["id"])?;
    let (zh, en) = (words("zh")?, words("en")?);
    // For the versions before `below` alone, where it says.
    if let Some(below) = notice.get("below").filter(|below| !below.is_null()) {
        if !crate::update::newer(below.as_str()?, own) {
            return None;
        }
    }
    let link = match notice.get("link").filter(|link| !link.is_null()) {
        None => None,
        // A page of the site, said whole: no space nor quote in it to be read as more than an address.
        Some(link) => {
            let link = link.as_str()?;
            let whole = link.chars().all(|c| c.is_ascii_graphic() && c != '"');
            if !whole || !link.starts_with(&format!("{site}/")) {
                return None;
            }
            Some(link.to_string())
        }
    };
    Some(Notice { id, zh, en, link })
}

/// The notice heard and not yet read: what the settings show.
pub fn unread() -> Option<Notice> {
    let notice = HEARD.lock().unwrap().clone()?;
    let read = crate::app().settings.lock().unwrap().notices_read.contains(&notice.id);
    (!read).then_some(notice)
}

/// Tells the notice heard in a notification, in `lang`, unless it was told before or read: whether it did. Clicked, the
/// notification opens the settings at it. (Once, though asked from two threads at once: noted as told under the
/// settings' lock.)
pub fn tell(lang: Lang) -> bool {
    let Some(notice) = unread() else { return false };
    let app = crate::app();
    if app.settings.lock().unwrap().notices_told.contains(&notice.id) {
        return false;
    }
    let mut first = false;
    app.change(|settings| {
        first = !settings.notices_told.contains(&notice.id);
        if first {
            settings.notices_told.push(notice.id.clone());
        }
    });
    if !first {
        return false;
    }
    let (title, text) = notice.words(lang);
    *TOLD_AT.lock().unwrap() = Some(Instant::now());
    crate::tray::notify_updates(title, text);
    true
}

/// Whether a notice was told in a notification since `then`.
pub fn told_since(then: Instant) -> bool {
    TOLD_AT.lock().unwrap().is_some_and(|at| at >= then)
}

/// The notice `id` read: not shown again.
pub fn read(id: &str) {
    crate::app().change(|settings| {
        if !settings.notices_read.iter().any(|read| read == id) {
            settings.notices_read.push(id.to_string());
        }
    });
}

/// A notice heard, or none, for pictures of the settings.
#[cfg(all(test, feature = "studio"))]
pub fn pretend(notice: Option<(&str, (&str, &str), (&str, &str), Option<&str>)>) {
    *HEARD.lock().unwrap() = notice.map(|(id, zh, en, link)| Notice {
        id: id.to_string(),
        zh: (zh.0.to_string(), zh.1.to_string()),
        en: (en.0.to_string(), en.1.to_string()),
        link: link.map(str::to_string),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SITE: &str = "https://glancepc.com";

    fn answer(notice: Value) -> Value {
        json!({ "tag_name": "v0.4.3", "assets": [], "notice": notice })
    }

    fn whole() -> Value {
        json!({
            "id": "2027-01-signing",
            "zh": { "title": "更新方式变更", "text": "请从官网下载安装一次。" },
            "en": { "title": "A change to updating", "text": "Download it once from the website." },
            "link": "https://glancepc.com/#download",
            "below": "0.5.0",
        })
    }

    /// A notice whole, for this version, is read as the site gave it, its words in each language.
    #[test]
    fn a_notice_is_read_as_the_site_gives_it() {
        let notice = notice_in(&answer(whole()), "0.4.3-preview", SITE).unwrap();
        assert_eq!(notice.id, "2027-01-signing");
        assert_eq!(notice.words(Lang::Zh), ("更新方式变更", "请从官网下载安装一次。"));
        assert_eq!(notice.words(Lang::En), ("A change to updating", "Download it once from the website."));
        assert_eq!(notice.link.as_deref(), Some("https://glancepc.com/#download"));
        // Without a link, and for every version.
        let mut plain = whole();
        plain.as_object_mut().unwrap().remove("link");
        plain.as_object_mut().unwrap().remove("below");
        let notice = notice_in(&answer(plain), "9.0.0", SITE).unwrap();
        assert_eq!(notice.link, None);
        let mut nulls = whole();
        nulls["link"] = Value::Null;
        nulls["below"] = Value::Null;
        assert_eq!(notice_in(&answer(nulls), "9.0.0", SITE).unwrap().link, None);
    }

    /// Where the site holds none, there is none: an answer without one, or with none said.
    #[test]
    fn no_notice_where_the_site_holds_none() {
        assert_eq!(notice_in(&json!({ "tag_name": "v0.4.3", "assets": [] }), "0.4.3", SITE), None);
        assert_eq!(notice_in(&answer(Value::Null), "0.4.3", SITE), None);
    }

    /// A notice for the versions before `below` is for those alone, versions ordered as releases are (a preview before
    /// its release).
    #[test]
    fn a_notice_is_for_the_versions_it_names() {
        for (own, shown) in [("0.4.2", true), ("0.4.3-preview", true), ("0.4.9", true), ("0.5.0-preview", true), ("0.5.0", false), ("0.5.1", false), ("1.0.0", false)] {
            assert_eq!(notice_in(&answer(whole()), own, SITE).is_some(), shown, "{own}");
        }
        let mut odd = whole();
        odd["below"] = json!(5);
        assert_eq!(notice_in(&answer(odd), "0.4.3", SITE), None);
    }

    /// A notice not whole is none: no id, or a title or text missing or empty in either language.
    #[test]
    fn a_notice_not_whole_is_none() {
        for (path, value) in [("id", json!("")), ("id", json!(3)), ("zh", json!({ "title": "更新" })), ("en", json!({ "title": " ", "text": "x" })), ("en", Value::Null)] {
            let mut notice = whole();
            notice[path] = value;
            assert_eq!(notice_in(&answer(notice), "0.4.3", SITE), None, "{path}");
        }
        let mut notice = whole();
        notice.as_object_mut().unwrap().remove("zh");
        assert_eq!(notice_in(&answer(notice), "0.4.3", SITE), None);
    }

    /// Its link goes to a page of Glance's site or the notice is none: not another site, not one named like it, not
    /// an address with more said after a space or a quote.
    #[test]
    fn a_notice_links_to_the_site_alone() {
        for link in [
            json!("https://example.com/"),
            json!("http://glancepc.com/"),
            json!("https://glancepc.com"),
            json!("https://glancepc.com.example.com/"),
            json!("https://glancepc.com@example.com/"),
            json!("https://glancepc.com/ --flag"),
            json!("https://glancepc.com/\"x"),
            json!("https://glancepc.com/页"),
            json!(7),
        ] {
            let mut notice = whole();
            notice["link"] = link.clone();
            assert_eq!(notice_in(&answer(notice), "0.4.3", SITE), None, "{link}");
        }
        let mut notice = whole();
        notice["link"] = json!("https://glancepc.com/en/#download");
        assert!(notice_in(&answer(notice), "0.4.3", SITE).is_some());
    }
}
