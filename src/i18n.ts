// Interface language. Every visible string goes through t().

export type Language = 'zh' | 'en';

const STRINGS = {
  uptime: { zh: '已开机 {0}', en: 'Up {0}' },
  days: { zh: '{0} 天 {1} 小时', en: '{0} d {1} h' },
  hours: { zh: '{0} 小时 {1} 分', en: '{0} h {1} min' },
  minutes: { zh: '{0} 分钟', en: '{0} min' },
  settings: { zh: '设置', en: 'Settings' },
  windowTitle: { zh: 'Glance 设置', en: 'Glance Settings' },
  memory: { zh: '内存', en: 'Memory' },
  network: { zh: '网络', en: 'Network' },
  disk: { zh: '磁盘', en: 'Disk' },
  processes: { zh: '进程', en: 'Processes' },
  storage: { zh: '存储', en: 'Storage' },
  storageDetail: { zh: '各分区的空间', en: 'Space on each drive' },
  battery: { zh: '电池', en: 'Battery' },
  batteryDetail: { zh: '笔记本电脑', en: 'Laptops' },
  system: { zh: '系统', en: 'System' },
  systemDetail: { zh: '开机时长、进程和句柄数', en: 'Uptime, processes, handles' },
  vram: { zh: '显存', en: 'VRAM' },
  clock: { zh: '频率', en: 'Clock' },
  fan: { zh: '风扇', en: 'Fan' },
  sharedVram: { zh: '共享显存', en: 'Shared' },
  committed: { zh: '已提交', en: 'Committed' },
  cached: { zh: '缓存', en: 'Cached' },
  download: { zh: '下载', en: 'Down' },
  upload: { zh: '上传', en: 'Up' },
  read: { zh: '读取', en: 'Read' },
  write: { zh: '写入', en: 'Write' },
  fullScale: { zh: '满刻度 {0}', en: 'Scale {0}' },
  adapter: { zh: '网卡', en: 'Adapter' },
  notConnected: { zh: '未连接', en: 'Not connected' },
  address: { zh: '地址', en: 'Address' },
  link: { zh: '链路', en: 'Link' },
  sinceBoot: { zh: '开机以来', en: 'Since boot' },
  sinceBootValue: { zh: '下载 {0}，上传 {1}', en: '{0} down, {1} up' },
  activeTime: { zh: '活动时间', en: 'Active time' },
  sortedBy: { zh: '按{0}排序', en: 'By {0}' },
  uptimeFact: { zh: '开机时长', en: 'Uptime' },
  processCount: { zh: '进程', en: 'Processes' },
  threadCount: { zh: '线程', en: 'Threads' },
  handleCount: { zh: '句柄', en: 'Handles' },
  charging: { zh: '正在充电', en: 'Charging' },
  onBattery: { zh: '使用电池', en: 'On battery' },
  remaining: { zh: '剩余 {0}', en: '{0} left' },
  threadsTitle: { zh: '{0} 个线程', en: '{0} threads' },
  engine3D: { zh: '3D', en: '3D' },
  engineCopy: { zh: '复制', en: 'Copy' },
  engineVideoDecode: { zh: '视频解码', en: 'Video decode' },
  engineVideoEncode: { zh: '视频编码', en: 'Video encode' },
  engineVideoCodec: { zh: '视频编解码', en: 'Video codec' },
  engineCompute: { zh: '计算', en: 'Compute' },
  // Settings
  appearance: { zh: '外观', en: 'Appearance' },
  skinPaper: { zh: '记录纸', en: 'Chart paper' },
  skinGlass: { zh: '液态玻璃', en: 'Liquid glass' },
  skinFluent: { zh: 'Windows 11', en: 'Windows 11' },
  theme: { zh: '明暗', en: 'Theme' },
  themeSystem: { zh: '跟随系统', en: 'System' },
  themeLight: { zh: '浅色', en: 'Light' },
  themeDark: { zh: '深色', en: 'Dark' },
  language: { zh: '语言', en: 'Language' },
  opening: { zh: '呼出', en: 'Opening' },
  edge: { zh: '屏幕边缘', en: 'Screen edge' },
  left: { zh: '左侧', en: 'Left' },
  right: { zh: '右侧', en: 'Right' },
  anchor: { zh: '面板位置', en: 'Position' },
  anchorPointer: { zh: '跟随指针', en: 'At pointer' },
  anchorCenter: { zh: '居中', en: 'Centred' },
  push: { zh: '推入力度', en: 'Push' },
  pushLight: { zh: '轻', en: 'Light' },
  pushMedium: { zh: '中', en: 'Medium' },
  pushFirm: { zh: '重', en: 'Firm' },
  closeDelay: { zh: '离开后收起', en: 'Close after' },
  closeNow: { zh: '立即', en: 'At once' },
  seconds: { zh: '{0} 秒', en: '{0} s' },
  shown: { zh: '显示内容', en: 'Shown' },
  dragToReorder: { zh: '拖动以调整顺序', en: 'Drag to reorder' },
  details: { zh: '细节', en: 'Details' },
  cpuThreads: { zh: 'CPU 线程', en: 'CPU threads' },
  cpuClock: { zh: 'CPU 频率', en: 'CPU clock' },
  gpuMemory: { zh: '显存', en: 'Video memory' },
  gpuSensors: { zh: 'GPU 温度、频率和风扇', en: 'GPU temperature, clock and fan' },
  gpuEngines: { zh: 'GPU 各引擎', en: 'GPU engines' },
  gpuEnginesDetail: { zh: '3D、复制、视频编解码', en: '3D, copy, video' },
  memoryDetails: { zh: '内存提交量和缓存', en: 'Committed and cached memory' },
  networkDetails: { zh: '网卡、地址和累计流量', en: 'Adapter, address and totals' },
  diskActive: { zh: '磁盘活动时间', en: 'Disk active time' },
  rateUnit: { zh: '网速单位', en: 'Network unit' },
  processCountSetting: { zh: '进程数量', en: 'Processes shown' },
  processSort: { zh: '进程排序', en: 'Sort processes by' },
  data: { zh: '数据', en: 'Data' },
  interval: { zh: '刷新间隔', en: 'Refresh every' },
  span: { zh: '曲线时长', en: 'Graph spans' },
  minutesShort: { zh: '{0} 分', en: '{0} min' },
  loadAlert: { zh: '负载警示', en: 'Load alert' },
  tempAlert: { zh: '温度警示', en: 'Temperature alert' },
  startup: { zh: '开机时启动', en: 'Start with Windows' },
  quit: { zh: '退出 Glance', en: 'Quit Glance' },
} satisfies Record<string, Record<Language, string>>;

export type Key = keyof typeof STRINGS;

let current: Language = 'zh';

/** Settles the language from the preference: a choice, or the system's. */
export function setLanguage(preference: 'system' | Language) {
  current = preference === 'system' ? (navigator.language.toLowerCase().startsWith('zh') ? 'zh' : 'en') : preference;
  document.documentElement.lang = current === 'zh' ? 'zh-CN' : 'en';
}

export function language(): Language {
  return current;
}

/** The string for `key`, with {0}, {1}… replaced by `args`. */
export function t(key: Key, ...args: (string | number)[]): string {
  return STRINGS[key][current].replace(/\{(\d)\}/g, (_, i) => String(args[Number(i)]));
}
