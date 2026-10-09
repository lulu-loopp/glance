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
//! ones mounted now, as a drive plugged in or out really changes them; and
//! so is a game: one in front now, or none.

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
    /// The disks' busy time (which macOS does not keep).
    pub disk_active: bool,
    pub address: bool,
    pub link: bool,
    /// The drives that give their temperature: number, and name.
    pub drives: Vec<(u32, String)>,
    pub volumes: Vec<String>,
    pub board: Option<BoardSeen>,
    pub battery: bool,
    /// The battery's power in and out, and its health.
    pub battery_power: bool,
    pub battery_health: bool,
    /// A game presenting frames (its window covering its screen), now.
    pub game: bool,
    /// What holds back the clock of a game's GPU (which NVIDIA's driver says).
    pub game_limit: bool,
    /// A microphone's mute.
    pub mic: bool,
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
            merge(&mut seen.engines, gpu.engines.iter().flatten().map(|(kind, _)| kind.clone()), |a, b| a == b);
        }
        self.dimms = self.dimms.max(s.dimm_temps.len());
        self.disk_active |= s.disk_active.is_some();
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
        self.battery_power |= s.battery.as_ref().is_some_and(|b| b.watts.is_some());
        self.battery_health |= s.battery.as_ref().is_some_and(|b| b.health.is_some());
        // Found, not taken for one by hand (that is the widgets' own: see
        // `view::lanes_at`).
        self.game = s.game.as_ref().is_some_and(|game| !game.by_hand);
        self.game_limit |= s.game.as_ref().is_some_and(|game| game.gpu_limit.is_some());
        self.mic |= s.mic_muted.is_some();
    }

    /// The readings `other` holds for the lane of module `id` that this
    /// does not, one at a time.
    pub fn news(&self, other: &Seen, id: &str, info: &StaticInfo) -> Vec<Item> {
        let mut news = Vec::new();
        let mut flag = |ours: bool, theirs: bool, item: Item| {
            if theirs && !ours {
                news.push(item);
            }
        };
        match id {
            "cpu" => {
                flag(self.cpu_temp, other.cpu_temp, Item::CpuTemp);
                flag(self.cpu_power, other.cpu_power, Item::CpuPower);
                flag(self.cpu_clock, other.cpu_clock, Item::CpuClock);
                flag(self.threads >= other.threads, true, Item::Threads(other.threads));
                news.extend(other.ccds.iter().filter(|ccd| !self.ccds.contains(ccd)).map(|ccd| Item::Ccd(*ccd)));
            }
            // A module's temperature at a time.
            "memory" => news.extend((self.dimms + 1..=other.dimms).map(Item::Dimms)),
            "network" => {
                flag(self.address, other.address, Item::Address);
                flag(self.link, other.link, Item::Link);
            }
            "disk" => news.extend(other.drives.iter().filter(|(id, _)| !self.drives.iter().any(|(ours, _)| ours == id)).map(|(id, _)| Item::Drive(*id))),
            "storage" => news.extend(other.volumes.iter().filter(|name| !self.volumes.contains(name)).cloned().map(Item::Volume)),
            "board" => {
                let (ours, theirs) = (self.board.clone().unwrap_or_default(), other.board.clone().unwrap_or_default());
                news.extend(theirs.temps.into_iter().filter(|name| !ours.temps.contains(name)).map(Item::BoardTemp));
                news.extend(theirs.fans.into_iter().filter(|name| !ours.fans.contains(name)).map(Item::BoardFan));
            }
            "battery" => {
                flag(self.battery, other.battery, Item::Battery);
                flag(self.battery_power, other.battery_power, Item::BatteryPower);
                flag(self.battery_health, other.battery_health, Item::BatteryHealth);
            }
            "game" => {
                flag(self.game, other.game, Item::Game);
                flag(self.game_limit, other.game_limit, Item::GameLimit);
                flag(self.mic, other.mic, Item::Mic);
            }
            _ => {
                let Some(index) = info.gpu_of(id) else { return news };
                let (ours, theirs) = (self.gpus.get(index).cloned().unwrap_or_default(), other.gpus.get(index).cloned().unwrap_or_default());
                flag(ours.present, theirs.present, Item::Gpu(index, GpuItem::Present));
                flag(ours.temp, theirs.temp, Item::Gpu(index, GpuItem::Temp));
                flag(ours.clock, theirs.clock, Item::Gpu(index, GpuItem::Clock));
                flag(ours.power, theirs.power, Item::Gpu(index, GpuItem::Power));
                flag(ours.fan, theirs.fan, Item::Gpu(index, GpuItem::Fan));
                news.extend(theirs.engines.iter().filter(|kind| !ours.engines.contains(kind)).cloned().map(|kind| Item::Gpu(index, GpuItem::Engine(kind))));
            }
        }
        news
    }

    /// The names of the drives this holds, as `other` has them now (a
    /// model read late): a name takes no more room than the one before.
    pub fn rename(&mut self, other: &Seen) {
        for (id, name) in &mut self.drives {
            if let Some((_, now)) = other.drives.iter().find(|(theirs, _)| theirs == id) {
                name.clone_from(now);
            }
        }
    }

    /// This and `other` together: everything either holds.
    pub fn join(&self, other: &Seen, info: &StaticInfo) -> Seen {
        let mut joined = self.clone();
        let mut lanes: Vec<String> = ["game", "cpu", "memory", "network", "disk", "storage", "board", "battery"].map(String::from).to_vec();
        lanes.extend(info.gpu_modules());
        for id in &lanes {
            for item in joined.news(other, id, info) {
                joined = joined.with(&item, other);
            }
        }
        joined.rename(other);
        joined
    }

    /// This with one more reading, `item` (one of `other`'s news), placed
    /// where `other` has it among what this holds.
    pub fn with(&self, item: &Item, other: &Seen) -> Seen {
        let mut seen = self.clone();
        // A list's new entry goes in after what precedes it in `other`.
        fn add<K: Clone>(ours: &mut Vec<K>, theirs: &[K], new: impl Fn(&K) -> bool, same: impl Fn(&K, &K) -> bool) {
            let kept: Vec<K> = theirs.iter().filter(|key| new(key) || ours.iter().any(|our| same(our, key))).cloned().collect();
            merge(ours, kept, same);
        }
        match item {
            Item::CpuTemp => seen.cpu_temp = true,
            Item::CpuPower => seen.cpu_power = true,
            Item::CpuClock => seen.cpu_clock = true,
            Item::Threads(count) => seen.threads = *count,
            Item::Ccd(ccd) => add(&mut seen.ccds, &other.ccds, |key| key == ccd, |a, b| a == b),
            Item::Dimms(count) => seen.dimms = *count,
            Item::Address => seen.address = true,
            Item::Link => seen.link = true,
            Item::Drive(id) => add(&mut seen.drives, &other.drives, |key| key.0 == *id, |a, b| a.0 == b.0),
            Item::Volume(name) => add(&mut seen.volumes, &other.volumes, |key| key == name, |a, b| a == b),
            Item::BoardTemp(name) | Item::BoardFan(name) => {
                let theirs = other.board.clone().unwrap_or_default();
                let ours = seen.board.get_or_insert_with(BoardSeen::default);
                if matches!(item, Item::BoardTemp(_)) {
                    add(&mut ours.temps, &theirs.temps, |key| key == name, |a, b| a == b);
                } else {
                    add(&mut ours.fans, &theirs.fans, |key| key == name, |a, b| a == b);
                }
            }
            Item::Battery => seen.battery = true,
            Item::BatteryPower => seen.battery_power = true,
            Item::BatteryHealth => seen.battery_health = true,
            Item::Game => seen.game = true,
            Item::GameLimit => seen.game_limit = true,
            Item::Mic => seen.mic = true,
            Item::Gpu(index, gpu) => {
                if seen.gpus.len() <= *index {
                    seen.gpus.resize(index + 1, GpuSeen::default());
                }
                let ours = &mut seen.gpus[*index];
                match gpu {
                    GpuItem::Present => ours.present = true,
                    GpuItem::Temp => ours.temp = true,
                    GpuItem::Clock => ours.clock = true,
                    GpuItem::Power => ours.power = true,
                    GpuItem::Fan => ours.fan = true,
                    GpuItem::Engine(kind) => {
                        let theirs = other.gpus.get(*index).map_or(&[][..], |g| g.engines.as_slice());
                        add(&mut ours.engines, theirs, |key| key == kind, |a, b| a == b);
                    }
                }
            }
        }
        seen
    }
}

