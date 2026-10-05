import type { Plot } from './chart';
import { type Key, t } from './i18n';
import * as fmt from './format';
import { element, escapeHtml } from './format';
import type { ProcessSample, Sample, StaticInfo, ViewPrefs } from './types';

/** One section of the panel: its readings and the plots it draws. */
export interface Lane {
  root: HTMLElement;
  plots: Plot[];
  /** `scale` gives the full-scale value a plot is currently drawn at. */
  update(sample: Sample, scale: (plot: Plot) => number): void;
}

export interface ModuleDef {
  id: string;
  title: string;
  /** Shown next to the title in the settings list. */
  detail?: string;
  build(prefs: ViewPrefs): Lane;
}

/** The engine kinds Task Manager shows by default, which are the ones a
 * workload is recognisably using; drivers report many more (timers, security,
 * high-priority queues) that say nothing to a reader. */
const ENGINE_NAMES: Record<string, Key> = {
  '3D': 'engine3D',
  Copy: 'engineCopy',
  VideoDecode: 'engineVideoDecode',
  VideoEncode: 'engineVideoEncode',
  VideoCodec: 'engineVideoCodec',
  Compute: 'engineCompute',
};

function head(title: string, device = ''): string {
  return `
    <header class="lane-head">
      <h2>${title}</h2>
      <span class="lane-device">${escapeHtml(device)}</span>
      <span class="lane-aside"></span>
    </header>`;
}

/** Label and value pairs under a lane; a pair whose value is null is left out. */
function facts(root: HTMLElement): (pairs: [string, string | null][]) => void {
  const list = element('<dl class="facts"></dl>');
  root.append(list);
  let shown = '';
  return (pairs) => {
    const present = pairs.filter((pair): pair is [string, string] => pair[1] !== null);
    const key = present.map(([label]) => label).join('|');
    if (key !== shown) {
      shown = key;
      list.replaceChildren(...present.map(([label]) => element(`<div><dt>${label}</dt><dd></dd></div>`)));
    }
    const cells = list.querySelectorAll('dd');
    present.forEach(([, value], i) => (cells[i].textContent = value));
    list.hidden = present.length === 0;
  };
}

function percentLane(options: {
  kind: string;
  title: string;
  device?: string;
  value: (sample: Sample) => number;
  hotLoad: number;
}): { root: HTMLElement; plot: Plot; figure: HTMLElement; aside: HTMLElement } {
  const root = element(`
    <section class="lane lane-${options.kind}">
      ${head(options.title, options.device)}
      <div class="readout"><span class="figure"></span><span class="unit">%</span></div>
      <div class="plot"></div>
    </section>`);
  return {
    root,
    plot: {
      el: root.querySelector<HTMLElement>('.plot')!,
      series: [options.value],
      max: 100,
      hotAbove: options.hotLoad,
    },
    figure: root.querySelector<HTMLElement>('.figure')!,
    aside: root.querySelector<HTMLElement>('.lane-aside')!,
  };
}

function setFigure(lane: { root: HTMLElement; figure: HTMLElement }, value: number, hot: number) {
  lane.figure.textContent = String(Math.round(value));
  lane.root.dataset.hot = String(value > hot);
}

function rateLane(options: {
  kind: string;
  title: string;
  device?: string;
  rows: { label: string; value: (sample: Sample) => number }[];
  bits: boolean;
}): { root: HTMLElement; plot: Plot; aside: HTMLElement; update: (sample: Sample, scale: number) => void } {
  const root = element(`
    <section class="lane lane-rates lane-${options.kind}">
      ${head(options.title, options.device)}
      <dl class="rates">
        ${options.rows.map((row) => `<div><dt>${row.label}</dt><dd></dd></div>`).join('')}
      </dl>
      <div class="plot"></div>
    </section>`);
  const cells = [...root.querySelectorAll<HTMLElement>('.rates dd')];
  const aside = root.querySelector<HTMLElement>('.lane-aside')!;
  return {
    root,
    aside,
    plot: { el: root.querySelector<HTMLElement>('.plot')!, series: options.rows.map((row) => row.value), max: 'auto' },
    update(sample, scale) {
      options.rows.forEach((row, i) => (cells[i].textContent = fmt.rate(row.value(sample), options.bits)));
      aside.textContent = t('fullScale', fmt.rate(scale, options.bits));
    },
  };
}

