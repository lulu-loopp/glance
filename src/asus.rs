//! The fans of ASUS laptops (ROG, TUF, Zenbook…), as their firmware's own
//! ACPI interface reports them to ASUS's software: the ATK device's DSTS
//! method, reached through WMI (root\wmi, AsusAtkWmi_WMNB). Which device
//! is which, and how a speed is given, are as Linux's asus-wmi driver has
//! them. A laptop's fans are its embedded controller's; this asks the
//! firmware, which knows, instead of reading the controller.

use windows::core::BSTR;
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::System::Com::{CoCreateInstance, CoSetProxyBlanket, CLSCTX_INPROC_SERVER, EOAC_NONE, RPC_C_AUTHN_LEVEL_CALL, RPC_C_IMP_LEVEL_IMPERSONATE};
use windows::Win32::System::Rpc::{RPC_C_AUTHN_WINNT, RPC_C_AUTHZ_NONE};
use windows::Win32::System::Wmi::{IWbemClassObject, IWbemLocator, IWbemServices, WbemLocator, WBEM_FLAG_FORWARD_ONLY, WBEM_FLAG_RETURN_WBEM_COMPLETE, WBEM_INFINITE};

/// The fans the firmware may have: device id, and the name the page
/// translates.
const FANS: [(u32, &str); 3] = [(0x0011_0013, "cpu_fan"), (0x0011_0014, "gpu_fan"), (0x0011_0031, "mid_fan")];
/// Set in a device's status when the device is there.
const PRESENT: u32 = 0x0001_0000;
/// A status the method gives for a device the firmware does not have.
const UNSUPPORTED: u32 = 0xFFFF_FFFE;

pub struct AsusFans {
    services: IWbemServices,
    /// The ATK device's object path, and DSTS's input, to be filled in.
    path: BSTR,
    input: IWbemClassObject,
    /// The fans this laptop has.
    fans: Vec<(u32, &'static str)>,
}

impl AsusFans {
    /// The laptop's fans, if it is an ASUS one whose firmware reports them.
    pub fn open() -> Option<Self> {
        let locator: IWbemLocator = unsafe { CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER) }.ok()?;
        let empty = BSTR::new();
        let services = unsafe { locator.ConnectServer(&BSTR::from(r"root\wmi"), &empty, &empty, &empty, 0, &empty, None) }.ok()?;
        unsafe { CoSetProxyBlanket(&services, RPC_C_AUTHN_WINNT, RPC_C_AUTHZ_NONE, None, RPC_C_AUTHN_LEVEL_CALL, RPC_C_IMP_LEVEL_IMPERSONATE, None, EOAC_NONE) }.ok()?;
        let class = BSTR::from("AsusAtkWmi_WMNB");
        // The ATK device: its object's path.
        let objects = unsafe { services.CreateInstanceEnum(&class, WBEM_FLAG_FORWARD_ONLY | WBEM_FLAG_RETURN_WBEM_COMPLETE, None) }.ok()?;
        let mut found = [None];
        let mut returned = 0u32;
        let _ = unsafe { objects.Next(WBEM_INFINITE, &mut found, &mut returned) };
        let device = found[0].take().filter(|_| returned == 1)?;
        let path = BSTR::try_from(&get(&device, "__PATH")?).ok()?;
        // DSTS's input, from the class's definition of the method.
        let mut definition = None;
        unsafe { services.GetObject(&class, Default::default(), None, Some(&mut definition), None) }.ok()?;
        let (mut input, mut output) = (None, None);
        unsafe { definition?.GetMethod(windows::core::w!("DSTS"), 0, &mut input, &mut output) }.ok()?;
        let mut fans = AsusFans { services, path, input: input?, fans: Vec::new() };
        fans.fans = FANS.into_iter().filter(|(id, _)| fans.status(*id).is_some_and(|status| status != UNSUPPORTED && status & PRESENT != 0)).collect();
        (!fans.fans.is_empty()).then_some(fans)
    }

    /// Each fan's speed (RPM), by name.
    pub fn read(&self) -> Vec<(String, f32)> {
        // The speed in hundreds of RPM, in the status's low word.
        self.fans.iter().filter_map(|(id, name)| Some((name.to_string(), (self.status(*id)? & 0xFFFF) as f32 * 100.0))).collect()
    }

    /// What the firmware says of device `id` (DSTS).
    fn status(&self, id: u32) -> Option<u32> {
        let input = unsafe { self.input.SpawnInstance(0) }.ok()?;
        // A uint32 goes to WMI as a 32-bit integer.
        unsafe { input.Put(windows::core::w!("Device_ID"), 0, &VARIANT::from(id as i32), 0) }.ok()?;
        let mut output = None;
        unsafe { self.services.ExecMethod(&self.path, &BSTR::from("DSTS"), Default::default(), None, &input, Some(&mut output), None) }.ok()?;
        let status = get(output.as_ref()?, "device_status")?;
        i32::try_from(&status).ok().map(|status| status as u32)
    }

    /// The fans found, for a report.
    pub fn describe(&self) -> String {
        format!("ASUS firmware: {}", self.fans.iter().map(|(_, name)| *name).collect::<Vec<_>>().join(", "))
    }
}

fn get(object: &IWbemClassObject, name: &str) -> Option<VARIANT> {
    let mut value = VARIANT::default();
    let name: Vec<u16> = name.encode_utf16().chain([0]).collect();
    unsafe { object.Get(windows::core::PCWSTR(name.as_ptr()), 0, &mut value, None, None) }.ok()?;
    Some(value)
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "asks this machine's firmware"]
    fn reads_the_fans() {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        let fans = super::AsusFans::open();
        println!("{:?}", fans.as_ref().map(|fans| (fans.describe(), fans.read())));
    }
}
