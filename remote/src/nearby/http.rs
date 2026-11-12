//! The little of HTTP/1.1 that nearby transfer speaks, over whatever carries it (TLS or plain): a request's head, its
//! body as it comes (by its length, or in chunks), and an answer; and, sending, an answer read back.

use std::io::{self, Read, Write};

/// The most a head may be (a request's or an answer's).
const HEAD: usize = 16 << 10;

pub struct Head {
    /// The request's method and path (without its query), or, for an answer, its status (in `status`).
    pub method: String,
    pub path: String,
    pub query: Vec<(String, String)>,
    pub status: u16,
    headers: Vec<(String, String)>,
    /// What came after the head with it: the body's start.
    rest: Vec<u8>,
}

impl Head {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(key, _)| key.eq_ignore_ascii_case(name)).map(|(_, value)| value.as_str())
    }

    pub fn query(&self, name: &str) -> Option<&str> {
        self.query.iter().find(|(key, _)| key == name).map(|(_, value)| value.as_str())
    }

    /// How its body comes: in chunks, or as long as it says (none said: none, for a request).
    fn framing(&self) -> io::Result<Framing> {
        if self.header("transfer-encoding").is_some_and(|coding| coding.to_ascii_lowercase().contains("chunked")) {
            return Ok(Framing::Chunked);
        }
        match self.header("content-length") {
            Some(length) => length.trim().parse().map(Framing::Length).map_err(|_| bad("a length that is no number")),
            None => Ok(Framing::Unsaid),
        }
    }
}

fn bad(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.to_string())
}

/// The head of what comes on `stream` (a request's, or an answer's where `answer`): none, where it closed first.
pub fn head(stream: &mut impl Read, answer: bool) -> io::Result<Option<Head>> {
    let mut taken = Vec::new();
    let mut chunk = [0u8; 4096];
    let end = loop {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            return if taken.is_empty() { Ok(None) } else { Err(bad("closed within the head")) };
        }
        // (Looked for from a little before what came now: the end may straddle two reads.)
        let from = taken.len().saturating_sub(3);
        taken.extend_from_slice(&chunk[..read]);
        if let Some(at) = taken[from..].windows(4).position(|four| four == b"\r\n\r\n") {
            break from + at;
        }
        if taken.len() > HEAD {
            return Err(bad("a head too long"));
        }
    };
    let text = std::str::from_utf8(&taken[..end]).map_err(|_| bad("a head that is not text"))?;
    let mut lines = text.split("\r\n");
    let first = lines.next().unwrap_or_default();
    let mut words = first.splitn(3, ' ');
    let (one, two) = (words.next().unwrap_or_default(), words.next().unwrap_or_default());
    let headers = lines.filter_map(|line| line.split_once(':')).map(|(key, value)| (key.trim().to_string(), value.trim().to_string())).collect();
    let rest = taken[end + 4..].to_vec();
    if answer {
        let status = two.parse().map_err(|_| bad("an answer with no status"))?;
        return Ok(Some(Head { method: String::new(), path: String::new(), query: Vec::new(), status, headers, rest }));
    }
    let (path, query) = two.split_once('?').unwrap_or((two, ""));
    let query = query.split('&').filter(|pair| !pair.is_empty()).map(|pair| {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        (unescaped(key), unescaped(value))
    });
    Ok(Some(Head { method: one.to_string(), path: unescaped(path), query: query.collect(), status: 0, headers, rest }))
}

/// `%XX` and `+` in a URL's part, as what they stand for.
fn unescaped(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let digit = |byte: u8| (byte as char).to_digit(16).map(|digit| digit as u8);
        match bytes[at] {
            b'%' if at + 2 < bytes.len() => match (digit(bytes[at + 1]), digit(bytes[at + 2])) {
                (Some(high), Some(low)) => {
                    out.push(high << 4 | low);
                    at += 3;
                    continue;
                }
                _ => out.push(b'%'),
            },
            b'+' => out.push(b' '),
            byte => out.push(byte),
        }
        at += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `%`-escapes what may not stand in a URL's query as it is.
pub fn escaped(text: &str) -> String {
    text.bytes().map(|byte| if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) { (byte as char).to_string() } else { format!("%{byte:02X}") }).collect()
}

