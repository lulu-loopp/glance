//! Finding devices, and being found: LocalSend's multicast group (`224.0.0.167:53317`), on every network this PC is
//! on. This PC announces itself (at start, when its networks change, and when Glance is about to send); a device
//! that hears an announcement registers with the announcer over HTTP(S), and this PC does the same with those it
//! hears (or, where that fails, answers in the group, as the protocol allows).

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddrV4, UdpSocket};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::json;
use socket2::{Domain, Protocol, Socket, Type};

use super::{send, Info, Receiver, PORT};

pub const GROUP: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 167);
/// A device heard within this long is nearby still.
pub const FRESH: Duration = Duration::from_secs(180);
/// When an announcement is sent again after the first (the protocol's burst: one may be lost).
const BURST: [u64; 3] = [100, 500, 2000];
/// How often this PC's networks are looked at again (a network come, the group is joined there, and this PC
/// announced).
const NETWORKS_EVERY: Duration = Duration::from_secs(15);
/// A device is registered with at most this often (its announcements come in bursts).
const REGISTER_EVERY: Duration = Duration::from_secs(5);

pub struct Discovery {
    receiver: Arc<Receiver>,
    /// What announcements go out by (one socket, its network set before each), and the networks it goes out on.
    out: Mutex<(UdpSocket, Vec<Ipv4Addr>)>,
    registered: Mutex<HashMap<String, Instant>>,
}

/// The IPv4 address of each network this PC is on and up (not its loopback).
pub fn networks() -> Vec<Ipv4Addr> {
    use windows::Win32::NetworkManagement::IpHelper::{GetAdaptersAddresses, GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER, GAA_FLAG_SKIP_MULTICAST, IF_TYPE_SOFTWARE_LOOPBACK, IP_ADAPTER_ADDRESSES_LH};
    use windows::Win32::NetworkManagement::Ndis::IfOperStatusUp;
    use windows::Win32::Networking::WinSock::{AF_INET, SOCKADDR_IN};
    let mut size = 16u32 << 10;
    let mut buffer: Vec<u64> = Vec::new();
    for _ in 0..3 {
        buffer = vec![0u64; size as usize / 8 + 1];
        let result = unsafe { GetAdaptersAddresses(AF_INET.0 as u32, GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER, None, Some(buffer.as_mut_ptr().cast()), &mut size) };
        match result {
            0 => break,
            // (Too small: as large as it said, and again.)
            111 => continue,
            _ => return Vec::new(),
        }
    }
    let mut found = Vec::new();
    let mut adapter = buffer.as_ptr() as *const IP_ADAPTER_ADDRESSES_LH;
    while !adapter.is_null() {
        let this = unsafe { &*adapter };
        if this.OperStatus == IfOperStatusUp && this.IfType != IF_TYPE_SOFTWARE_LOOPBACK {
            let mut unicast = this.FirstUnicastAddress;
            while !unicast.is_null() {
                let address = unsafe { &*unicast };
                let socket = address.Address.lpSockaddr;
                if !socket.is_null() && unsafe { (*socket).sa_family } == AF_INET {
                    let v4 = unsafe { &*(socket as *const SOCKADDR_IN) };
                    let ip = Ipv4Addr::from(u32::from_be(unsafe { v4.sin_addr.S_un.S_addr }));
                    if !ip.is_loopback() && !ip.is_link_local() {
                        found.push(ip);
                    }
                }
                unicast = address.Next;
            }
        }
        adapter = this.Next;
    }
    found
}