/// One reading a lane holds, as a panel already up takes it in.
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    CpuTemp,
    CpuPower,
    CpuClock,
    /// The CPU's threads, this many.
    Threads(usize),
    Ccd(usize),
    /// The memory modules' temperatures, this many (one more).
    Dimms(usize),
    Address,
    Link,
    Drive(u32),
    Volume(String),
    BoardTemp(String),
    BoardFan(String),
    Battery,
    BatteryPower,
    BatteryHealth,
    Game,
    GameLimit,
    Mic,
    /// Of the GPU at this place in `StaticInfo::gpus`.
    Gpu(usize, GpuItem),
}

#[derive(Clone, Debug, PartialEq)]
pub enum GpuItem {
    Present,
    Temp,
    Clock,
    Power,
    Fan,
    Engine(String),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> StaticInfo {
        StaticInfo {
            cpu_name: String::new(),
            memory_modules: None,
            drives: Vec::new(),
            network_adapter: None,
            board: String::new(),
            threads: 1,
            mem_total: 1,
            gpus: Vec::new(),
            found: Vec::new(),
        }
    }

    #[test]
    fn takes_news_a_reading_at_a_time() {
        let (ours, mut theirs) = (Seen::default(), Seen::default());
        theirs.dimms = 2;
        // Two modules' temperatures: one, then the other.
        assert_eq!(ours.news(&theirs, "memory", &info()), [Item::Dimms(1), Item::Dimms(2)]);
        assert_eq!(ours.with(&Item::Dimms(1), &theirs).dimms, 1);
        // A volume goes in where `theirs` has it among ours.
        let ours = Seen { volumes: vec!["C:".into(), "E:".into()], ..Seen::default() };
        let theirs = Seen { volumes: vec!["C:".into(), "D:".into(), "E:".into()], ..Seen::default() };
        assert_eq!(ours.news(&theirs, "storage", &info()), [Item::Volume("D:".into())]);
        assert_eq!(ours.with(&Item::Volume("D:".into()), &theirs).volumes, ["C:", "D:", "E:"]);
    }

    #[test]
    fn joins_what_either_holds() {
        // D: unplugged since; the panel laid out again while up keeps it.
        let held = Seen { volumes: vec!["C:".into(), "D:".into()], drives: vec![(0, "Disk 0".into())], ..Seen::default() };
        let now = Seen { volumes: vec!["C:".into()], drives: vec![(0, "Samsung SSD".into())], battery: true, ..Seen::default() };
        let joined = held.join(&now, &info());
        assert_eq!(joined.volumes, ["C:", "D:"]);
        assert!(joined.battery);
        // A drive's model read late: its name, not a second row.
        assert_eq!(joined.drives, [(0, "Samsung SSD".to_string())]);
    }
}