function meterRow(label: string): { root: HTMLElement; set: (fraction: number, text: string, hot: boolean) => void } {
  const root = element(`<div class="meter-row"><span class="meter-label">${label}</span><div class="meter"></div><span class="meter-value"></span></div>`);
  const meter = root.querySelector<HTMLElement>('.meter')!;
  const value = root.querySelector<HTMLElement>('.meter-value')!;
  return {
    root,
    set(fraction, text, hot) {
      meter.style.setProperty('--load', String(Math.min(Math.max(fraction, 0), 1)));
      meter.dataset.hot = String(hot);
      value.textContent = text;
    },
  };
}

function cpu(info: StaticInfo): ModuleDef {
  return {
    id: 'cpu',
    title: 'CPU',
    detail: info.cpu_name,
    build(prefs) {
      const lane = percentLane({ kind: 'cpu', title: 'CPU', device: info.cpu_name, value: (s) => s.cpu, hotLoad: prefs.hotLoad });
      const setFacts = facts(lane.root);
      let cells: HTMLElement[] = [];
      if (prefs.cpu.threads) {
        const grid = element(`<div class="threads" title="${t('threadsTitle', info.threads)}"></div>`);
        cells = Array.from({ length: info.threads }, () => document.createElement('i'));
        grid.append(...cells);
        lane.root.append(grid);
      }
      return {
        root: lane.root,
        plots: [lane.plot],
        update(sample) {
          setFigure(lane, sample.cpu, prefs.hotLoad);
          // The temperature takes the corner, as on the GPU lanes; the clock,
          // the power and each chiplet's temperature go below.
          const sensors = sample.cpu_sensors;
          const temp = sensors?.temp ?? null;
          const clock = prefs.cpu.clock ? `${sample.ghz.toFixed(2)} GHz` : null;
          lane.aside.textContent = temp !== null ? `${Math.round(temp)} °C` : (clock ?? '');
          lane.aside.dataset.hot = String(temp !== null && temp > prefs.hotTemp);
          const ccds = sensors && sensors.ccds.length > 1 ? sensors.ccds : [];
          setFacts([
            [t('clock'), temp !== null ? clock : null],
            [t('power'), sensors?.power != null ? `${sensors.power.toFixed(1)} W` : null],
            ...ccds.map((celsius, i): [string, string] => [t('ccd', i + 1), `${Math.round(celsius)} °C`]),
          ]);
          sample.threads.forEach((load, i) => {
            if (!cells[i]) return;
            cells[i].style.setProperty('--load', String(load / 100));
            cells[i].dataset.hot = String(load > prefs.hotLoad);
          });
        },
      };
    },
  };
}

function gpu(info: StaticInfo, index: number): ModuleDef {
  const gpu = info.gpus[index];
  return {
    id: `gpu:${index}`,
    title: 'GPU',
    detail: gpu.name,
    build(prefs) {
      const lane = percentLane({ kind: 'gpu', title: 'GPU', device: gpu.name, value: (s) => s.gpus[index].usage, hotLoad: prefs.hotLoad });
      const memory = meterRow(t('vram'));
      if (prefs.gpu.memory) lane.root.append(memory.root);
      const engines = element('<div class="engines"></div>');
      if (prefs.gpu.engines) lane.root.append(engines);
      const setFacts = facts(lane.root);
      let engineRows: { kind: string; row: ReturnType<typeof meterRow> }[] = [];
      return {
        root: lane.root,
        plots: [lane.plot],
        update(sample) {
          const reading = sample.gpus[index];
          setFigure(lane, reading.usage, prefs.hotLoad);
          const hotTemp = reading.temp !== null && reading.temp > prefs.hotTemp;
          lane.aside.textContent = prefs.gpu.sensors && reading.temp !== null ? `${Math.round(reading.temp)} °C` : '';
          lane.aside.dataset.hot = String(hotTemp);
          memory.set(reading.mem_used / gpu.mem_total, fmt.usage(reading.mem_used, gpu.mem_total), false);
          if (prefs.gpu.engines) {
            const shown = reading.engines.filter(([kind]) => kind in ENGINE_NAMES);
            if (engineRows.map((e) => e.kind).join() !== shown.map(([kind]) => kind).join()) {
              engineRows = shown.map(([kind]) => ({ kind, row: meterRow(t(ENGINE_NAMES[kind])) }));
              engines.replaceChildren(...engineRows.map((e) => e.row.root));
            }
            shown.forEach(([, load], i) => engineRows[i].row.set(load / 100, fmt.percent(load), load > prefs.hotLoad));
          }
          setFacts(
            prefs.gpu.sensors
              ? [
                  [t('clock'), reading.clock_mhz === null ? null : `${Math.round(reading.clock_mhz)} MHz`],
                  [t('fan'), reading.fan_rpm === null ? null : `${reading.fan_rpm} RPM`],
                  [t('sharedVram'), fmt.usage(reading.shared_used, gpu.shared_total)],
                ]
              : [],
          );
        },
      };
    },
  };
}

