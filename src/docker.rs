//! Docker, as its engine reports it: the containers running and what each
//! uses, asked of the engine the Docker CLI's current context points at,
//! through its read-only API, on a thread of its own (an engine slow to
//! answer holds up nothing else), while the module is on.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::reading::{ContainerSample, DockerSample};

/// How long an answer may take before the engine is taken as not answering.
const PATIENCE: Duration = Duration::from_secs(5);
/// The most an answer may hold (a container's statistics are a few
/// kilobytes; the list, some per container).
const MOST: usize = 8 << 20;

pub struct Docker {
    latest: Arc<Mutex<Option<DockerSample>>>,
    stop: Arc<AtomicBool>,
    /// When the request under way began, if one is.
    asking: Arc<Mutex<Option<Instant>>>,
    worker: std::thread::JoinHandle<()>,
}

impl Docker {
    pub fn new() -> Self {
        let latest = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        let asking = Arc::new(Mutex::new(None));
        let (shared, stopping, busy) = (latest.clone(), stop.clone(), asking.clone());
        let worker = std::thread::spawn(move || {
            let mut before: HashMap<String, Counters> = HashMap::new();
            while !stopping.load(Ordering::Relaxed) {
                let started = Instant::now();
                let sample = read(&mut before, &busy, &stopping);
                if stopping.load(Ordering::Relaxed) {
                    break;
                }
                *shared.lock().unwrap() = Some(sample);
                // As often as Glance samples.
                let every = crate::app().settings.lock().unwrap().interval();
                std::thread::sleep(every.saturating_sub(started.elapsed()));
            }
        });
        Docker { latest, stop, asking, worker }
    }

    /// What was read last; none until the first answer. A request under way
    /// longer than it may take is cut short (a pipe has no time limit of
    /// its own).
    pub fn read(&self) -> Option<DockerSample> {
        if self.asking.lock().unwrap().is_some_and(|since| since.elapsed() > PATIENCE) {
            self.cancel();
        }
        self.latest.lock().unwrap().clone()
    }

    /// Ends the worker's blocking read or write, if it is in one.
    fn cancel(&self) {
        use std::os::windows::io::AsRawHandle;
        let thread = windows::Win32::Foundation::HANDLE(self.worker.as_raw_handle());
        let _ = unsafe { windows::Win32::System::IO::CancelSynchronousIo(thread) };
    }
}

impl Drop for Docker {
    fn drop(&mut self) {
        // The worker stops: told to, and its requests cut short until it
        // has (one it was about to make as it was told is cut short too),
        // by a thread of its own that outlives this.
        self.stop.store(true, Ordering::Relaxed);
        let worker = std::mem::replace(&mut self.worker, std::thread::spawn(|| {}));
        std::thread::spawn(move || {
            use std::os::windows::io::AsRawHandle;
            let thread = windows::Win32::Foundation::HANDLE(worker.as_raw_handle());
            while !worker.is_finished() {
                let _ = unsafe { windows::Win32::System::IO::CancelSynchronousIo(thread) };
                std::thread::sleep(Duration::from_millis(50));
            }
        });
    }
}

/// Where the engine is, as the Docker CLI finds it: the context named by
/// DOCKER_CONTEXT, else DOCKER_HOST, else the current context.
struct Endpoint {
    context: String,
    host: String,
}

fn endpoint() -> Endpoint {
    let home = std::env::var_os("USERPROFILE").map(std::path::PathBuf::from).unwrap_or_default();
    let docker = std::env::var_os("DOCKER_CONFIG").map(std::path::PathBuf::from).unwrap_or_else(|| home.join(".docker"));
    let named = std::env::var("DOCKER_CONTEXT").ok().filter(|name| !name.is_empty());
    let host = std::env::var("DOCKER_HOST").ok().filter(|host| !host.is_empty());
    if let (None, Some(host)) = (&named, &host) {
        return Endpoint { context: "default".into(), host: host.clone() };
    }
    #[derive(Deserialize)]
    struct Config {
        #[serde(rename = "currentContext", default)]
        current: String,
    }
    let context = named.unwrap_or_else(|| {
        std::fs::read(docker.join("config.json")).ok().and_then(|bytes| serde_json::from_slice::<Config>(&bytes).ok()).map(|config| config.current).unwrap_or_default()
    });
    let default = || Endpoint { context: "default".into(), host: "npipe:////./pipe/docker_engine".into() };
    if context.is_empty() || context == "default" {
        return default();
    }
    // A context's endpoint, in its metadata (found by its name).
    #[derive(Deserialize)]
    #[serde(rename_all = "PascalCase")]
    struct Meta {
        name: String,
        endpoints: HashMap<String, MetaEndpoint>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "PascalCase")]
    struct MetaEndpoint {
        #[serde(default)]
        host: String,
    }
    let found = std::fs::read_dir(docker.join("contexts").join("meta")).into_iter().flatten().flatten().find_map(|dir| {
        let meta: Meta = serde_json::from_slice(&std::fs::read(dir.path().join("meta.json")).ok()?).ok()?;
        (meta.name == context).then(|| meta.endpoints.get("docker").map(|e| e.host.clone()).unwrap_or_default())
    });
    match found {
        Some(host) => Endpoint { context, host },
        None => default(),
    }
}

