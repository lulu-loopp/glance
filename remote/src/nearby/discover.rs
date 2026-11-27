//! Finding devices, and being found: LocalSend's multicast group (`224.0.0.167:53317`), on every network this PC is
//! on. This PC announces itself (at start, when its networks change, and when Glance is about to send); a device
//! that hears an announcement registers with the announcer over HTTP(S), and this PC does the same with those it
//! hears (or, where that fails, answers in the group, as the protocol allows).

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddrV4, UdpSocket};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
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

/// The IPv4 addresses among `addresses`: the networks this PC announces itself on.
fn networks_of(addresses: &[Address]) -> Vec<Ipv4Addr> {
    addresses
        .iter()
        .filter_map(|address| match address.address {
            IpAddr::V4(v4) => Some(v4),
            IpAddr::V6(_) => None,
        })
        .collect()
}

/// An address another device reaches this PC at: on which network (as Windows names it), the address, and whether
/// that network leads beyond this PC (it has a gateway: not a virtual machine's network inside it, as WSL's and
/// Hyper-V's own are, where no other device is).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Address {
    pub network: String,
    pub address: IpAddr,
    #[serde(skip)]
    pub routed: bool,
}

/// An IPv6 address of a network as Windows keeps it: the address, how its prefix and the rest came (Windows'
/// `NL_PREFIX_ORIGIN` and `NL_SUFFIX_ORIGIN`), and how long it is valid (seconds).
#[derive(Clone, Copy, Debug)]
pub struct V6 {
    pub address: Ipv6Addr,
    pub prefix: i32,
    pub suffix: i32,
    pub valid: u32,
}

/// The addresses of a network another device may reach this PC at over IPv6 (ones of the Internet's, 2000::/3: an
/// address of the link alone, fe80::, reaches nothing beyond it), the one meant to last longest: given by hand or by
/// the network's DHCP, then one made of the network card's own address, then the one valid longest (an address made
/// up for privacy is replaced within a day or a week). None where there is none.
pub fn lasting(addresses: &[V6]) -> Option<Ipv6Addr> {
    // (Windows' NL_PREFIX_ORIGIN: IpPrefixOriginManual 1, IpPrefixOriginDhcp 3. NL_SUFFIX_ORIGIN: IpSuffixOriginManual
    // 1, IpSuffixOriginDhcp 3, IpSuffixOriginLinkLayerAddress 4.)
    let rank = |v6: &V6| match (v6.prefix, v6.suffix) {
        (1 | 3, _) | (_, 1 | 3) => 0,
        (_, 4) => 1,
        _ => 2,
    };
    addresses.iter().filter(|v6| v6.address.segments()[0] & 0xe000 == 0x2000).min_by_key(|v6| (rank(v6), std::cmp::Reverse(v6.valid))).map(|v6| v6.address)
}

