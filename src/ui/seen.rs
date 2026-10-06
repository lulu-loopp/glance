//! What the machine has shown it can read since Glance started, which
//! decides the rows and lanes the panel holds; each value comes from the
//! latest sample.
//!
//! Readings come and go with what the hardware is doing: a fan stops, a
//! sensor misses a read, an engine idles out of the counters, the battery
//! goes unanswered for a moment. A panel built from the latest sample alone
//! would gain and lose rows with them, and change its shape. Instead a
//! reading once read keeps its place for as long as Glance runs, showing
//! "—" while it is missing. The volumes are the exception: they are the
//! ones mounted now, as a drive plugged in or out really changes them.

use crate::reading::{Sample, StaticInfo};

#[derive(Clone, Default, Debug, PartialEq)]
pub struct Seen {
    pub cpu_temp: bool,
    pub cpu_power: bool,
    pub cpu_clock: bool,
    /// The chiplets, by number.
    pub ccds: Vec<usize>,
    pub threads: usize,
    /// Each of the machine's GPUs, as `StaticInfo::gpus` lists them.
    pub gpus: Vec<GpuSeen>,
    pub dimms: usize,
    pub address: bool,
    pub link: bool,
    /// The drives that give their temperature: number, and name.
    pub drives: Vec<(u32, String)>,
    pub volumes: Vec<String>,
    pub board: Option<BoardSeen>,
    pub battery: bool,
}

#[derive(Clone, Default, Debug, PartialEq)]
pub struct GpuSeen {
    pub present: bool,
    pub temp: bool,
    pub clock: bool,
    pub power: bool,
    pub fan: bool,
    pub engines: Vec<String>,
}

#[derive(Clone, Default, Debug, PartialEq)]
pub struct BoardSeen {
    pub temps: Vec<String>,
    pub fans: Vec<String>,
}

impl Seen {
    /// What `history`, oldest first, has shown.
    #[cfg(any(test, feature = "studio"))]
    pub fn of(history: &[Sample]) -> Self {
        let mut seen = Seen::default();
        history.iter().for_each(|sample| seen.note(sample));
        seen
    }

    /// Takes in what a new sample shows.
    pub fn note(&mut self, s: &Sample) {
        if let Some(cpu) = &s.cpu_sensors {
            self.cpu_temp |= cpu.temp.is_some();
            self.cpu_power |= cpu.power.is_some();
            merge(&mut self.ccds, cpu.ccds.iter().map(|(ccd, _)| *ccd), |a, b| a == b);
        }
        self.cpu_clock |= s.ghz.is_some();
        self.threads = self.threads.max(s.threads.len());
        if self.gpus.len() < s.gpus.len() {
            self.gpus.resize(s.gpus.len(), GpuSeen::default());
        }
        for (seen, gpu) in self.gpus.iter_mut().zip(&s.gpus) {
            seen.present = true;
            seen.temp |= gpu.temp.is_some();
            seen.clock |= gpu.clock_mhz.is_some();
            seen.power |= gpu.power.is_some();
            seen.fan |= gpu.fan_rpm.is_some();
            merge(&mut seen.engines, gpu.engines.iter().map(|(kind, _)| kind.clone()), |a, b| a == b);
        }
        self.dimms = self.dimms.max(s.dimm_temps.len());
        if let Some(network) = &s.network {
            self.address |= network.ipv4.is_some();
            self.link |= network.link_bps > 0;
        }
        merge(&mut self.drives, s.drive_temps.iter().map(|d| (d.id, d.name.clone())), |a, b| a.0 == b.0);
        self.volumes = s.volumes.iter().map(|v| v.name.clone()).collect();
        if let Some(board) = &s.board {
            let seen = self.board.get_or_insert_with(BoardSeen::default);
            merge(&mut seen.temps, board.temps.iter().map(|(name, _)| name.clone()), |a, b| a == b);
            merge(&mut seen.fans, board.fans.iter().map(|(name, _)| name.clone()), |a, b| a == b);
        }
        self.battery |= s.battery.is_some();
    }

    /// Takes in what `other` holds for the lane of module `id` (and only
    /// for it), keeping what this holds besides: a volume gone in `other`
    /// stays, unread.
    pub fn adopt(&mut self, other: &Seen, id: &str, info: &StaticInfo) {
        match id {
            "cpu" => {
                self.cpu_temp |= other.cpu_temp;
                self.cpu_power |= other.cpu_power;
                self.cpu_clock |= other.cpu_clock;
                merge(&mut self.ccds, other.ccds.iter().copied(), |a, b| a == b);
                self.threads = self.threads.max(other.threads);
            }
            "memory" => self.dimms = self.dimms.max(other.dimms),
            "network" => {
                self.address |= other.address;
                self.link |= other.link;
            }
            "disk" => merge(&mut self.drives, other.drives.iter().cloned(), |a, b| a.0 == b.0),
            "storage" => merge(&mut self.volumes, other.volumes.iter().cloned(), |a, b| a == b),
            "board" => {
                if let Some(theirs) = &other.board {
                    let ours = self.board.get_or_insert_with(BoardSeen::default);
                    merge(&mut ours.temps, theirs.temps.iter().cloned(), |a, b| a == b);
                    merge(&mut ours.fans, theirs.fans.iter().cloned(), |a, b| a == b);
                }
            }
            "battery" => self.battery |= other.battery,
            _ => {
                let Some(theirs) = info.gpu_of(id).and_then(|index| Some((index, other.gpus.get(index)?))) else { return };
                let (index, theirs) = theirs;
                if self.gpus.len() <= index {
                    self.gpus.resize(index + 1, GpuSeen::default());
                }
                let ours = &mut self.gpus[index];
                ours.present |= theirs.present;
                ours.temp |= theirs.temp;
                ours.clock |= theirs.clock;
                ours.power |= theirs.power;
                ours.fan |= theirs.fan;
                merge(&mut ours.engines, theirs.engines.iter().cloned(), |a, b| a == b);
            }
        }
    }
}

/// Takes `now`, in its order, into `known`, which keeps its own: one not
/// known yet goes in after what comes before it in `now`. A name of a
/// known one is brought up to date.
fn merge<K>(known: &mut Vec<K>, now: impl IntoIterator<Item = K>, same: impl Fn(&K, &K) -> bool) {
    let mut after: Option<usize> = None;
    for key in now {
        let at = match known.iter().position(|old| same(old, &key)) {
            Some(at) => {
                known[at] = key;
                at
            }
            None => {
                let at = after.map_or(0, |after| after + 1);
                known.insert(at, key);
                at
            }
        };
        after = Some(at);
    }
}
