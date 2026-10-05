import { element, escapeHtml } from './format';
import { DEFAULT_OFF, type ModuleDef } from './modules';
import type { Settings, Skin, ViewPrefs } from './types';

const DEFAULT_PREFS: Omit<ViewPrefs, 'modules'> = {
  cpu: { threads: true, clock: true },
  gpu: { memory: true, sensors: true, engines: false },
  memory: { details: false },
  network: { bits: false, details: false },
  disk: { active: false },
  processes: { count: 5, sort: 'cpu' },
  chartSeconds: 60,
  hotLoad: 85,
  hotTemp: 85,
};

/**
 * Stored preferences completed with defaults, with the module list matched to
 * this machine: modules it no longer has are dropped, new ones appended.
 */
export function resolvePrefs(stored: ViewPrefs | null, modules: ModuleDef[]): ViewPrefs {
  const merged = structuredClone({ ...DEFAULT_PREFS, modules: [] }) as ViewPrefs;
  if (stored) {
    for (const key of Object.keys(DEFAULT_PREFS) as (keyof typeof DEFAULT_PREFS)[]) {
      const value = stored[key];
      if (value === undefined) continue;
      (merged as any)[key] = typeof value === 'object' ? { ...(merged as any)[key], ...value } : value;
    }
  }
  const known = new Set(modules.map((module) => module.id));
  const kept = (stored?.modules ?? []).filter((entry) => known.has(entry.id));
  const listed = new Set(kept.map((entry) => entry.id));
  merged.modules = [
    ...kept,
    ...modules.filter((module) => !listed.has(module.id)).map((module) => ({ id: module.id, on: !DEFAULT_OFF.has(module.id) })),
  ];
  return merged;
}

type Choice<T> = { value: T; label: string };

function segmented<T>(label: string, choices: Choice<T>[], current: T, pick: (value: T) => void): HTMLElement {
  const row = element(`<div class="field"><span class="field-label">${label}</span><div class="segmented" role="radiogroup"></div></div>`);
  const group = row.querySelector<HTMLElement>('.segmented')!;
  const buttons = choices.map((choice) => {
    const button = element<HTMLButtonElement>(`<button type="button" role="radio">${choice.label}</button>`);
    button.addEventListener('click', () => {
      buttons.forEach((b, i) => b.setAttribute('aria-checked', String(choices[i].value === choice.value)));
      pick(choice.value);
    });
    button.setAttribute('aria-checked', String(choice.value === current));
    return button;
  });
  group.append(...buttons);
  return row;
}

function toggle(label: string, checked: boolean, change: (on: boolean) => void, hint = ''): HTMLElement {
  const row = element(`
    <label class="field field-toggle">
      <span class="field-label">${label}${hint ? `<small>${escapeHtml(hint)}</small>` : ''}</span>
      <button type="button" class="switch" role="switch"></button>
    </label>`);
  const button = row.querySelector<HTMLButtonElement>('.switch')!;
  const show = (on: boolean) => button.setAttribute('aria-checked', String(on));
  show(checked);
  button.addEventListener('click', (event) => {
    event.preventDefault();
    const on = button.getAttribute('aria-checked') !== 'true';
    show(on);
    change(on);
  });
  return row;
}

function section(title: string, ...children: HTMLElement[]): HTMLElement {
  const root = element(`<section class="settings-section"><h3>${title}</h3></section>`);
  root.append(...children);
  return root;
}

const SKINS: { value: Skin; label: string }[] = [
  { value: 'paper', label: '记录纸' },
  { value: 'glass', label: '液态玻璃' },
  { value: 'fluent', label: 'Windows 11' },
];

function skinPicker(current: Skin, pick: (skin: Skin) => void): HTMLElement {
  const root = element('<div class="skins" role="radiogroup"></div>');
  const cards = SKINS.map((skin) => {
    const card = element<HTMLButtonElement>(`
      <button type="button" class="skin-card" role="radio" data-preview="${skin.value}">
        <span class="skin-preview"><i></i><i></i><i></i></span>
        <span>${skin.label}</span>
      </button>`);
    card.setAttribute('aria-checked', String(skin.value === current));
    card.addEventListener('click', () => {
      cards.forEach((c, i) => c.setAttribute('aria-checked', String(SKINS[i].value === skin.value)));
      pick(skin.value);
    });
    return card;
  });
  root.append(...cards);
  return root;
}

/** The module list: a switch per module, dragged by its handle to reorder. */
function moduleList(prefs: ViewPrefs, modules: ModuleDef[], changed: () => void): HTMLElement {
  const byId = new Map(modules.map((module) => [module.id, module]));
  const list = element('<ol class="module-list"></ol>');
  for (const entry of prefs.modules) {
    const module = byId.get(entry.id)!;
    const row = element(`
      <li class="module-row" data-id="${entry.id}">
        <span class="grip" title="拖动以调整顺序"><svg viewBox="0 0 8 14" aria-hidden="true"><circle cx="2" cy="2" r="1.2"/><circle cx="6" cy="2" r="1.2"/><circle cx="2" cy="7" r="1.2"/><circle cx="6" cy="7" r="1.2"/><circle cx="2" cy="12" r="1.2"/><circle cx="6" cy="12" r="1.2"/></svg></span>
      </li>`);
    row.append(
      toggle(module.title, entry.on, (on) => {
        entry.on = on;
        changed();
      }, module.detail),
    );
    list.append(row);
  }

  list.addEventListener('pointerdown', (event) => {
    const grip = (event.target as Element).closest('.grip');
    if (!grip) return;
    const row = grip.closest<HTMLElement>('.module-row')!;
    event.preventDefault();
    grip.setPointerCapture(event.pointerId);
    row.dataset.dragging = 'true';
    const move = (e: PointerEvent) => {
      const others = [...list.children].filter((child) => child !== row) as HTMLElement[];
      const before = others.find((other) => {
        const box = other.getBoundingClientRect();
        return e.clientY < box.top + box.height / 2;
      });
      if (before) list.insertBefore(row, before);
      else list.append(row);
    };
    const end = () => {
      grip.removeEventListener('pointermove', move as EventListener);
      delete row.dataset.dragging;
      const order = [...list.children].map((child) => (child as HTMLElement).dataset.id!);
      prefs.modules.sort((a, b) => order.indexOf(a.id) - order.indexOf(b.id));
      changed();
    };
    grip.addEventListener('pointermove', move as EventListener);
    grip.addEventListener('pointerup', end, { once: true });
    grip.addEventListener('pointercancel', end, { once: true });
  });
  return list;
}