function memory(info: StaticInfo): ModuleDef {
  return {
    id: 'memory',
    title: t('memory'),
    detail: info.memory_modules ?? undefined,
    build(prefs) {
      const lane = percentLane({
        kind: 'memory',
        title: t('memory'),
        value: (s) => (s.memory.used / info.mem_total) * 100,
        hotLoad: prefs.hotLoad,
      });
      const setFacts = facts(lane.root);
      return {
        root: lane.root,
        plots: [lane.plot],
        update(sample) {
          setFigure(lane, (sample.memory.used / info.mem_total) * 100, prefs.hotLoad);
          lane.aside.textContent = fmt.usage(sample.memory.used, info.mem_total);
          setFacts(
            prefs.memory.details
              ? [
                  [t('committed'), fmt.usage(sample.memory.committed, sample.memory.commit_limit)],
                  [t('cached'), fmt.size(sample.memory.cached)],
                ]
              : [],
          );
        },
      };
    },
  };
}

function network(info: StaticInfo): ModuleDef {
  return {
    id: 'network',
    title: t('network'),
    detail: info.network_adapter ?? t('networkDetail'),
    build(prefs) {
      const lane = rateLane({
        kind: 'network',
        title: t('network'),
        bits: prefs.network.bits,
        rows: [
          { label: t('download'), value: (s) => s.net_down },
          { label: t('upload'), value: (s) => s.net_up },
        ],
      });
      const setFacts = facts(lane.root);
      return {
        root: lane.root,
        plots: [lane.plot],
        update(sample, scale) {
          lane.update(sample, scale(lane.plot));
          const adapter = sample.network;
          setFacts(
            prefs.network.details
              ? [
                  [t('adapter'), adapter ? adapter.name : t('notConnected')],
                  [t('address'), adapter?.ipv4 ?? null],
                  [t('link'), adapter && adapter.link_bps > 0 ? fmt.linkSpeed(adapter.link_bps) : null],
                  [t('sinceBoot'), t('sinceBootValue', fmt.bytes(sample.net_total_down), fmt.bytes(sample.net_total_up))],
                ]
              : [],
          );
        },
      };
    },
  };
}

function disk(info: StaticInfo): ModuleDef {
  return {
    id: 'disk',
    title: t('disk'),
    detail: info.drives.join(', ') || undefined,
    build(prefs) {
      const lane = rateLane({
        kind: 'disk',
        title: t('disk'),
        device: info.drives.join(', '),
        bits: false,
        rows: [
          { label: t('read'), value: (s) => s.disk_read },
          { label: t('write'), value: (s) => s.disk_write },
        ],
      });
      const setFacts = facts(lane.root);
      return {
        root: lane.root,
        plots: [lane.plot],
        update(sample, scale) {
          lane.update(sample, scale(lane.plot));
          // A drive's temperature takes the corner, as a GPU's does, and the
          // graph's scale moves below. With several drives, each is listed.
          const temps = sample.drive_temps;
          const hottest = temps.reduce<number | null>((max, drive) => Math.max(max ?? drive.celsius, drive.celsius), null);
          if (hottest !== null) {
            lane.aside.textContent = `${Math.round(hottest)} °C`;
            lane.aside.dataset.hot = String(hottest > prefs.hotTemp);
          }
          setFacts([
            ...(temps.length > 1
              ? temps.map((drive): [string, string] => [drive.name, `${Math.round(drive.celsius)} °C`])
              : []),
            [t('activeTime'), prefs.disk.active ? fmt.percent(sample.disk_active) : null],
            [t('scale'), hottest !== null ? fmt.rate(scale(lane.plot), false) : null],
          ]);
        },
      };
    },
  };
}

