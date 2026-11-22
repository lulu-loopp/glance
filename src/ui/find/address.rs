//! A web address typed into search, told from other words, and what a web search of words is opened at.

/// The domains there are at the top (IANA's, each as it is written and as it is encoded), a line each.
const DOMAINS: &str = include_str!("domains.txt");

/// A web address typed.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Address {
    /// What it is opened at.
    pub url: String,
    /// What it can be nothing else than: with how it is reached, a port or a place at it told, or a number's.
    pub sure: bool,
}

/// The address `typed` is, where it is one: as one is written whole; or a host there can be (its domain one there is, a
/// number's, this PC's), reached over https (a number's and this PC's, http: a router's page has no certificate).
pub fn typed(typed: &str) -> Option<Address> {
    let typed = typed.trim();
    if typed.contains(|c: char| c.is_whitespace() || c.is_control() || c == '\\') {
        return None;
    }
    let scheme = ["https://", "http://"].into_iter().find(|scheme| typed.get(..scheme.len()).is_some_and(|start| start.eq_ignore_ascii_case(scheme)));
    let rest = &typed[scheme.map_or(0, str::len)..];
    // The host, its port, and what is asked of it.
    let (at, asked) = rest.split_at(rest.find(['/', '?', '#']).unwrap_or(rest.len()));
    // (A dot as an input method types one.)
    let at = at.replace(['。', '．', '｡'], ".");
    let (host, port) = match at.rsplit_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (at.as_str(), None),
    };
    if port.is_some_and(|port| port.is_empty() || port.parse::<u16>().is_err() || !port.bytes().all(|c| c.is_ascii_digit())) {
        return None;
    }
    let local = host.eq_ignore_ascii_case("localhost");
    let kind = host_kind(host);
    let told = port.is_some() || !asked.is_empty();
    let (scheme, sure) = match (scheme, kind) {
        // (Reached as told, whatever host: one of the user's own network has a name of its own.)
        (Some(scheme), _) if labelled(host) => (scheme, true),
        (None, Some(HostKind::Number)) => ("http://", true),
        (None, _) if local && told => ("http://", true),
        (None, Some(HostKind::Named)) => ("https://", told || host.get(..4).is_some_and(|start| start.eq_ignore_ascii_case("www."))),
        _ => return None,
    };
    Some(Address { url: format!("{scheme}{at}{}", encoded(asked)), sure })
}

#[derive(Clone, Copy, PartialEq)]
enum HostKind {
    Number,
    Named,
}

/// Whether `host` is made of labels as a host is.
fn labelled(host: &str) -> bool {
    !host.is_empty() && host.split('.').all(|label| label.chars().all(|c| c.is_alphanumeric() || matches!(c, '-' | '_')) && !label.is_empty() && !label.starts_with('-') && !label.ends_with('-'))
}

/// What kind of host `host` is: a number's, or one named, in a domain there is.
fn host_kind(host: &str) -> Option<HostKind> {
    let labels: Vec<&str> = host.split('.').collect();
    if labels.len() < 2 || !labelled(host) {
        return None;
    }
    if labels.iter().all(|label| label.bytes().all(|c| c.is_ascii_digit())) {
        let number = labels.len() == 4 && labels.iter().all(|label| label.parse::<u8>().is_ok() && (label.len() == 1 || !label.starts_with('0')));
        return number.then_some(HostKind::Number);
    }
    let domain = labels.last()?.to_lowercase();
    DOMAINS.lines().any(|line| line == domain).then_some(HostKind::Named)
}

/// `asked`, of an address, with what an address holds none of encoded.
fn encoded(asked: &str) -> String {
    asked.bytes().fold(String::new(), |mut whole, byte| {
        match byte {
            b'!' | b'#'..=b';' | b'=' | b'?'..=b'[' | b']' | b'_' | b'a'..=b'z' | b'~' => whole.push(char::from(byte)),
            _ => whole.push_str(&format!("%{byte:02X}")),
        }
        whole
    })
}

/// `words` as they are asked for in an address.
pub fn quoted(words: &str) -> String {
    words.bytes().fold(String::new(), |mut whole, byte| {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => whole.push(char::from(byte)),
            _ => whole.push_str(&format!("%{byte:02X}")),
        }
        whole
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_are_told_from_other_words() {
        let sure = |url: &str| Some(Address { url: url.into(), sure: true });
        let maybe = |url: &str| Some(Address { url: url.into(), sure: false });
        for (words, address) in [
            ("github.com", maybe("https://github.com")),
            ("GitHub.COM", maybe("https://GitHub.COM")),
            ("github。com", maybe("https://github.com")),
            ("main.rs", maybe("https://main.rs")),
            ("中国互联网络信息中心.中国", maybe("https://中国互联网络信息中心.中国")),
            ("www.gov.cn", sure("https://www.gov.cn")),
            ("bilibili.com/video", sure("https://bilibili.com/video")),
            ("example.com:8443", sure("https://example.com:8443")),
            ("https://a.b/c", sure("https://a.b/c")),
            ("HTTP://nas", sure("http://nas")),
            ("http://my_pc:5000/x?y=1#z", sure("http://my_pc:5000/x?y=1#z")),
            ("192.168.1.1", sure("http://192.168.1.1")),
            ("10.0.0.2:8080/x", sure("http://10.0.0.2:8080/x")),
            ("localhost:3000", sure("http://localhost:3000")),
            ("example.com/搜索?q=\"a\"", sure("https://example.com/%E6%90%9C%E7%B4%A2?q=%22a%22")),
        ] {
            assert_eq!(typed(words), address, "{words}");
        }
        for words in ["", "readme", "readme.txt", "two words.com", "1.2", "1.2.3", "v1.2.3", "256.1.1.1", "192.168.01.1", "localhost", "a@b.com", ".com", "com.", "a..com", "-a.com", r"D:\a.com", "file:///c:/a", "ms-settings:display", "http://", "example.com:99999", "example.com:", "https://a b"] {
            assert_eq!(typed(words), None, "{words}");
        }
    }

    #[test]
    fn words_are_quoted_for_a_web_search() {
        assert_eq!(quoted("主板 温度"), "%E4%B8%BB%E6%9D%BF%20%E6%B8%A9%E5%BA%A6");
        assert_eq!(quoted("a+b&c"), "a%2Bb%26c");
    }
}