/// A connection to the engine: a named pipe on this machine, or plain TCP.
enum Connection {
    Pipe(std::fs::File),
    Tcp(std::net::TcpStream),
}

impl Connection {
    /// Opened to `host`; none for a kind of address this does not reach
    /// (ssh, TLS), and an error for one it could not.
    fn open(host: &str) -> Option<std::io::Result<Connection>> {
        if let Some(pipe) = host.strip_prefix("npipe://") {
            // npipe:////./pipe/name, the slashes the CLI's own.
            let path = pipe.trim_start_matches('/').replace('/', "\\");
            let path = format!("\\\\{path}");
            return Some(std::fs::OpenOptions::new().read(true).write(true).open(path).map(Connection::Pipe));
        }
        if let Some(address) = host.strip_prefix("tcp://") {
            let opened = std::net::TcpStream::connect(address.trim_end_matches('/')).and_then(|stream| {
                stream.set_read_timeout(Some(PATIENCE))?;
                stream.set_write_timeout(Some(PATIENCE))?;
                Ok(stream)
            });
            return Some(opened.map(Connection::Tcp));
        }
        None
    }

    /// GET `path`: the answer's status and body.
    fn get(mut self, path: &str) -> std::io::Result<(u16, Vec<u8>)> {
        let request = format!("GET {path} HTTP/1.1\r\nHost: docker\r\nUser-Agent: Glance\r\nConnection: close\r\n\r\n");
        let mut answer = Vec::new();
        match &mut self {
            Connection::Pipe(pipe) => {
                pipe.write_all(request.as_bytes())?;
                read_all(pipe, &mut answer)?;
            }
            Connection::Tcp(stream) => {
                stream.write_all(request.as_bytes())?;
                read_all(stream, &mut answer)?;
            }
        }
        parse(&answer)
    }
}

/// Everything until the other end closes (a pipe closed says so as an
/// error), up to `MOST`: an answer longer is not one.
fn read_all(from: &mut impl Read, into: &mut Vec<u8>) -> std::io::Result<()> {
    let mut chunk = [0u8; 16 << 10];
    loop {
        match from.read(&mut chunk) {
            Ok(0) => return Ok(()),
            Ok(n) => {
                if into.len() + n > MOST {
                    return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "an answer past its bound"));
                }
                into.extend_from_slice(&chunk[..n]);
            }
            Err(error) if error.raw_os_error() == Some(109) && !into.is_empty() => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
}

/// An HTTP/1.1 answer's status and body, its chunks joined.
fn parse(answer: &[u8]) -> std::io::Result<(u16, Vec<u8>)> {
    let bad = || std::io::Error::new(std::io::ErrorKind::InvalidData, "not an HTTP answer");
    let split = answer.windows(4).position(|w| w == b"\r\n\r\n").ok_or_else(bad)?;
    let head = std::str::from_utf8(&answer[..split]).map_err(|_| bad())?;
    let body = &answer[split + 4..];
    let status: u16 = head.split(' ').nth(1).and_then(|code| code.parse().ok()).ok_or_else(bad)?;
    let chunked = head.lines().any(|line| line.to_ascii_lowercase().starts_with("transfer-encoding:") && line.to_ascii_lowercase().contains("chunked"));
    if !chunked {
        return Ok((status, body.to_vec()));
    }
    let (mut joined, mut rest) = (Vec::new(), body);
    loop {
        let end = rest.windows(2).position(|w| w == b"\r\n").ok_or_else(bad)?;
        let size = usize::from_str_radix(std::str::from_utf8(&rest[..end]).map_err(|_| bad())?.split(';').next().unwrap_or("").trim(), 16).map_err(|_| bad())?;
        rest = &rest[end + 2..];
        if size == 0 {
            return Ok((status, joined));
        }
        joined.extend_from_slice(rest.get(..size).ok_or_else(bad)?);
        rest = rest.get(size + 2..).ok_or_else(bad)?;
    }
}

