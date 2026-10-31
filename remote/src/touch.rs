//! What touches the PC, as Windows hands it to a window that asks to hear
//! of it in the background: which device it came from, and whether it said
//! anything was moved or pressed (not what). Not everything Windows takes
//! for a touch is one: a keyboard that is a mouse and a set of media keys
//! besides will now and then report, as each of those, that nothing is
//! moved and nothing pressed.
//!
//! Both Glance and its remote hear of it, the same way (this file is part
//! of each): Windows lets a program hear nothing of what goes to a window
//! of a program run with more rights than its own, so the remote, which
//! has none, is deaf while such a window is in front; Glance's sensors'
//! process, elevated, is not, and Glance tells the remote what it heard.

use windows::Win32::Devices::HumanInterfaceDevice::{
    HidP_GetButtonCaps, HidP_GetCaps, HidP_GetUsageValue, HidP_GetUsages, HidP_GetValueCaps, HidP_Input, HidP_MaxUsageListLength, HIDP_BUTTON_CAPS, HIDP_CAPS, HIDP_STATUS_SUCCESS, HIDP_VALUE_CAPS, PHIDP_PREPARSED_DATA,
};
use windows::Win32::Foundation::{HANDLE, HWND, LPARAM};
use windows::Win32::UI::Input::{
    GetRawInputData, GetRawInputDeviceInfoW, RegisterRawInputDevices, HRAWINPUT, RAWHID, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER, RIDEV_INPUTSINK, RIDEV_REMOVE, RIDI_DEVICENAME, RIDI_PREPARSEDDATA, RID_INPUT, RIM_TYPEKEYBOARD,
    RIM_TYPEMOUSE,
};

/// The kinds of device whose touch wakes the screens: the mouse, the
/// keyboard, a remote's or a keyboard's media keys, a pen, a touch screen,
/// a touch pad (each a usage page and a usage, as HID names them).
/// (The mouse first.)
const TOUCHED_BY: [(u16, u16); 6] = [(0x01, 0x02), (0x01, 0x06), (0x0C, 0x01), (0x0D, 0x02), (0x0D, 0x04), (0x0D, 0x05)];

/// Has `window` told of what is touched (as WM_INPUT, whichever window is
/// in front), or no longer.
#[allow(dead_code)]
pub fn hear(window: HWND, hear: bool) -> windows::core::Result<()> {
    hear_kinds(window, hear, &TOUCHED_BY)
}

/// As `hear`, of every kind but the mouse: for a process that hears the
/// mouse at another window of its own already (a process hears a kind of
/// device at one window only), and passes on what that one hears.
#[allow(dead_code)]
pub fn hear_but_the_mouse(window: HWND, hear: bool) -> windows::core::Result<()> {
    hear_kinds(window, hear, &TOUCHED_BY[1..])
}

fn hear_kinds(window: HWND, hear: bool, kinds: &[(u16, u16)]) -> windows::core::Result<()> {
    let devices: Vec<RAWINPUTDEVICE> = kinds.iter().map(|(page, usage)| RAWINPUTDEVICE { usUsagePage: *page, usUsage: *usage, dwFlags: if hear { RIDEV_INPUTSINK } else { RIDEV_REMOVE }, hwndTarget: if hear { window } else { HWND::default() } }).collect();
    unsafe { RegisterRawInputDevices(&devices, size_of::<RAWINPUTDEVICE>() as u32) }
}

/// What a program that works this PC for someone elsewhere (a phone it streams the desktop to, say) marks the input
/// it makes with, as that input's extra information ("DPKT"): none of it is anyone here touching the PC.
const FROM_ELSEWHERE: usize = 0x4450_4B54;

/// Whether input came from a program that works the PC for someone elsewhere (see `FROM_ELSEWHERE`): made by a
/// program (from no device), and so marked.
fn from_elsewhere(device: HANDLE, extra: usize) -> bool {
    (device.is_invalid() || device.0.is_null()) && extra == FROM_ELSEWHERE
}

