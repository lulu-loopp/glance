export type Edge = 'left' | 'right' | 'top';
export type Skin = 'paper' | 'glass' | 'fluent';
export type ViewName = 'monitor' | 'settings';

export interface GpuInfo {
  name: string;
  mem_total: number;
  shared_total: number;
}

export interface StaticInfo {
  cpu_name: string;
  threads: number;
  mem_total: number;
  gpus: GpuInfo[];
}

export interface GpuSample {
  usage: number;
  engines: [string, number][];
  mem_used: number;
  shared_used: number;
  temp: number | null;
  clock_mhz: number | null;
  fan_rpm: number | null;
}

export interface ProcessSample {
  name: string;
  /** Percent of the whole machine. */
  cpu: number;
  mem: number;
}

export interface Sample {
  /** Milliseconds since the Unix epoch. */
  t: number;
  cpu: number;
  threads: number[];
  ghz: number;
  memory: { used: number; committed: number; commit_limit: number; cached: number };
  gpus: GpuSample[];
  net_down: number;
  net_up: number;
  net_total_down: number;
  net_total_up: number;
  network: { name: string; ipv4: string | null; link_bps: number } | null;
  disk_read: number;
  disk_write: number;
  disk_active: number;
  volumes: { name: string; used: number; total: number }[];
  by_cpu: ProcessSample[];
  by_memory: ProcessSample[];
  system: { uptime_s: number; processes: number; threads: number; handles: number };
  battery: { percent: number; charging: boolean; seconds_left: number | null } | null;
}

/** What the panel shows and how; stored by the backend without looking inside. */
export interface ViewPrefs {
  modules: { id: string; on: boolean }[];
  cpu: { threads: boolean; clock: boolean };
  gpu: { memory: boolean; sensors: boolean; engines: boolean };
  memory: { details: boolean };
  network: { bits: boolean; details: boolean };
  disk: { active: boolean };
  processes: { count: number; sort: 'cpu' | 'memory' };
  chartSeconds: number;
  hotLoad: number;
  hotTemp: number;
  /** 'backdrop': light or dark by what is behind the panel. */
  theme: 'system' | 'light' | 'dark' | 'backdrop';
  language: 'system' | 'zh' | 'en';
}

export interface Settings {
  edge: Edge;
  skin: Skin;
  anchor: 'pointer' | 'center';
  sensitivity: 'light' | 'medium' | 'firm';
  close_delay_ms: number;
  interval_ms: number;
  live_backdrop: boolean;
  view: ViewPrefs | null;
}

export interface Bootstrap {
  info: StaticInfo;
  settings: Settings;
  accent: { on_light: string; on_dark: string };
  autostart: boolean;
}
