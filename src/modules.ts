import type { Plot } from './chart';
import * as fmt from './format';
import { element, escapeHtml } from './format';
import type { Sample, StaticInfo, ViewPrefs } from './types';

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
const ENGINE_NAMES: Record<string, string> = {
  '3D': '3D',
  Copy: '复制',
  VideoDecode: '视频解码',
  VideoEncode: '视频编码',
  VideoCodec: '视频编解码',
  Compute: '计算',
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
  rows: { label: string; value: (sample: Sample) => number }[];
  bits: boolean;
}): { root: HTMLElement; plot: Plot; update: (sample: Sample, scale: number) => void } {
  const root = element(`
    <section class="lane lane-rates lane-${options.kind}">
      ${head(options.title)}
      <dl class="rates">
        ${options.rows.map((row) => `<div><dt>${row.label}</dt><dd></dd></div>`).join('')}
      </dl>
      <div class="plot"></div>
    </section>`);
  const cells = [...root.querySelectorAll<HTMLElement>('.rates dd')];
  const aside = root.querySelector<HTMLElement>('.lane-aside')!;
  return {
    root,
    plot: { el: root.querySelector<HTMLElement>('.plot')!, series: options.rows.map((row) => row.value), max: 'auto' },
    update(sample, scale) {
      options.rows.forEach((row, i) => (cells[i].textContent = fmt.rate(row.value(sample), options.bits)));
      aside.textContent = `满刻度 ${fmt.rate(scale, options.bits)}`;
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
    build(prefs) {
      const lane = percentLane({ kind: 'cpu', title: 'CPU', device: info.cpu_name, value: (s) => s.cpu, hotLoad: prefs.hotLoad });
      let cells: HTMLElement[] = [];
      if (prefs.cpu.threads) {
        const grid = element(`<div class="threads" title="${info.threads} 个线程"></div>`);
        cells = Array.from({ length: info.threads }, () => document.createElement('i'));
        grid.append(...cells);
        lane.root.append(grid);
      }
      return {
        root: lane.root,
        plots: [lane.plot],
        update(sample) {
          setFigure(lane, sample.cpu, prefs.hotLoad);
          lane.aside.textContent = prefs.cpu.clock ? `${sample.ghz.toFixed(2)} GHz` : '';
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
      const memory = meterRow('显存');
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
              engineRows = shown.map(([kind]) => ({ kind, row: meterRow(ENGINE_NAMES[kind]) }));
              engines.replaceChildren(...engineRows.map((e) => e.row.root));
            }
            shown.forEach(([, load], i) => engineRows[i].row.set(load / 100, fmt.percent(load), load > prefs.hotLoad));
          }
          setFacts(
            prefs.gpu.sensors
              ? [
                  ['频率', reading.clock_mhz === null ? null : `${Math.round(reading.clock_mhz)} MHz`],
                  ['风扇', reading.fan_rpm === null ? null : `${reading.fan_rpm} RPM`],
                  ['共享显存', fmt.usage(reading.shared_used, gpu.shared_total)],
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
    title: '内存',
    build(prefs) {
      const lane = percentLane({
        kind: 'memory',
        title: '内存',
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
                  ['已提交', fmt.usage(sample.memory.committed, sample.memory.commit_limit)],
                  ['缓存', fmt.size(sample.memory.cached)],
                ]
              : [],
          );
        },
      };
    },
  };
}

function network(): ModuleDef {
  return {
    id: 'network',
    title: '网络',
    build(prefs) {
      const lane = rateLane({
        kind: 'network',
        title: '网络',
        bits: prefs.network.bits,
        rows: [
          { label: '下载', value: (s) => s.net_down },
          { label: '上传', value: (s) => s.net_up },
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
                  ['网卡', adapter ? adapter.name : '未连接'],
                  ['地址', adapter?.ipv4 ?? null],
                  ['链路', adapter && adapter.link_bps > 0 ? fmt.linkSpeed(adapter.link_bps) : null],
                  ['开机以来', `下载 ${fmt.bytes(sample.net_total_down)}，上传 ${fmt.bytes(sample.net_total_up)}`],
                ]
              : [],
          );
        },
      };
    },
  };
}

function disk(): ModuleDef {
  return {
    id: 'disk',
    title: '磁盘',
    build(prefs) {
      const lane = rateLane({
        kind: 'disk',
        title: '磁盘',
        bits: false,
        rows: [
          { label: '读取', value: (s) => s.disk_read },
          { label: '写入', value: (s) => s.disk_write },
        ],
      });
      const setFacts = facts(lane.root);
      return {
        root: lane.root,
        plots: [lane.plot],
        update(sample, scale) {
          lane.update(sample, scale(lane.plot));
          setFacts(prefs.disk.active ? [['活动时间', fmt.percent(sample.disk_active)]] : []);
        },
      };
    },
  };
}

function processes(): ModuleDef {
  return {
    id: 'processes',
    title: '进程',
    build(prefs) {
      const byMemory = prefs.processes.sort === 'memory';
      const root = element(`
        <section class="lane lane-list lane-processes">
          <header class="lane-head">
            <h2>进程</h2>
            <span class="lane-device">按${byMemory ? '内存' : ' CPU '}排序</span>
            <span class="lane-aside">CPU</span>
            <span class="lane-aside">内存</span>
          </header>
          <ol class="rows"></ol>
        </section>`);
      const list = root.querySelector('ol')!;
      return {
        root,
        plots: [],
        update(sample) {
          const ranked = (byMemory ? sample.by_memory : sample.by_cpu).slice(0, prefs.processes.count);
          list.replaceChildren(
            ...ranked.map((process) =>
              element(`
                <li>
                  <span class="row-name">${escapeHtml(process.name)}</span>
                  <span class="row-value">${fmt.percent(process.cpu)}</span>
                  <span class="row-value">${fmt.size(process.mem)}</span>
                </li>`),
            ),
          );
        },
      };
    },
  };
}

function storage(): ModuleDef {
  return {
    id: 'storage',
    title: '存储',
    detail: '各分区的空间',
    build(prefs) {
      const root = element(`
        <section class="lane lane-list lane-storage">
          <header class="lane-head"><h2>存储</h2></header>
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

/** Charge at or below which a battery on its own is shown in the signal colour. */
const LOW_BATTERY = 20;

function battery(): ModuleDef {
  return {
    id: 'battery',
    title: '电池',
    detail: '笔记本电脑',
    build() {
      const root = element(`
        <section class="lane lane-battery">
          ${head('电池')}
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
          aside.textContent = charging ? '正在充电' : seconds_left === null ? '使用电池' : `剩余 ${fmt.duration(seconds_left)}`;
        },
      };
    },
  };
}

function system(): ModuleDef {
  return {
    id: 'system',
    title: '系统',
    detail: '开机时长、进程和句柄数',
    build() {
      const root = element(`<section class="lane lane-system">${head('系统')}</section>`);
      const setFacts = facts(root);
      return {
        root,
        plots: [],
        update(sample) {
          setFacts([
            ['开机时长', fmt.duration(sample.system.uptime_s)],
            ['进程', String(sample.system.processes)],
            ['线程', String(sample.system.threads)],
            ['句柄', String(sample.system.handles)],
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
    network(),
    disk(),
    processes(),
    storage(),
    battery(),
    system(),
  ];
}

/** Modules shown when nothing has been chosen yet. */
export const DEFAULT_OFF = new Set(['system']);