enum Framing {
    Length(u64),
    Chunked,
    Unsaid,
}

/// A body as it comes, after its head: as long as it said, or in chunks till the last; with no length said, none (a
/// request's) or all there is till the other side closes (an answer's).
pub struct Body<'a, S: Read> {
    stream: &'a mut S,
    rest: Vec<u8>,
    framing: Framing,
    /// Left of what was said (its whole length, or the chunk's), and whether the last chunk has come.
    left: u64,
    done: bool,
    answer: bool,
}

impl<'a, S: Read> Body<'a, S> {
    pub fn of(head: &mut Head, stream: &'a mut S, answer: bool) -> io::Result<Body<'a, S>> {
        let framing = head.framing()?;
        let left = match framing {
            Framing::Length(length) => length,
            _ => 0,
        };
        let done = matches!(framing, Framing::Length(0)) || (matches!(framing, Framing::Unsaid) && !answer);
        Ok(Body { stream, rest: std::mem::take(&mut head.rest), framing, left, done, answer })
    }

    /// Reads into `into` from what came already, then from the stream.
    fn raw(&mut self, into: &mut [u8]) -> io::Result<usize> {
        if !self.rest.is_empty() {
            let count = into.len().min(self.rest.len());
            into[..count].copy_from_slice(&self.rest[..count]);
            self.rest.drain(..count);
            return Ok(count);
        }
        self.stream.read(into)
    }

    /// One line of a chunk's framing (its size, or the empty line after it).
    fn line(&mut self) -> io::Result<String> {
        let mut line = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            if self.raw(&mut byte)? == 0 {
                return Err(bad("closed within a chunk's size"));
            }
            if byte[0] == b'\n' {
                break;
            }
            line.push(byte[0]);
            if line.len() > 1024 {
                return Err(bad("a chunk's size too long"));
            }
        }
        Ok(String::from_utf8_lossy(&line).trim().to_string())
    }

    /// All of it, if it is no more than `most` bytes.
    pub fn whole(mut self, most: usize) -> io::Result<Vec<u8>> {
        let mut all = Vec::new();
        let mut chunk = [0u8; 8192];
        loop {
            let read = self.read(&mut chunk)?;
            if read == 0 {
                return Ok(all);
            }
            all.extend_from_slice(&chunk[..read]);
            if all.len() > most {
                return Err(bad("a body too long"));
            }
        }
    }
}

impl<S: Read> Read for Body<'_, S> {
    fn read(&mut self, into: &mut [u8]) -> io::Result<usize> {
        if self.done || into.is_empty() {
            return Ok(0);
        }
        match self.framing {
            Framing::Unsaid => {
                debug_assert!(self.answer);
                self.raw(into)
            }
            Framing::Length(_) => {
                let most = into.len().min(self.left.min(usize::MAX as u64) as usize);
                let read = self.raw(&mut into[..most])?;
                if read == 0 {
                    return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "closed before the whole body came"));
                }
                self.left -= read as u64;
                self.done = self.left == 0;
                Ok(read)
            }
            Framing::Chunked => {
                if self.left == 0 {
                    let size = self.line()?;
                    let size = size.split(';').next().unwrap_or_default();
                    self.left = u64::from_str_radix(size, 16).map_err(|_| bad("a chunk's size that is no number"))?;
                    if self.left == 0 {
                        // The trailer, to its empty line.
                        while !self.line()?.is_empty() {}
                        self.done = true;
                        return Ok(0);
                    }
                }
                let most = into.len().min(self.left.min(usize::MAX as u64) as usize);
                let read = self.raw(&mut into[..most])?;
                if read == 0 {
                    return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "closed within a chunk"));
                }
                self.left -= read as u64;
                if self.left == 0 {
                    // The line's end after the chunk.
                    self.line()?;
                }
                Ok(read)
            }
        }
    }
}