export interface SettingsActions {
  /** Something changed; `layout` says whether the panel's content must be rebuilt. */
  changed(layout: boolean): void;
  skin(skin: Skin): void;
  edge(): void;
  autostart(enabled: boolean): Promise<boolean>;
  quit(): void;
}

export function buildSettings(
  settings: Settings,
  prefs: ViewPrefs,
  modules: ModuleDef[],
  autostart: boolean,
  actions: SettingsActions,
): HTMLElement {
  const root = element('<div class="settings"></div>');
  const relayout = () => actions.changed(true);
  const save = () => actions.changed(false);

  root.append(
    section('外观', skinPicker(settings.skin, (skin) => actions.skin(skin))),
    section(
      '呼出',
      segmented('屏幕边缘', [{ value: 'left', label: '左侧' }, { value: 'right', label: '右侧' }] as const, settings.edge, (edge) => {
        settings.edge = edge;
        actions.edge();
      }),
      segmented('面板位置', [{ value: 'pointer', label: '跟随指针' }, { value: 'center', label: '居中' }] as const, settings.anchor, (anchor) => {
        settings.anchor = anchor;
        save();
      }),
      segmented('推入力度', [{ value: 'light', label: '轻' }, { value: 'medium', label: '中' }, { value: 'firm', label: '重' }] as const, settings.sensitivity, (sensitivity) => {
        settings.sensitivity = sensitivity;
        save();
      }),
      segmented('离开后收起', [{ value: 200, label: '立即' }, { value: 500, label: '0.5 秒' }, { value: 1000, label: '1 秒' }], settings.close_delay_ms, (ms) => {
        settings.close_delay_ms = ms;
        save();
      }),
    ),
    section('显示内容', moduleList(prefs, modules, relayout)),
    section(
      '细节',
      toggle('CPU 线程', prefs.cpu.threads, (on) => ((prefs.cpu.threads = on), relayout())),
      toggle('CPU 频率', prefs.cpu.clock, (on) => ((prefs.cpu.clock = on), relayout())),
      toggle('显存', prefs.gpu.memory, (on) => ((prefs.gpu.memory = on), relayout())),
      toggle('GPU 温度、频率和风扇', prefs.gpu.sensors, (on) => ((prefs.gpu.sensors = on), relayout())),
      toggle('GPU 各引擎', prefs.gpu.engines, (on) => ((prefs.gpu.engines = on), relayout()), '3D、复制、视频编解码'),
      toggle('内存提交量和缓存', prefs.memory.details, (on) => ((prefs.memory.details = on), relayout())),
      toggle('网卡、地址和累计流量', prefs.network.details, (on) => ((prefs.network.details = on), relayout())),
      toggle('磁盘活动时间', prefs.disk.active, (on) => ((prefs.disk.active = on), relayout())),
      segmented('网速单位', [{ value: false, label: 'MB/s' }, { value: true, label: 'Mbps' }], prefs.network.bits, (bits) => {
        prefs.network.bits = bits;
        relayout();
      }),
      segmented('进程数量', [{ value: 3, label: '3' }, { value: 5, label: '5' }, { value: 8, label: '8' }], prefs.processes.count, (count) => {
        prefs.processes.count = count;
        relayout();
      }),
      segmented('进程排序', [{ value: 'cpu', label: 'CPU' }, { value: 'memory', label: '内存' }] as const, prefs.processes.sort, (sort) => {
        prefs.processes.sort = sort;
        relayout();
      }),
    ),
    section(
      '数据',
      segmented('刷新间隔', [{ value: 500, label: '0.5 秒' }, { value: 1000, label: '1 秒' }, { value: 2000, label: '2 秒' }], settings.interval_ms, (ms) => {
        settings.interval_ms = ms;
        relayout();
      }),
      segmented('曲线时长', [{ value: 30, label: '30 秒' }, { value: 60, label: '1 分' }, { value: 120, label: '2 分' }, { value: 300, label: '5 分' }], prefs.chartSeconds, (seconds) => {
        prefs.chartSeconds = seconds;
        relayout();
      }),
      segmented('负载警示', [{ value: 70, label: '70%' }, { value: 85, label: '85%' }, { value: 95, label: '95%' }], prefs.hotLoad, (load) => {
        prefs.hotLoad = load;
        relayout();
      }),
      segmented('温度警示', [{ value: 75, label: '75 °C' }, { value: 85, label: '85 °C' }, { value: 95, label: '95 °C' }], prefs.hotTemp, (temp) => {
        prefs.hotTemp = temp;
        relayout();
      }),
    ),
  );

  const startup = toggle('开机时启动', autostart, async (on) => {
    const now = await actions.autostart(on);
    startup.querySelector('.switch')!.setAttribute('aria-checked', String(now));
  });
  const quit = element<HTMLButtonElement>('<button type="button" class="quit">退出 Glance</button>');
  quit.addEventListener('click', () => actions.quit());
  root.append(section('系统', startup, quit));
  return root;
}
