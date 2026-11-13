//! How the PC spends power, as Windows' own settings put it: on the
//! "Balanced" plan, a power mode over it (best power efficiency, balanced,
//! best performance: the three in Settings > System > Power); on another
//! plan, Windows offers no mode, and the plans themselves are the choice
//! (as in the older Control Panel).

use windows::core::{s, w, GUID};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

use crate::actions::{power_plan, power_plans, set_power_plan, POWER_SAVER};

/// Windows' own "Balanced" plan: the one the power modes are over.
const BALANCED: GUID = GUID::from_u128(0x381b4222_f694_41f0_9685_ff5bb260df2e);
/// The power modes, as Windows knows each, with the names they go by here.
const MODES: [(&str, GUID); 3] = [
    ("efficiency", GUID::from_u128(0x961cc777_2547_4f9d_8174_7d86181b8a7a)),
    ("balanced", GUID::from_u128(0)),
    ("performance", GUID::from_u128(0xded574b5_45a0_4f42_8737_46345c09c238)),
];

/// One of powrprof's functions for the power modes, which Windows has (since
/// its 1709) without declaring them.
fn powrprof(name: windows::core::PCSTR) -> Option<unsafe extern "system" fn() -> isize> {
    unsafe { GetProcAddress(LoadLibraryW(w!("powrprof.dll")).ok()?, name) }
}

/// The power mode in force; none where Windows has no such thing.
fn mode() -> Option<GUID> {
    let get: unsafe extern "system" fn(*mut GUID) -> u32 = unsafe { std::mem::transmute(powrprof(s!("PowerGetEffectiveOverlayScheme"))?) };
    let mut mode = GUID::zeroed();
    (unsafe { get(&mut mode) } == 0).then_some(mode)
}

fn set_mode(mode: GUID) -> bool {
    let Some(set) = powrprof(s!("PowerSetActiveOverlayScheme")) else { return false };
    let set: unsafe extern "system" fn(*const GUID) -> u32 = unsafe { std::mem::transmute(set) };
    unsafe { set(&mode) == 0 }
}

/// The mode in force, where modes are the choice: on the Balanced plan.
fn mode_in_force() -> Option<GUID> {
    (power_plan() == Some(BALANCED)).then(mode).flatten()
}

/// One way to spend power: what it is asked for by, its name as Windows
/// has it (a power mode has none here: whoever shows it names it), and
/// whether it is the one in force.
pub struct Choice {
    pub id: String,
    pub name: String,
    pub on: bool,
}

/// The ways there are to choose from now: the power modes, or the plans.
pub fn choices() -> Vec<Choice> {
    match mode_in_force() {
        Some(now) => MODES.iter().map(|(id, mode)| Choice { id: id.to_string(), name: String::new(), on: *mode == now }).collect(),
        None => {
            let now = power_plan();
            power_plans().into_iter().map(|(plan, name)| Choice { id: format!("{plan:?}"), name, on: Some(plan) == now }).collect()
        }
    }
}

/// Goes to the choice asked for by `id` (see `choices`): whether it took.
pub fn choose(id: &str) -> bool {
    if mode_in_force().is_some() {
        return MODES.iter().find(|(name, _)| *name == id).is_some_and(|(_, mode)| set_mode(*mode));
    }
    power_plans().into_iter().find(|(plan, _)| format!("{plan:?}") == id).is_some_and(|(plan, _)| set_power_plan(plan))
}

/// What there was before the most saving way was gone to, to go back to (each GUID as its number, so that it is kept).
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Before {
    Mode(u128),
    Plan(u128),
}

/// Goes to the most saving way there is (the mode of best power
/// efficiency; where modes are not the choice, the Power saver plan):
/// what there was before, none if it was that already.
pub fn save() -> Option<Before> {
    let saving = MODES[0].1;
    match mode_in_force() {
        Some(now) => (now != saving && set_mode(saving)).then_some(Before::Mode(now.to_u128())),
        None => power_plan().filter(|plan| *plan != POWER_SAVER && set_power_plan(POWER_SAVER)).map(|plan| Before::Plan(plan.to_u128())),
    }
}

pub fn restore(before: Before) {
    match before {
        Before::Mode(mode) => set_mode(GUID::from_u128(mode)),
        Before::Plan(plan) => set_power_plan(GUID::from_u128(plan)),
    };
}
