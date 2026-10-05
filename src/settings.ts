import { element, escapeHtml } from './format';
import { type Key, t } from './i18n';
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
  theme: 'system',
  language: 'system',
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

const SKINS: { value: Skin; label: Key }[] = [
  { value: 'paper', label: 'skinPaper' },
  { value: 'glass', label: 'skinGlass' },
  { value: 'fluent', label: 'skinFluent' },
];

function skinPicker(current: Skin, pick: (skin: Skin) => void): HTMLElement {
  const root = element('<div class="skins" role="radiogroup"></div>');
  const cards = SKINS.map((skin) => {
    const card = element<HTMLButtonElement>(`
      <button type="button" class="skin-card" role="radio" data-preview="${skin.value}">
        <span class="skin-preview"><i></i><i></i><i></i></span>
        <span>${t(skin.label)}</span>
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
        <span class="grip" title="${t('dragToReorder')}"><svg viewBox="0 0 8 14" aria-hidden="true"><circle cx="2" cy="2" r="1.2"/><circle cx="6" cy="2" r="1.2"/><circle cx="2" cy="7" r="1.2"/><circle cx="6" cy="7" r="1.2"/><circle cx="2" cy="12" r="1.2"/><circle cx="6" cy="12" r="1.2"/></svg></span>
      </li>`);
    row.append(
      toggle(module.title, entry.on, (on) => {
        entry.on = on;
        changed();
      }, module.detail),
    );
    list.append(row);
  }

  // The held row follows the pointer; the others make way for it, each
  // gliding from where it was to where it now belongs; on release the held
  // row settles into its place.
  list.addEventListener('pointerdown', (event) => {
    const grip = (event.target as Element).closest('.grip');
    if (!grip) return;
    const row = grip.closest<HTMLElement>('.module-row')!;
    event.preventDefault();
    const startY = event.clientY;
    const startTop = row.offsetTop;
    row.dataset.dragging = 'true';
    const follow = (pointerY: number) => {
      row.style.transform = `translateY(${pointerY - startY - (row.offsetTop - startTop)}px)`;
    };
    const glide = (rows: HTMLElement[], before: Map<HTMLElement, number>) => {
      for (const other of rows) {
        const shift = before.get(other)! - other.offsetTop;
        if (shift === 0) continue;
        other.animate([{ transform: `translateY(${shift}px)` }, { transform: 'none' }], {
          duration: 200,
          easing: 'cubic-bezier(0.16, 1, 0.3, 1)',
        });
      }
    };
    // Followed on the window, not captured to the grip: moving the row in
    // the list detaches the grip for a moment, which ends a capture.
    const move = (e: PointerEvent) => {
      const others = [...list.children].filter((child) => child !== row) as HTMLElement[];
      // Where the held row's middle is now decides its place.
      const box = row.getBoundingClientRect();
      const middle = box.top + box.height / 2;
      const next = others.find((other) => {
        const r = other.getBoundingClientRect();
        return middle < r.top + r.height / 2;
      });
      if ((next ?? null) !== row.nextElementSibling) {
        const before = new Map(others.map((other) => [other, other.offsetTop]));
        if (next) list.insertBefore(row, next);
        else list.append(row);
        glide(others, before);
      }
      follow(e.clientY);
    };
    const end = () => {
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', end);
      window.removeEventListener('pointercancel', end);
      const settle = row.animate([{ transform: row.style.transform || 'none' }, { transform: 'none' }], {
        duration: 180,
        easing: 'cubic-bezier(0.16, 1, 0.3, 1)',
      });
      row.style.transform = '';
      settle.onfinish = () => delete row.dataset.dragging;
      const order = [...list.children].map((child) => (child as HTMLElement).dataset.id!);
      prefs.modules.sort((x, y) => order.indexOf(x.id) - order.indexOf(y.id));
      changed();
    };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', end);
    window.addEventListener('pointercancel', end);
  });
  return list;
}

export interface SettingsActions {
  /** Something changed that the panel shows. */
  changed(): void;
  /** The skin or the edge changed: the panel's window has to move or change. */
  moved(): void;
  /** The interface language changed: everything is drawn again. */
  relabel(): void;
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
  const changed = () => actions.changed();

  root.append(
    section(
      t('appearance'),
      skinPicker(settings.skin, (skin) => {
        settings.skin = skin;
        actions.moved();
      }),
      segmented(t('theme'), [
        { value: 'system', label: t('themeSystem') },
        { value: 'light', label: t('themeLight') },
        { value: 'dark', label: t('themeDark') },
        { value: 'backdrop', label: t('themeBackdrop') },
      ] as const, prefs.theme, (theme) => {
        prefs.theme = theme;
        changed();
      }),
      toggle(t('liveBackdrop'), settings.live_backdrop, (on) => {
        settings.live_backdrop = on;
        changed();
      }, t('liveBackdropHint')),
      segmented(t('language'), [
        { value: 'system', label: t('themeSystem') },
        { value: 'zh', label: '中文' },
        { value: 'en', label: 'English' },
      ] as const, prefs.language, (language) => {
        prefs.language = language;
        actions.relabel();
      }),
    ),
    section(
      t('opening'),
      segmented(t('edge'), [
        { value: 'left', label: t('left') },
        { value: 'top', label: t('top') },
        { value: 'right', label: t('right') },
      ] as const, settings.edge, (edge) => {
        settings.edge = edge;
        actions.moved();
      }),
      segmented(t('anchor'), [{ value: 'pointer', label: t('anchorPointer') }, { value: 'center', label: t('anchorCenter') }] as const, settings.anchor, (anchor) => {
        settings.anchor = anchor;
        changed();
      }),
      segmented(t('push'), [
        { value: 'light', label: t('pushLight') },
        { value: 'medium', label: t('pushMedium') },
        { value: 'firm', label: t('pushFirm') },
      ] as const, settings.sensitivity, (sensitivity) => {
        settings.sensitivity = sensitivity;
        changed();
      }),
      segmented(t('closeDelay'), [
        { value: 200, label: t('closeNow') },
        { value: 500, label: t('seconds', 0.5) },
        { value: 1000, label: t('seconds', 1) },
      ], settings.close_delay_ms, (ms) => {
        settings.close_delay_ms = ms;
        changed();
      }),
    ),
    section(t('shown'), moduleList(prefs, modules, changed)),
    section(
      t('details'),
      toggle(t('cpuThreads'), prefs.cpu.threads, (on) => ((prefs.cpu.threads = on), changed())),
      toggle(t('cpuClock'), prefs.cpu.clock, (on) => ((prefs.cpu.clock = on), changed())),
      toggle(t('gpuMemory'), prefs.gpu.memory, (on) => ((prefs.gpu.memory = on), changed())),
      toggle(t('gpuSensors'), prefs.gpu.sensors, (on) => ((prefs.gpu.sensors = on), changed())),
      toggle(t('gpuEngines'), prefs.gpu.engines, (on) => ((prefs.gpu.engines = on), changed()), t('gpuEnginesDetail')),
      toggle(t('memoryDetails'), prefs.memory.details, (on) => ((prefs.memory.details = on), changed())),
      toggle(t('networkDetails'), prefs.network.details, (on) => ((prefs.network.details = on), changed())),
      toggle(t('diskActive'), prefs.disk.active, (on) => ((prefs.disk.active = on), changed())),
      segmented(t('rateUnit'), [{ value: false, label: 'MB/s' }, { value: true, label: 'Mbps' }], prefs.network.bits, (bits) => {
        prefs.network.bits = bits;
        changed();
      }),
      segmented(t('processCountSetting'), [{ value: 5, label: '5' }, { value: 8, label: '8' }, { value: 12, label: '12' }], prefs.processes.count, (count) => {
        prefs.processes.count = count;
        changed();
      }),
      segmented(t('processSort'), [
        { value: 'cpu', label: 'CPU' },
        { value: 'memory', label: t('memory') },
        { value: 'io', label: t('io') },
        { value: 'gpu', label: 'GPU' },
      ] as const, prefs.processes.sort, (sort) => {
        prefs.processes.sort = sort;
        changed();
      }),
    ),
    section(
      t('data'),
      segmented(t('interval'), [
        { value: 500, label: t('seconds', 0.5) },
        { value: 1000, label: t('seconds', 1) },
        { value: 2000, label: t('seconds', 2) },
      ], settings.interval_ms, (ms) => {
        settings.interval_ms = ms;
        changed();
      }),
      segmented(t('span'), [
        { value: 30, label: t('seconds', 30) },
        { value: 60, label: t('minutesShort', 1) },
        { value: 120, label: t('minutesShort', 2) },
        { value: 300, label: t('minutesShort', 5) },
      ], prefs.chartSeconds, (seconds) => {
        prefs.chartSeconds = seconds;
        changed();
      }),
      segmented(t('loadAlert'), [{ value: 70, label: '70%' }, { value: 85, label: '85%' }, { value: 95, label: '95%' }], prefs.hotLoad, (load) => {
        prefs.hotLoad = load;
        changed();
      }),
      segmented(t('tempAlert'), [{ value: 75, label: '75 °C' }, { value: 85, label: '85 °C' }, { value: 95, label: '95 °C' }], prefs.hotTemp, (temp) => {
        prefs.hotTemp = temp;
        changed();
      }),
    ),
  );

  const startup = toggle(t('startup'), autostart, async (on) => {
    const now = await actions.autostart(on);
    startup.querySelector('.switch')!.setAttribute('aria-checked', String(now));
  });
  const quit = element(`
    <div class="field">
      <span class="field-label">${t('quit')}<small>${escapeHtml(t('quitDetail'))}</small></span>
      <button type="button" class="quit">${t('quitAction')}</button>
    </div>`);
  quit.querySelector('button')!.addEventListener('click', () => actions.quit());
  root.append(section(t('system'), startup, quit));
  return root;
}
