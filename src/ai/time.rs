//! When this PC's day began.

/// When the day the moment `now` (ms since 1970) falls in began on this PC's clock, its midnight (ms since 1970).
#[cfg(windows)]
pub fn today_start(now: u64) -> u64 {
    use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
    use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToFileTime, SystemTimeToTzSpecificLocalTime, TzSpecificLocalTimeToSystemTime};
    // (A file time counts hundreds of nanoseconds from 1601.)
    const FROM_1601_MS: u64 = 11_644_473_600_000;
    let ticks = (now + FROM_1601_MS) * 10_000;
    let file = FILETIME { dwLowDateTime: ticks as u32, dwHighDateTime: (ticks >> 32) as u32 };
    let (mut utc, mut local) = (SYSTEMTIME::default(), SYSTEMTIME::default());
    let midnight = unsafe {
        FileTimeToSystemTime(&file, &mut utc).ok().and_then(|_| SystemTimeToTzSpecificLocalTime(None, &utc, &mut local).ok()).and_then(|_| {
            let local_midnight = SYSTEMTIME { wHour: 0, wMinute: 0, wSecond: 0, wMilliseconds: 0, ..local };
            let mut utc_midnight = SYSTEMTIME::default();
            TzSpecificLocalTimeToSystemTime(None, &local_midnight, &mut utc_midnight).ok()?;
            let mut file = FILETIME::default();
            SystemTimeToFileTime(&utc_midnight, &mut file).ok()?;
            Some(((file.dwHighDateTime as u64) << 32 | file.dwLowDateTime as u64) / 10_000 - FROM_1601_MS)
        })
    };
    // (The clock unreadable: the day as UTC counts it.)
    midnight.unwrap_or(now - now % super::DAY_MS)
}

#[cfg(target_os = "macos")]
pub fn today_start(now: u64) -> u64 {
    let seconds = (now / 1000) as libc::time_t;
    let mut local: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&seconds, &mut local) };
    local.tm_hour = 0;
    local.tm_min = 0;
    local.tm_sec = 0;
    // (Whether summer time is kept at midnight: as the clock says.)
    local.tm_isdst = -1;
    let midnight = unsafe { libc::mktime(&mut local) };
    if midnight < 0 { now - now % super::DAY_MS } else { midnight as u64 * 1000 }
}