/// GET `path` from the engine at `host`, its body read as JSON; `asking`
/// says when it began, while it is under way.
fn ask<T: for<'a> Deserialize<'a>>(host: &str, path: &str, asking: &Mutex<Option<Instant>>) -> std::io::Result<Option<T>> {
    *asking.lock().unwrap() = Some(Instant::now());
    let answer = Connection::open(host).map(|connection| connection.and_then(|connection| connection.get(path)));
    *asking.lock().unwrap() = None;
    let Some(answer) = answer else { return Ok(None) };
    let (status, body) = answer?;
    if status != 200 {
        return Err(std::io::Error::other(format!("{path}: {status}")));
    }
    serde_json::from_slice(&body).map(Some).map_err(std::io::Error::other)
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Listed {
    id: String,
    #[serde(default)]
    names: Vec<String>,
    #[serde(default)]
    status: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Stats {
    cpu_stats: CpuStats,
    memory_stats: MemoryStats,
    networks: Option<HashMap<String, Network>>,
    blkio_stats: Blkio,
}
#[derive(Deserialize, Default)]
#[serde(default)]
struct CpuStats {
    cpu_usage: CpuUsage,
    system_cpu_usage: Option<u64>,
}
#[derive(Deserialize, Default)]
#[serde(default)]
struct CpuUsage {
    total_usage: u64,
}
#[derive(Deserialize, Default)]
#[serde(default)]
struct MemoryStats {
    usage: Option<u64>,
    limit: Option<u64>,
    stats: HashMap<String, u64>,
}
#[derive(Deserialize, Default)]
#[serde(default)]
struct Network {
    rx_bytes: u64,
    tx_bytes: u64,
}
#[derive(Deserialize, Default)]
#[serde(default)]
struct Blkio {
    io_service_bytes_recursive: Option<Vec<BlkioEntry>>,
}
#[derive(Deserialize, Default)]
#[serde(default)]
struct BlkioEntry {
    op: String,
    value: u64,
}

/// A container's running totals, at a moment: what its rates are worked
/// out from, the next time.
struct Counters {
    at: Instant,
    cpu: u64,
    system: u64,
    net: u64,
    io: u64,
}

/// The engine asked once: its containers running, each with what it uses
/// (rates against what `before` holds of the last time).
fn read(before: &mut HashMap<String, Counters>, asking: &Mutex<Option<Instant>>, stop: &AtomicBool) -> DockerSample {
    let Endpoint { context, host } = endpoint();
    let listed: Vec<Listed> = match ask(&host, "/containers/json", asking) {
        Ok(Some(listed)) => listed,
        Ok(None) => return DockerSample::Unsupported { context, host },
        Err(_) => {
            before.clear();
            return DockerSample::Stopped { context };
        }
    };
    let mut containers = Vec::new();
    let mut kept = HashMap::new();
    for container in listed {
        // Switched off meanwhile: no more asked.
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let name = container.names.first().map(|name| name.trim_start_matches('/').to_string()).unwrap_or_else(|| container.id.chars().take(12).collect());
        let stats: Option<Stats> = ask(&host, &format!("/containers/{}/stats?stream=false&one-shot=true", container.id), asking).ok().flatten();
        let Some(stats) = stats else {
            containers.push(ContainerSample { name, status: container.status, cpu: None, mem: None, limit: None, net: None, io: None });
            continue;
        };
        let now = Counters {
            at: Instant::now(),
            cpu: stats.cpu_stats.cpu_usage.total_usage,
            system: stats.cpu_stats.system_cpu_usage.unwrap_or(0),
            net: stats.networks.as_ref().map_or(0, |networks| networks.values().map(|n| n.rx_bytes + n.tx_bytes).sum()),
            io: stats.blkio_stats.io_service_bytes_recursive.as_ref().map_or(0, |entries| {
                entries.iter().filter(|e| e.op.eq_ignore_ascii_case("read") || e.op.eq_ignore_ascii_case("write")).map(|e| e.value).sum()
            }),
        };
        // Its share of the machine's processors, and its rates, since the
        // time before (none the first time it is seen).
        let (cpu, net, io) = match before.get(&container.id) {
            Some(was) => {
                let seconds = now.at.duration_since(was.at).as_secs_f64().max(0.001);
                let system = now.system.saturating_sub(was.system);
                let cpu = (system > 0).then(|| (now.cpu.saturating_sub(was.cpu) as f64 / system as f64 * 100.0).clamp(0.0, 100.0) as f32);
                (cpu, Some(now.net.saturating_sub(was.net) as f64 / seconds), Some(now.io.saturating_sub(was.io) as f64 / seconds))
            }
            None => (None, None, None),
        };
        // In use as the Docker CLI counts it: less the file cache it can drop.
        let cache = stats.memory_stats.stats.get("inactive_file").or_else(|| stats.memory_stats.stats.get("total_inactive_file")).copied().unwrap_or(0);
        let mem = stats.memory_stats.usage.map(|usage| usage.saturating_sub(cache));
        containers.push(ContainerSample { name, status: container.status, cpu, mem, limit: stats.memory_stats.limit, net, io });
        kept.insert(container.id, now);
    }
    *before = kept;
    DockerSample::Running { context, containers }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cuts_short_an_engine_that_does_not_answer() {
        use std::os::windows::io::AsRawHandle;
        use windows::core::w;
        use windows::Win32::Storage::FileSystem::PIPE_ACCESS_DUPLEX;
        use windows::Win32::System::Pipes::{CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE, PIPE_WAIT};
        // An engine that takes the request and never answers.
        let server = unsafe { CreateNamedPipeW(w!(r"\\.\pipe\glance-test-silent"), PIPE_ACCESS_DUPLEX, PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT, 1, 4096, 4096, 0, None) };
        assert!(!server.is_invalid());
        let asking = Arc::new(Mutex::new(None));
        let busy = asking.clone();
        let started = Instant::now();
        let worker = std::thread::spawn(move || ask::<serde_json::Value>("npipe:////./pipe/glance-test-silent", "/containers/json", &busy));
        while asking.lock().unwrap().is_none() {
            std::thread::sleep(Duration::from_millis(10));
        }
        std::thread::sleep(Duration::from_millis(200));
        let thread = windows::Win32::Foundation::HANDLE(worker.as_raw_handle());
        // Cut short, as `Docker::read` does once it has taken too long.
        while !worker.is_finished() {
            let _ = unsafe { windows::Win32::System::IO::CancelSynchronousIo(thread) };
            std::thread::sleep(Duration::from_millis(20));
            assert!(started.elapsed() < Duration::from_secs(5), "the request was not cut short");
        }
        assert!(worker.join().unwrap().is_err());
        assert!(asking.lock().unwrap().is_none());
        let _ = unsafe { windows::Win32::Foundation::CloseHandle(server) };
    }

    #[test]
    fn bounds_an_answer() {
        // An answer that goes on past the bound is no answer.
        let mut endless = std::io::repeat(b'x');
        let mut into = Vec::new();
        assert!(read_all(&mut endless, &mut into).is_err());
        assert!(into.len() <= MOST);
    }

    #[test]
    fn joins_chunks() {
        let answer = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n";
        assert_eq!(parse(answer).unwrap(), (200, b"hello world".to_vec()));
        let plain = b"HTTP/1.1 404 Not Found\r\nContent-Length: 2\r\n\r\n{}";
        assert_eq!(parse(plain).unwrap(), (404, b"{}".to_vec()));
    }

    #[test]
    #[ignore = "asks this machine's Docker engine, with containers running; run with --ignored"]
    fn reads_docker() {
        let (mut before, asking, stop) = (HashMap::new(), Mutex::new(None), AtomicBool::new(false));
        for _ in 0..3 {
            println!("{}", serde_json::to_string(&read(&mut before, &asking, &stop)).unwrap());
            std::thread::sleep(Duration::from_secs(1));
        }
        assert!(matches!(read(&mut before, &asking, &stop), DockerSample::Running { .. }));
    }
}