impl Discovery {
    /// Joins the group on every network, hears it (a thread), and announces this PC. Where the group's port cannot be
    /// listened on (another program holds it alone), this PC is found only by those it announces itself to.
    pub fn start(receiver: Arc<Receiver>) -> std::io::Result<Arc<Discovery>> {
        let out = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
        out.set_multicast_ttl_v4(1)?;
        let discovery = Arc::new(Discovery { receiver, out: Mutex::new((out, Vec::new())), registered: Mutex::default() });
        let heard = (|| -> std::io::Result<Socket> {
            let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
            socket.set_reuse_address(true)?;
            socket.bind(&SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, PORT).into())?;
            socket.set_multicast_loop_v4(true)?;
            Ok(socket)
        })();
        let heard = match heard {
            Ok(socket) => Some(Arc::new(socket)),
            Err(error) => {
                crate::journal::note(format!("nearby: the group's port cannot be listened on ({error}): only announcing"));
                None
            }
        };
        if let Some(heard) = &heard {
            let hearing = discovery.clone();
            let socket = heard.clone();
            std::thread::spawn(move || hearing.hear(&socket));
        }
        // Its networks, joined as they come, and this PC announced on each new one.
        let watching = discovery.clone();
        std::thread::spawn(move || loop {
            let now = networks();
            let new: Vec<Ipv4Addr> = {
                let mut out = watching.out.lock().unwrap();
                let new = now.iter().filter(|network| !out.1.contains(network)).copied().collect();
                out.1 = now.clone();
                new
            };
            if let Some(heard) = &heard {
                for network in &new {
                    // (Joined already, as a network comes back: no harm.)
                    let _ = heard.join_multicast_v4(&GROUP, network);
                }
            }
            if !new.is_empty() {
                watching.announce();
            }
            std::thread::sleep(NETWORKS_EVERY);
        });
        Ok(discovery)
    }

    /// This PC as it announces itself (`announce`: an announcement, which is answered; else an answer).
    fn told(&self, announce: bool) -> Vec<u8> {
        let me = &self.receiver.info;
        json!({ "alias": me.alias, "version": super::VERSION, "deviceModel": me.device_model, "deviceType": me.device_type, "fingerprint": me.fingerprint, "port": me.port, "protocol": "https", "download": false, "announce": announce, "announcement": announce }).to_string().into_bytes()
    }

    /// Sends `what` to the group on every network.
    fn to_all(&self, what: &[u8]) {
        let out = self.out.lock().unwrap();
        let socket = socket2::SockRef::from(&out.0);
        for network in &out.1 {
            if socket.set_multicast_if_v4(network).is_ok() {
                let _ = out.0.send_to(what, (GROUP, PORT));
            }
        }
    }

    /// Announces this PC on every network, in the protocol's burst (a thread).
    pub fn announce(self: &Arc<Self>) {
        let announcing = self.clone();
        std::thread::spawn(move || {
            let what = announcing.told(true);
            let mut waited = 0;
            for at in BURST {
                std::thread::sleep(Duration::from_millis(at - waited));
                waited = at;
                announcing.to_all(&what);
            }
        });
    }

    /// Hears the group: each device known as it says, and one that announces itself registered with.
    fn hear(self: &Arc<Self>, socket: &Socket) {
        let mut packet = [std::mem::MaybeUninit::<u8>::uninit(); 4096];
        loop {
            let Ok((length, from)) = socket.recv_from(&mut packet) else { continue };
            let Some(from) = from.as_socket() else { continue };
            let bytes: Vec<u8> = packet[..length].iter().map(|byte| unsafe { byte.assume_init() }).collect();
            let Ok(info) = serde_json::from_slice::<Info>(&bytes) else { continue };
            if info.fingerprint.is_empty() || info.fingerprint.eq_ignore_ascii_case(&self.receiver.info.fingerprint) {
                continue;
            }
            self.receiver.heard(&info, from.ip(), None);
            let announced = info.announce.unwrap_or(false) || serde_json::from_slice::<serde_json::Value>(&bytes).ok().and_then(|value| value.get("announcement").and_then(serde_json::Value::as_bool)).unwrap_or(false);
            if announced {
                let fresh = {
                    let mut registered = self.registered.lock().unwrap();
                    let fresh = registered.get(&info.fingerprint).is_none_or(|at| at.elapsed() >= REGISTER_EVERY);
                    if fresh {
                        registered.insert(info.fingerprint.clone(), Instant::now());
                    }
                    fresh
                };
                if fresh {
                    let registering = self.clone();
                    let address = from.ip();
                    std::thread::spawn(move || registering.register(address, &info));
                }
            }
        }
    }

    /// Tells the device at `address` that announced `info` of this PC; where it cannot be reached so, answers in the
    /// group instead.
    fn register(&self, address: IpAddr, info: &Info) {
        match send::register(&self.receiver.own, &self.receiver.info, address, info) {
            // (Its fingerprint as the connection proved it where it was pinned to it: as it announced.)
            Ok(answered) => {
                let mut known = answered.clone();
                known.port = info.port;
                known.protocol = info.protocol.clone();
                let https = info.protocol.as_deref() != Some("http");
                self.receiver.heard(&known, address, https.then_some(info.fingerprint.to_ascii_uppercase()).as_deref());
            }
            Err(_) => self.to_all(&self.told(false)),
        }
    }
}
