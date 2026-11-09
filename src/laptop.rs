//! Whether this machine is a laptop (or a tablet, a convertible), as its
//! firmware says it in either of two places: the enclosure its SMBIOS table
//! lists, or the power role its ACPI table gives it (the FADT's preferred
//! power management profile, as Windows reads it). Cheap laptops often list
//! their enclosure as of an unknown kind, or even as a desktop's, so either
//! saying so is enough. Neither asks any device anything.

use windows::Win32::System::Power::{
    POWER_PLATFORM_ROLE, POWER_PLATFORM_ROLE_V2, PlatformRoleMobile, PlatformRoleSlate, PowerDeterminePlatformRoleEx,
};

/// The power roles of a machine carried about: a mobile one (a laptop's, a notebook's) and a slate (a tablet's, a
/// convertible's, told apart from a mobile one by the role's second version alone).
const CARRIED: [POWER_PLATFORM_ROLE; 2] = [PlatformRoleMobile, PlatformRoleSlate];

/// Whether the machine whose enclosures are of the chassis types `chassis` and whose firmware gives it the power
/// role `role` is a laptop: either says so (an enclosure carried about, see `smbios::portable`, or the role of a
/// mobile machine or a slate). The role of a desktop, a workstation or a server does not undo a laptop's enclosure,
/// nor a desktop's enclosure a mobile role.
pub fn laptop(chassis: &[u8], role: POWER_PLATFORM_ROLE) -> bool {
    crate::smbios::portable(chassis) || CARRIED.contains(&role)
}

/// The power role this machine's firmware gives it, as Windows reads it from the ACPI table (unspecified where the
/// firmware gives none).
pub fn role() -> POWER_PLATFORM_ROLE {
    unsafe { PowerDeterminePlatformRoleEx(POWER_PLATFORM_ROLE_V2) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::System::Power::{
        PlatformRoleAppliancePC, PlatformRoleDesktop, PlatformRoleEnterpriseServer, PlatformRolePerformanceServer,
        PlatformRoleSOHOServer, PlatformRoleUnspecified, PlatformRoleWorkstation,
    };

    /// A laptop by its enclosure or by its power role, either: the G14 (a notebook, its role mobile) is one, and so
    /// is a machine whose firmware lists a desktop's enclosure or one of an unknown kind but gives it a mobile role
    /// or a slate's. A desktop (this one, its role a desktop's) is not, nor one of a workstation's or a server's role,
    /// nor one whose firmware gives no role.
    #[test]
    fn tells_a_laptop_by_its_chassis_or_its_role() {
        let (g14, desktop) = (crate::smbios::examples::g14(), crate::smbios::examples::desktop());
        let (g14, desktop) = (crate::smbios::chassis(&g14), crate::smbios::chassis(&desktop));
        assert!(laptop(&g14, PlatformRoleMobile));
        assert!(laptop(&g14, PlatformRoleDesktop));
        assert!(laptop(&g14, PlatformRoleUnspecified));
        assert!(laptop(&desktop, PlatformRoleMobile));
        assert!(laptop(&desktop, PlatformRoleSlate));
        assert!(laptop(&[0x02], PlatformRoleMobile));
        assert!(laptop(&[], PlatformRoleMobile));
        for role in [
            PlatformRoleDesktop,
            PlatformRoleWorkstation,
            PlatformRoleEnterpriseServer,
            PlatformRolePerformanceServer,
            PlatformRoleSOHOServer,
            PlatformRoleAppliancePC,
            PlatformRoleUnspecified,
        ] {
            assert!(!laptop(&desktop, role), "{role:?}");
            assert!(!laptop(&[0x02], role), "{role:?}");
        }
    }

    #[test]
    #[ignore = "reads this machine's power role"]
    fn reads_this_machines_role() {
        println!("{:?}", role());
    }
}