type ProcessSort = ViewPrefs['processes']['sort'];

/** The process table's columns: what each shows, and how it sorts. */
const PROCESS_COLUMNS: { sort: ProcessSort; label: () => string; value: (p: ProcessSample) => number; text: (p: ProcessSample) => string }[] = [
  { sort: 'cpu', label: () => 'CPU', value: (p) => p.cpu, text: (p) => fmt.percent(p.cpu) },
  { sort: 'memory', label: () => t('memory'), value: (p) => p.mem, text: (p) => fmt.size(p.mem) },
  { sort: 'io', label: () => t('io'), value: (p) => p.io, text: (p) => fmt.rate(p.io, false) },
  { sort: 'gpu', label: () => 'GPU', value: (p) => p.gpu, text: (p) => fmt.percent(p.gpu) },
];

/**
 * The busiest programs, as a table. A column heading sorts by it; the list
 * shows as many rows as chosen and scrolls through the rest.
 */
function processes(): ModuleDef {
  return {
    id: 'processes',
    title: t('processes'),
    detail: t('processesDetail'),
    build(prefs) {
      const root = element(`
        <section class="lane lane-list lane-processes" style="--visible-rows: ${prefs.processes.count}">
          <header class="lane-head">
            <h2>${t('processes')}</h2>
            ${PROCESS_COLUMNS.map((c) => `<button type="button" class="sort" data-sort="${c.sort}">${c.label()}</button>`).join('')}
          </header>
          <ol class="rows"></ol>
        </section>`);
      const list = root.querySelector('ol')!;
      const headings = [...root.querySelectorAll<HTMLButtonElement>('.sort')];
      let latest: ProcessSample[] = [];
      const draw = () => {
        const column = PROCESS_COLUMNS.find((c) => c.sort === prefs.processes.sort)!;
        headings.forEach((h) => h.setAttribute('aria-pressed', String(h.dataset.sort === column.sort)));
        const ranked = [...latest].sort((x, y) => column.value(y) - column.value(x));
        // Rows are rewritten in place: emptying the list, even for a moment,
        // would throw its scroll position back to the top.
        while (list.children.length < ranked.length) {
          list.append(element(`<li><span class="row-name"></span>${'<span class="row-value"></span>'.repeat(PROCESS_COLUMNS.length)}</li>`));
        }
        while (list.children.length > ranked.length) list.lastElementChild!.remove();
        ranked.forEach((process, i) => {
          const cells = list.children[i].children;
          cells[0].textContent = process.name;
          PROCESS_COLUMNS.forEach((c, j) => (cells[j + 1].textContent = c.text(process)));
        });
      };
      for (const heading of headings) {
        heading.addEventListener('click', () => {
          prefs.processes.sort = heading.dataset.sort as ProcessSort;
          list.scrollTop = 0;
          draw();
          // Kept for next time, as if chosen in the settings.
          window.dispatchEvent(new CustomEvent('prefs-changed'));
        });
      }
      return {
        root,
        plots: [],
        update(sample) {
          latest = sample.processes;
          draw();
        },
      };
    },
  };
}

function storage(): ModuleDef {
  return {
    id: 'storage',
    title: t('storage'),
    detail: t('storageDetail'),
    build(prefs) {
      const root = element(`
        <section class="lane lane-list lane-storage">
          <header class="lane-head"><h2>${t('storage')}</h2></header>
          <div class="volumes"></div>
        </section>`);
      const list = root.querySelector<HTMLElement>('.volumes')!;
      let rows: ReturnType<typeof meterRow>[] = [];
      let names = '';
      return {
        root,
        plots: [],
        update(sample) {
          const key = sample.volumes.map((v) => v.name).join();
          if (key !== names) {
            names = key;
            rows = sample.volumes.map((volume) => meterRow(volume.name));
            list.replaceChildren(...rows.map((row) => row.root));
          }
          sample.volumes.forEach((volume, i) => {
            const full = volume.used / volume.total;
            rows[i].set(full, fmt.usage(volume.used, volume.total), full * 100 > prefs.hotLoad);
          });
        },
      };
    },
  };
}

