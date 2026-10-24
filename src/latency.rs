//! How long the network takes to answer: an echo (a ping) sent to the address chosen in the network module's card (see
//! `prefs::PingTarget`), on a thread of its own, as often as Glance samples, while the module's latency is shown. The
//! system's own echo (`IcmpSendEcho`): no administrator's rights asked for it, nothing of any other program touched.

use std::sync::{Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use windows::Win32::NetworkManagement::IpHelper::{GetBestRoute, IcmpCreateFile, IcmpSendEcho, ICMP_ECHO_REPLY, MIB_IPFORWARDROW};

use crate::ui::prefs::PingTarget;

/// How long an answer is waited for: none by then, none had.
const PATIENCE: Duration = Duration::from_secs(1);
/// What an echo carries (as Windows' own ping sends).
const ECHOED: [u8; 32] = *b"abcdefghijklmnopqrstuvwabcdefghi";

/// What is asked of the thread, and what it last had.
#[derive(Default)]
struct Shared {
    /// Where to, and how often; none: not asked.
    asked: Option<(PingTarget, Duration)>,
    /// The last answer's round trip (ms), and when it was had; none where none came in time.
    latest: Option<(f32, Instant)>,
}

fn shared() -> &'static (Mutex<Shared>, Condvar) {
    static SHARED: OnceLock<(Mutex<Shared>, Condvar)> = OnceLock::new();
    SHARED.get_or_init(|| {
        std::thread::Builder::new().name("glance: latency".into()).spawn(run).expect("a thread started");
        Default::default()
    })
}

/// Echoes sent to `target` every `every` from now (none: no more sent, and what was had forgotten).
pub fn want(target: Option<PingTarget>, every: Duration) {
    let (lock, told) = shared();
    let mut shared = lock.lock().unwrap();
    let asked = target.map(|target| (target, every));
    if shared.asked != asked {
        if asked.is_none() || shared.asked.map(|(was, _)| was) != target {
            shared.latest = None;
        }
        shared.asked = asked;
        told.notify_one();
    }
}

/// The last round trip had (ms), if it is recent: none since the last two echoes went unanswered.
pub fn latest() -> Option<f32> {
    let shared = shared().0.lock().unwrap();
    let (_, every) = shared.asked?;
    shared.latest.filter(|(_, at)| at.elapsed() <= 2 * every + PATIENCE).map(|(ms, _)| ms)
}

fn run() {
    let (lock, told) = shared();
    let Ok(handle) = (unsafe { IcmpCreateFile() }) else {
        crate::journal::note("latency: no echo could be sent (IcmpCreateFile failed)");
        return;
    };
    // (Its handle kept for as long as Glance runs.)
    loop {
        let (target, every) = {
            let mut shared = lock.lock().unwrap();
            loop {
                if let Some(asked) = shared.asked {
                    break asked;
                }
                shared = told.wait(shared).unwrap();
            }
        };
        let began = Instant::now();
        let answered = address(target).and_then(|to| echo(handle, to));
        {
            let mut shared = lock.lock().unwrap();
            // (Asked otherwise meanwhile: what came is of what is no longer asked.)
            if shared.asked.map(|(now, _)| now) == Some(target) {
                if let Some(ms) = answered {
                    shared.latest = Some((ms, Instant::now()));
                }
            }
            let rest = every.saturating_sub(began.elapsed());
            let _ = told.wait_timeout_while(shared, rest, |shared| shared.asked == Some((target, every)));
        }
    }
}

/// An IPv4 address as the system takes one (its bytes in order: network order).
fn ipv4(a: u8, b: u8, c: u8, d: u8) -> u32 {
    u32::from_ne_bytes([a, b, c, d])
}

/// Where `target` is: the router the way out goes through (to the public addresses), or a public resolver's address.
/// None: no way out (nothing connected, or nothing between, as on a network of one's own).
fn address(target: PingTarget) -> Option<u32> {
    match target {
        PingTarget::Gateway => {
            let mut route = MIB_IPFORWARDROW::default();
            let found = unsafe { GetBestRoute(ipv4(223, 5, 5, 5), None, &mut route) } == 0;
            (found && route.dwForwardNextHop != 0).then_some(route.dwForwardNextHop)
        }
        PingTarget::Alibaba => Some(ipv4(223, 5, 5, 5)),
        PingTarget::Tencent => Some(ipv4(119, 29, 29, 29)),
        PingTarget::Cloudflare => Some(ipv4(1, 1, 1, 1)),
    }
}

/// One echo sent to `to`: its round trip (ms), if it was answered in time.
fn echo(handle: windows::Win32::Foundation::HANDLE, to: u32) -> Option<f32> {
    // (Room for the answer, what it carries back, and an error's message.)
    let mut reply = vec![0u8; size_of::<ICMP_ECHO_REPLY>() + ECHOED.len() + 8];
    let answers = unsafe { IcmpSendEcho(handle, to, ECHOED.as_ptr().cast(), ECHOED.len() as u16, None, reply.as_mut_ptr().cast(), reply.len() as u32, PATIENCE.as_millis() as u32) };
    if answers == 0 {
        return None;
    }
    let reply = unsafe { std::ptr::read_unaligned(reply.as_ptr().cast::<ICMP_ECHO_REPLY>()) };
    // (IP_SUCCESS.)
    (reply.Status == 0).then_some(reply.RoundTripTime as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The router the way out goes through answers an echo, as a ping sent by Windows' own would be: a round trip had,
    /// no longer than it was waited for. (Not where nothing is connected.)
    #[test]
    #[ignore = "sends an echo on this PC's network"]
    fn the_way_out_answers_an_echo() {
        let handle = unsafe { IcmpCreateFile() }.unwrap();
        let to = address(PingTarget::Gateway).expect("connected, through a router");
        let ms = echo(handle, to).expect("answered");
        assert!(ms <= PATIENCE.as_millis() as f32, "{ms}");
        unsafe {
            let _ = windows::Win32::NetworkManagement::IpHelper::IcmpCloseHandle(handle);
        }
    }

    /// Asked for, the latency is had as often as asked, of the public resolver chosen; no longer asked, none.
    #[test]
    #[ignore = "sends echoes on this PC's network"]
    fn asked_for_the_latency_is_had_till_it_is_not() {
        want(Some(PingTarget::Alibaba), Duration::from_millis(200));
        let began = Instant::now();
        while latest().is_none() && began.elapsed() < Duration::from_secs(3) {
            std::thread::sleep(Duration::from_millis(20));
        }
        let had = latest();
        want(None, Duration::from_millis(200));
        let after = latest();
        assert!(had.is_some_and(|ms| ms <= PATIENCE.as_millis() as f32), "{had:?}");
        assert_eq!(after, None);
    }
}