/// What other devices may reach this PC at, network by network (each up, not its loopback): its IPv6 address meant
/// to last (see `lasting`), first, then its IPv4 ones (not those of the link alone, 169.254.x.x). Only addresses in
/// use are counted (an IPv6 one still being checked as unique, or deprecated, is not).
pub fn addresses() -> Vec<Address> {
    use windows::Win32::NetworkManagement::IpHelper::{GetAdaptersAddresses, GAA_FLAG_INCLUDE_GATEWAYS, GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER, GAA_FLAG_SKIP_MULTICAST, IF_TYPE_SOFTWARE_LOOPBACK, IP_ADAPTER_ADDRESSES_LH};
    use windows::Win32::NetworkManagement::Ndis::IfOperStatusUp;
    use windows::Win32::Networking::WinSock::{IpDadStatePreferred, AF_INET, AF_INET6, AF_UNSPEC, SOCKADDR_IN, SOCKADDR_IN6};
    let mut size = 16u32 << 10;
    let mut buffer: Vec<u64> = Vec::new();
    for _ in 0..3 {
        buffer = vec![0u64; size as usize / 8 + 1];
        let result = unsafe { GetAdaptersAddresses(AF_UNSPEC.0 as u32, GAA_FLAG_INCLUDE_GATEWAYS | GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER, None, Some(buffer.as_mut_ptr().cast()), &mut size) };
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
            let network = unsafe { this.FriendlyName.to_string() }.unwrap_or_default();
            let routed = !this.FirstGatewayAddress.is_null();
            let (mut v4s, mut v6s) = (Vec::new(), Vec::new());
            let mut unicast = this.FirstUnicastAddress;
            while !unicast.is_null() {
                let address = unsafe { &*unicast };
                let socket = address.Address.lpSockaddr;
                if !socket.is_null() && unsafe { (*socket).sa_family } == AF_INET {
                    let v4 = unsafe { &*(socket as *const SOCKADDR_IN) };
                    let ip = Ipv4Addr::from(u32::from_be(unsafe { v4.sin_addr.S_un.S_addr }));
                    if !ip.is_loopback() && !ip.is_link_local() {
                        v4s.push(ip);
                    }
                } else if !socket.is_null() && unsafe { (*socket).sa_family } == AF_INET6 && address.DadState == IpDadStatePreferred {
                    let v6 = unsafe { &*(socket as *const SOCKADDR_IN6) };
                    v6s.push(V6 { address: Ipv6Addr::from(unsafe { v6.sin6_addr.u.Byte }), prefix: address.PrefixOrigin.0, suffix: address.SuffixOrigin.0, valid: address.ValidLifetime });
                }
                unicast = address.Next;
            }
            found.extend(lasting(&v6s).map(|v6| Address { network: network.clone(), address: IpAddr::V6(v6), routed }));
            found.extend(v4s.into_iter().map(|v4| Address { network: network.clone(), address: IpAddr::V4(v4), routed }));
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
            // (The addresses on networks that lead beyond this PC kept by the receiver, to tell other devices and
            // Glance; every IPv4 one a network announced on.)
            let addresses = addresses();
            watching.receiver.addressed(addresses.iter().filter(|address| address.routed).cloned().collect());
            let now = networks_of(&addresses);
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
        let me = &self.receiver.me();
        json!({ "alias": me.alias, "version": super::VERSION, "deviceModel": me.device_model, "deviceType": me.device_type, "fingerprint": me.fingerprint, "port": me.port, "protocol": "https", "download": false, "announce": announce, "announcement": announce, "glance": true }).to_string().into_bytes()
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
        match send::register(&self.receiver.own, &self.receiver.me(), address, info) {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A network's IPv6 address told to other devices is one of the Internet's (never one of the link alone, nor a
    /// local one), the one meant to last: given by hand or by DHCP, then one made of the card's own address, then the
    /// one valid longest (not an address made up for privacy, replaced within days).
    #[test]
    fn the_ipv6_address_told_is_the_one_meant_to_last() {
        let v6 = |text: &str, prefix: i32, suffix: i32, valid: u32| V6 { address: text.parse().unwrap(), prefix, suffix, valid };
        // (Prefix by router advertisement 4, suffix random 5: a privacy address, valid a day, beside the lasting one.)
        let made_up = v6("2001:da8:215:3c01:a1b2:c3d4:e5f6:1", 4, 5, 86_400);
        let lasting_random = v6("2001:da8:215:3c01:1111:2222:3333:4444", 4, 5, 2_592_000);
        let link = v6("fe80::1", 0, 4, u32::MAX);
        let local = v6("fd00::1", 1, 1, u32::MAX);
        assert_eq!(lasting(&[made_up, lasting_random, link, local]), Some(lasting_random.address));
        let card = v6("2001:da8:215:3c01:2e0:4cff:fe68:1", 4, 4, 86_400);
        assert_eq!(lasting(&[made_up, lasting_random, card]), Some(card.address));
        let dhcp = v6("2001:da8:215:3c01::5a2", 3, 3, 3_600);
        assert_eq!(lasting(&[made_up, card, dhcp]), Some(dhcp.address));
        assert_eq!(lasting(&[link, local]), None);
        assert_eq!(lasting(&[]), None);
    }
}