/** A board input's name: what it is wired to, or its number. */
function inputName(name: string, numbered: Key): string {
  return /^\d+$/.test(name) ? t(numbered, name) : t(name as Key);
}

function board(info: StaticInfo): ModuleDef {
  return {
    id: 'board',
    title: t('board'),
    detail: info.board || undefined,
    build(prefs) {
      const root = element(`
        <section class="lane lane-list lane-board">
          ${head(t('board'), info.board)}
          <dl class="facts board-temps"></dl>
          <dl class="facts board-fans"></dl>
        </section>`);
      const temps = root.querySelector<HTMLElement>('.board-temps')!;
      const fans = root.querySelector<HTMLElement>('.board-fans')!;
      const fill = (list: HTMLElement, rows: [string, string, boolean][]) => {
        list.hidden = rows.length === 0;
        list.replaceChildren(
          ...rows.map(([label, value, hot]) =>
            element(`<div data-hot="${hot}"><dt>${escapeHtml(label)}</dt><dd>${value}</dd></div>`),
          ),
        );
      };
      return {
        root,
        plots: [],
        update(sample) {
          // Without the driver there is nothing to show.
          root.hidden = sample.board === null;
          if (!sample.board) return;
          fill(temps, sample.board.temps.map(([name, celsius]) => [inputName(name, 'sensor'), `${Math.round(celsius)} °C`, celsius > prefs.hotTemp]));
          fill(fans, sample.board.fans.map(([name, rpm]) => [inputName(name, 'fanNumbered'), `${Math.round(rpm)} RPM`, false]));
        },
      };
    },
  };
}

/** Charge at or below which a battery on its own is shown in the signal colour. */
const LOW_BATTERY = 20;

function battery(): ModuleDef {
  return {
    id: 'battery',
    title: t('battery'),
    detail: t('batteryDetail'),
    build() {
      const root = element(`
        <section class="lane lane-battery">
          ${head(t('battery'))}
          <div class="readout"><span class="figure"></span><span class="unit">%</span></div>
        </section>`);
      const figure = root.querySelector<HTMLElement>('.figure')!;
      const aside = root.querySelector<HTMLElement>('.lane-aside')!;
      return {
        root,
        plots: [],
        update(sample) {
          root.hidden = sample.battery === null;
          if (!sample.battery) return;
          const { percent, charging, seconds_left } = sample.battery;
          figure.textContent = String(percent);
          root.dataset.hot = String(!charging && percent <= LOW_BATTERY);
          aside.textContent = charging ? t('charging') : seconds_left === null ? t('onBattery') : t('remaining', fmt.duration(seconds_left));
        },
      };
    },
  };
}

function system(): ModuleDef {
  return {
    id: 'system',
    title: t('system'),
    detail: t('systemDetail'),
    build() {
      const root = element(`<section class="lane lane-system">${head(t('system'))}</section>`);
      const setFacts = facts(root);
      return {
        root,
        plots: [],
        update(sample) {
          setFacts([
            [t('uptimeFact'), fmt.duration(sample.system.uptime_s)],
            [t('processCount'), String(sample.system.processes)],
            [t('threadCount'), String(sample.system.threads)],
            [t('handleCount'), String(sample.system.handles)],
          ]);
        },
      };
    },
  };
}

/** Every module this machine can show, in the default order. */
export function catalog(info: StaticInfo): ModuleDef[] {
  return [
    cpu(info),
    ...info.gpus.map((_, index) => gpu(info, index)),
    memory(info),
    network(info),
    disk(info),
    processes(),
    storage(),
    board(info),
    battery(),
    system(),
  ];
}

/** Modules shown when nothing has been chosen yet. */
export const DEFAULT_OFF = new Set(['system']);