/// A device as it is named in a log: what kind it is and whose (its bus,
/// maker and model), without the numbers that tell one unit from another.
fn device(handle: HANDLE) -> String {
    if handle.is_invalid() || handle.0.is_null() {
        return "no device (a program made it)".into();
    }
    let mut name = [0u16; 512];
    let mut length = name.len() as u32;
    let got = unsafe { GetRawInputDeviceInfoW(Some(handle), RIDI_DEVICENAME, Some(name.as_mut_ptr().cast()), &mut length) };
    if got == u32::MAX || got == 0 {
        return "a device that does not say what it is".into();
    }
    let name = String::from_utf16_lossy(&name[..name.iter().position(|c| *c == 0).unwrap_or(name.len())]);
    name.trim_start_matches(r"\\?\").split('#').take(2).collect::<Vec<_>>().join(" ")
}

/// Whether what a device that is neither mouse nor keyboard reported says
/// anything is pressed or set: read as the device itself describes its
/// reports. One whose description cannot be read is taken to say so.
fn says_something(device: HANDLE, hid: &RAWHID) -> bool {
    unsafe {
        let mut length = 0u32;
        GetRawInputDeviceInfoW(Some(device), RIDI_PREPARSEDDATA, None, &mut length);
        // (Kept in words, for the description's alignment.)
        let mut described = vec![0u64; (length as usize).div_ceil(8)];
        if length == 0 || GetRawInputDeviceInfoW(Some(device), RIDI_PREPARSEDDATA, Some(described.as_mut_ptr().cast()), &mut length) == u32::MAX {
            return true;
        }
        let described = PHIDP_PREPARSED_DATA(described.as_ptr() as isize);
        let mut caps = HIDP_CAPS::default();
        if HidP_GetCaps(described, &mut caps) != HIDP_STATUS_SUCCESS {
            return true;
        }
        let mut count = caps.NumberInputButtonCaps;
        let mut buttons = vec![HIDP_BUTTON_CAPS::default(); count as usize];
        if count > 0 && HidP_GetButtonCaps(HidP_Input, buttons.as_mut_ptr(), &mut count, described) != HIDP_STATUS_SUCCESS {
            return true;
        }
        buttons.truncate(count as usize);
        let mut count = caps.NumberInputValueCaps;
        let mut values = vec![HIDP_VALUE_CAPS::default(); count as usize];
        if count > 0 && HidP_GetValueCaps(HidP_Input, values.as_mut_ptr(), &mut count, described) != HIDP_STATUS_SUCCESS {
            return true;
        }
        values.truncate(count as usize);
        let all = std::slice::from_raw_parts(hid.bRawData.as_ptr(), hid.dwSizeHid as usize * hid.dwCount as usize);
        for report in all.chunks(hid.dwSizeHid.max(1) as usize) {
            let mut report = report.to_vec();
            for button in &buttons {
                let mut pressed = HidP_MaxUsageListLength(HidP_Input, Some(button.UsagePage), described);
                let mut usages = vec![0u16; pressed as usize];
                // (How many are pressed comes back in the same count.)
                let read = pressed > 0 && HidP_GetUsages(HidP_Input, button.UsagePage, Some(button.LinkCollection), usages.as_mut_ptr(), &mut pressed, described, &mut report) == HIDP_STATUS_SUCCESS;
                if read && pressed > 0 {
                    return true;
                }
            }
            for value in &values {
                let usages = if value.IsRange { value.Anonymous.Range.UsageMin..=value.Anonymous.Range.UsageMax } else { value.Anonymous.NotRange.Usage..=value.Anonymous.NotRange.Usage };
                for usage in usages {
                    let mut set = 0u32;
                    if HidP_GetUsageValue(HidP_Input, value.UsagePage, Some(value.LinkCollection), usage, &mut set, described, &report) == HIDP_STATUS_SUCCESS && set != 0 {
                        return true;
                    }
                }
            }
        }
        false
    }
}

/// What has been heard while listening: the last touch, and the last
/// report that said nothing was moved and nothing pressed; each by the
/// device it came from and when (Windows' clock of milliseconds).
#[derive(Default)]
pub struct Heard {
    pub touched: Option<(String, u32)>,
    pub nothing: Option<(String, u32)>,
    /// The device last heard from, by its handle, and what it is called:
    /// asked of Windows once, not at every report.
    last: Option<(isize, String)>,
}

impl Heard {
    /// Takes what a WM_INPUT brought, at `now`: whether it said anything
    /// (none, where it could not be read).
    pub fn take(&mut self, input: LPARAM, now: u32) -> Option<bool> {
        let input = HRAWINPUT(input.0 as *mut _);
        let head = size_of::<RAWINPUTHEADER>() as u32;
        let mut size = 0u32;
        unsafe { GetRawInputData(input, RID_INPUT, None, &mut size, head) };
        // (Kept in words, for the alignment of what it is read as.)
        let mut data = vec![0u64; (size as usize).div_ceil(8).max(size_of::<RAWINPUT>().div_ceil(8))];
        if size == 0 || unsafe { GetRawInputData(input, RID_INPUT, Some(data.as_mut_ptr().cast()), &mut size, head) } != size {
            return None;
        }
        let raw = unsafe { &*(data.as_ptr() as *const RAWINPUT) };
        // A mouse that neither moved nor had a button or its wheel touched said nothing; a key is always a touch.
        // Neither says anything that a program working the PC for someone elsewhere made.
        let source = raw.header.hDevice;
        let (kind, said) = match raw.header.dwType {
            kind if kind == RIM_TYPEMOUSE.0 => {
                let mouse = unsafe { raw.data.mouse };
                ("a mouse", !from_elsewhere(source, mouse.ulExtraInformation as usize) && (mouse.lLastX != 0 || mouse.lLastY != 0 || unsafe { mouse.Anonymous.Anonymous.usButtonFlags } != 0))
            }
            kind if kind == RIM_TYPEKEYBOARD.0 => ("a keyboard", !from_elsewhere(source, unsafe { raw.data.keyboard }.ExtraInformation as usize)),
            _ => ("another kind of device", says_something(raw.header.hDevice, unsafe { &raw.data.hid })),
        };
        let handle = raw.header.hDevice.0 as isize;
        if self.last.as_ref().is_none_or(|(last, _)| *last != handle) {
            self.last = Some((handle, device(raw.header.hDevice)));
        }
        let from = format!("{} ({kind})", self.last.as_ref().map_or("", |(_, name)| name));
        *(if said { &mut self.touched } else { &mut self.nothing }) = Some((from, now));
        Some(said)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only input a program made and marked as from elsewhere is so: a device's never, whatever it carries.
    #[test]
    fn input_from_elsewhere_is_a_program_s_and_marked() {
        let nothing = HANDLE(std::ptr::null_mut());
        assert!(from_elsewhere(nothing, FROM_ELSEWHERE));
        assert!(!from_elsewhere(nothing, 0));
        assert!(!from_elsewhere(HANDLE(0x1234 as *mut _), FROM_ELSEWHERE));
    }
}
