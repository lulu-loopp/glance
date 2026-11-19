//! How long the network takes to answer, and how many of its echoes go unanswered: an echo (a ping) sent to the address
//! chosen in the network module's card (see `prefs::PingTarget`), on a thread of its own, as often as Glance samples,
//! while the module's latency or its loss is shown. The system's own echo (`IcmpSendEcho`): no administrator's rights
//! asked for it, nothing of any other program touched.

use std::collections::VecDeque;
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use windows::Win32::NetworkManagement::IpHelper::{GetBestRoute, IcmpCreateFile, IcmpSendEcho, ICMP_ECHO_REPLY, MIB_IPFORWARDROW};

use crate::ui::prefs::PingTarget;

/// How long an answer is waited for: none by then, none had.
const PATIENCE: Duration = Duration::from_secs(1);
/// What an echo carries (as Windows' own ping sends).
const ECHOED: [u8; 32] = *b"abcdefghijklmnopqrstuvwabcdefghi";
/// The echoes the loss is told of: the last so many sent (a minute's, at a sample a second).
pub const LOSS_OF: usize = 60;

/// What is asked of the thread, and what it last had.
#[derive(Default)]
struct Shared {
    /// Where to, and how often; none: not asked.
    asked: Option<(PingTarget, Duration)>,
    /// The last answer's round trip (ms), and when it was had; none where none came in time.
    latest: Option<(f32, Instant)>,
    /// Whether each of the last echoes sent (up to `LOSS_OF`) was answered in time, the latest last, and when the last
    /// was sent.
    sent: VecDeque<bool>,
    sent_at: Option<Instant>,
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
            shared.sent.clear();
            shared.sent_at = None;
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

/// The share of the last echoes sent (up to `LOSS_OF`) that went unanswered in time, from 0 to 1: none before the first
/// was sent, or since the last two went unsent (no way out to send them by).
pub fn loss() -> Option<f32> {
    let shared = shared().0.lock().unwrap();
    let (_, every) = shared.asked?;
    shared.sent_at.filter(|at| at.elapsed() <= 2 * every + PATIENCE)?;
    lost_share(&shared.sent)
}

/// One echo more sent, `answered` in time or not: the oldest let go of past the last `LOSS_OF`.
fn record(sent: &mut VecDeque<bool>, answered: bool) {
    sent.push_back(answered);
    while sent.len() > LOSS_OF {
        sent.pop_front();
    }
}

/// The share of the echoes `sent` that went unanswered; none of none.
fn lost_share(sent: &VecDeque<bool>) -> Option<f32> {
    (!sent.is_empty()).then(|| sent.iter().filter(|answered| !**answered).count() as f32 / sent.len() as f32)
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
        let to = address(target);
        let answered = to.and_then(|to| echo(handle, to));
        {
            let mut shared = lock.lock().unwrap();
            // (Asked otherwise meanwhile: what came is of what is no longer asked.)
            if shared.asked.map(|(now, _)| now) == Some(target) {
                if let Some(ms) = answered {
                    shared.latest = Some((ms, Instant::now()));
                }
                // (An echo with nowhere to go was not sent: not lost.)
                if to.is_some() {
                    record(&mut shared.sent, answered.is_some());
                    shared.sent_at = Some(Instant::now());
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

    /// The loss is the share of the last `LOSS_OF` echoes sent that went unanswered, those before them let go of.
    #[test]
    fn the_loss_is_of_the_last_echoes_sent() {
        let mut sent = VecDeque::new();
        assert_eq!(lost_share(&sent), None);
        record(&mut sent, false);
        assert_eq!(lost_share(&sent), Some(1.0));
        // Ten lost long ago, then three lost among the last sixty.
        let mut sent = VecDeque::new();
        (0..10).for_each(|_| record(&mut sent, false));
        (0..LOSS_OF).for_each(|at| record(&mut sent, !matches!(at, 5 | 30 | 59)));
        assert_eq!((sent.len(), lost_share(&sent)), (LOSS_OF, Some(3.0 / LOSS_OF as f32)));
    }

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

    /// Asked for, the latency and the loss are had as often as asked, of the public resolver chosen; no longer asked,
    /// none.
    #[test]
    #[ignore = "sends echoes on this PC's network"]
    fn asked_for_the_latency_is_had_till_it_is_not() {
        want(Some(PingTarget::Alibaba), Duration::from_millis(200));
        let began = Instant::now();
        while latest().is_none() && began.elapsed() < Duration::from_secs(3) {
            std::thread::sleep(Duration::from_millis(20));
        }
        let (had, lost) = (latest(), loss());
        want(None, Duration::from_millis(200));
        let after = (latest(), loss());
        assert!(had.is_some_and(|ms| ms <= PATIENCE.as_millis() as f32), "{had:?}");
        assert!(lost.is_some_and(|lost| (0.0..1.0).contains(&lost)), "{lost:?}");
        assert_eq!(after, (None, None));
    }
}
