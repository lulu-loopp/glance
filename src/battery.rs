//! The batteries' own figures, from their drivers (the battery class's
//! IOCTLs, as the system's own battery report reads them): how fast they
//! are charging or discharging, and how much they hold against what they
//! were made to hold. No rights needed.

use windows::core::PCWSTR;
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW, SetupDiGetDeviceInterfaceDetailW, DIGCF_DEVICEINTERFACE, DIGCF_PRESENT,
    SP_DEVICE_INTERFACE_DATA, SP_DEVICE_INTERFACE_DETAIL_DATA_W,
};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Storage::FileSystem::{CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
use windows::Win32::System::Power::{
    BatteryInformation, BATTERY_CAPACITY_RELATIVE, BATTERY_INFORMATION, BATTERY_QUERY_INFORMATION, BATTERY_STATUS, BATTERY_SYSTEM_BATTERY, BATTERY_WAIT_STATUS,
    GUID_DEVICE_BATTERY, IOCTL_BATTERY_QUERY_INFORMATION, IOCTL_BATTERY_QUERY_STATUS, IOCTL_BATTERY_QUERY_TAG,
};
use windows::Win32::System::IO::DeviceIoControl;

/// What the machine's batteries say together.
#[derive(Default)]
pub struct BatteryFigures {
    /// Watts going in (charging, above 0) or out (discharging, below 0):
    /// on battery power, what the whole machine draws.
    pub watts: Option<f32>,
    /// What they hold when full, as a share of what they were made to
    /// hold (percent).
    pub health: Option<f32>,
}

/// The figures of every battery that powers the machine (not a mouse's or
/// a UPS's); none where no battery says them in watts.
pub fn read() -> BatteryFigures {
    let mut watts: Option<f32> = None;
    let (mut full, mut design) = (0u64, 0u64);
    each_battery(|handle| {
        let Some((tag, info)) = information(handle) else { return };
        // A battery of the machine's own, measured in milliwatt-hours (a
        // relative one gives no units to reckon watts with).
        if info.Capabilities & BATTERY_SYSTEM_BATTERY == 0 || info.Capabilities & BATTERY_CAPACITY_RELATIVE != 0 {
            return;
        }
        if info.DesignedCapacity > 0 {
            full += info.FullChargedCapacity as u64;
            design += info.DesignedCapacity as u64;
        }
        // Milliwatts; i32::MIN when the driver does not know.
        if let Some(rate) = status(handle, tag).map(|status| status.Rate).filter(|&rate| rate != i32::MIN) {
            *watts.get_or_insert(0.0) += rate as f32 / 1000.0;
        }
    });
    BatteryFigures { watts, health: (design > 0).then(|| full as f32 / design as f32 * 100.0) }
}

/// Calls `visit` with each battery's device, opened.
fn each_battery(mut visit: impl FnMut(HANDLE)) {
    let Ok(set) = (unsafe { SetupDiGetClassDevsW(Some(&GUID_DEVICE_BATTERY), PCWSTR::null(), None, DIGCF_PRESENT | DIGCF_DEVICEINTERFACE) }) else { return };
    for index in 0.. {
        let mut interface = SP_DEVICE_INTERFACE_DATA { cbSize: size_of::<SP_DEVICE_INTERFACE_DATA>() as u32, ..Default::default() };
        if unsafe { SetupDiEnumDeviceInterfaces(set, None, &GUID_DEVICE_BATTERY, index, &mut interface) }.is_err() {
            break;
        }
        // The path, in a buffer as large as the system asks for.
        let mut needed = 0u32;
        let _ = unsafe { SetupDiGetDeviceInterfaceDetailW(set, &interface, None, 0, Some(&mut needed), None) };
        if needed == 0 {
            continue;
        }
        let mut buffer = vec![0u64; (needed as usize).div_ceil(8)];
        let detail = buffer.as_mut_ptr() as *mut SP_DEVICE_INTERFACE_DETAIL_DATA_W;
        unsafe { (*detail).cbSize = size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32 };
        if unsafe { SetupDiGetDeviceInterfaceDetailW(set, &interface, Some(detail), needed, None, None) }.is_err() {
            continue;
        }
        let path = PCWSTR(unsafe { std::ptr::addr_of!((*detail).DevicePath) } as *const u16);
        let opened = unsafe {
            CreateFileW(path, (FILE_GENERIC_READ | FILE_GENERIC_WRITE).0, FILE_SHARE_READ | FILE_SHARE_WRITE, None, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, None)
        };
        if let Ok(handle) = opened {
            visit(handle);
            let _ = unsafe { CloseHandle(handle) };
        }
    }
    let _ = unsafe { SetupDiDestroyDeviceInfoList(set) };
}

/// The battery's tag (which battery is in the slot now) and what it is.
fn information(handle: HANDLE) -> Option<(u32, BATTERY_INFORMATION)> {
    // Asked without waiting: 0 while no battery is in.
    let (wait, mut tag) = (0u32, 0u32);
    ioctl(handle, IOCTL_BATTERY_QUERY_TAG, &wait, &mut tag)?;
    if tag == 0 {
        return None;
    }
    let query = BATTERY_QUERY_INFORMATION { BatteryTag: tag, InformationLevel: BatteryInformation, AtRate: 0 };
    let mut info = BATTERY_INFORMATION::default();
    ioctl(handle, IOCTL_BATTERY_QUERY_INFORMATION, &query, &mut info)?;
    Some((tag, info))
}

fn status(handle: HANDLE, tag: u32) -> Option<BATTERY_STATUS> {
    let wait = BATTERY_WAIT_STATUS { BatteryTag: tag, ..Default::default() };
    let mut status = BATTERY_STATUS::default();
    ioctl(handle, IOCTL_BATTERY_QUERY_STATUS, &wait, &mut status)?;
    Some(status)
}

fn ioctl<I, O>(handle: HANDLE, code: u32, input: &I, output: &mut O) -> Option<()> {
    let mut returned = 0u32;
    unsafe {
        DeviceIoControl(
            handle,
            code,
            Some(input as *const I as *const _),
            size_of::<I>() as u32,
            Some(output as *mut O as *mut _),
            size_of::<O>() as u32,
            Some(&mut returned),
            None,
        )
    }
    .ok()
    .filter(|_| returned as usize == size_of::<O>())
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "reads this machine's batteries"]
    fn reads_the_batteries() {
        let figures = super::read();
        println!("watts {:?} health {:?}", figures.watts, figures.health);
    }
}