/// An answer: `status`, with `json` as its body where there is one.
pub fn answer(stream: &mut impl Write, status: u16, json: Option<&str>) -> io::Result<()> {
    let reason = match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        409 => "Conflict",
        422 => "Unprocessable Entity",
        429 => "Too Many Requests",
        _ => "Error",
    };
    let body = json.unwrap_or_default();
    let kind = if json.is_some() { "Content-Type: application/json\r\n" } else { "" };
    write!(stream, "HTTP/1.1 {status} {reason}\r\n{kind}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())?;
    stream.flush()
}

/// A request's head: `method` at `path` (with its query) of `host`, with a body of `length` bytes as JSON where
/// `json`, else as bytes.
pub fn ask(stream: &mut impl Write, method: &str, path: &str, host: &str, length: u64, json: bool) -> io::Result<()> {
    let kind = if json { "application/json" } else { "application/octet-stream" };
    write!(stream, "{method} {path} HTTP/1.1\r\nHost: {host}\r\nContent-Type: {kind}\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body_of(bytes: &[u8], most: usize) -> io::Result<(Head, Vec<u8>)> {
        let mut stream = std::io::Cursor::new(bytes.to_vec());
        let mut head = head(&mut stream, false)?.unwrap();
        let body = Body::of(&mut head, &mut stream, false)?.whole(most)?;
        Ok((head, body))
    }

    /// A request's head, its query unescaped, and its body by its length, however it was split between reads.
    #[test]
    fn a_request_is_read_by_its_length() {
        let (head, body) = body_of(b"POST /api/localsend/v2/upload?sessionId=a%20b&fileId=x+y&token=t HTTP/1.1\r\nContent-Length: 5\r\nHost: h\r\n\r\nhelloEXTRA", 100).unwrap();
        assert_eq!((head.method.as_str(), head.path.as_str()), ("POST", "/api/localsend/v2/upload"));
        assert_eq!(head.query("sessionId"), Some("a b"));
        assert_eq!(head.query("fileId"), Some("x y"));
        assert_eq!(head.header("host"), Some("h"));
        assert_eq!(body, b"hello");
    }

    /// A body in chunks, with extensions and a trailer, read whole; one longer than allowed, refused.
    #[test]
    fn a_body_in_chunks_is_read_whole() {
        let (_, body) = body_of(b"POST /x HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n5;ext=1\r\nhello\r\n6\r\n world\r\n0\r\nTrailer: x\r\n\r\n", 100).unwrap();
        assert_eq!(body, b"hello world");
        assert!(body_of(b"POST /x HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\n\r\n", 3).is_err());
    }

    /// A request with no length has no body; one that ends before its length is an error, not a short body.
    #[test]
    fn a_body_ends_as_it_said() {
        let (_, body) = body_of(b"POST /x HTTP/1.1\r\n\r\nignored", 100).unwrap();
        assert!(body.is_empty());
        assert!(body_of(b"POST /x HTTP/1.1\r\nContent-Length: 9\r\n\r\nshort", 100).is_err());
    }

    /// An answer's status and body, read to the stream's end where no length is said.
    #[test]
    fn an_answer_is_read() {
        let mut stream = std::io::Cursor::new(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n".to_vec());
        let mut answer = head(&mut stream, true).unwrap().unwrap();
        assert_eq!(answer.status, 204);
        assert!(Body::of(&mut answer, &mut stream, true).unwrap().whole(10).unwrap().is_empty());
        let mut stream = std::io::Cursor::new(b"HTTP/1.1 200 OK\r\n\r\n{\"a\":1}".to_vec());
        let mut answer = head(&mut stream, true).unwrap().unwrap();
        assert_eq!(Body::of(&mut answer, &mut stream, true).unwrap().whole(100).unwrap(), b"{\"a\":1}");
    }
}
