// The panel itself: lanes, chart, bar, columns and glass, inside a host
// element that carries the skin. The panel window uses its body as the host;
// the settings window uses a stage in its preview.

import { type Plot, Recorder, offsetWithin } from './chart';
import * as fmt from './format';
import { element } from './format';
import { t } from './i18n';
import type { Backdrop } from './glass';
import type { Lane, ModuleDef } from './modules';
import type { Sample, Settings, ViewPrefs } from './types';

/** Closest the panel gets to the top and bottom of its host (px); the backend keeps the same. */
export const GAP = 12;
/** Columns a panel may spread over before the window has to zoom out instead; the backend keeps the same. */
const MAX_COLUMNS = 3;

/** What the column rule works from, measured with the lanes in one column (px). */
export interface Measures {
  laneHeights: number[];
  laneGap: number;
  chromeHeight: number;
  columnWidth: number;
  columnGap: number;
}

/** The Settings glyph of the system icon font. */
const GEAR = '<span class="glyph" aria-hidden="true"></span>';

/** What a panel may take up, in px: its height and how many columns. The backend works it out by the same rule. */
export interface Room {
  height: number;
  max_columns: number;
}

/** Share of the height a panel opened from the top may take; the backend keeps the same. */
const TOP_SHARE = 0.6;

/** The room on a screen `width` by `height` px, for a panel opened from `edge`. */
export function roomFor(edge: string, width: number, height: number, measures: Measures): Room {
  if (edge !== 'top') return { height, max_columns: MAX_COLUMNS };
  const fit = Math.floor((width - 2 * GAP + measures.columnGap) / (measures.columnWidth + measures.columnGap));
  return { height: height * TOP_SHARE, max_columns: Math.max(fit, 1) };
}

export interface Box {
  left: number;
  top: number;
  width: number;
  height: number;
}

export class PanelView {
  readonly panel: HTMLElement;
  samples: Sample[] = [];
  private lanesHost: HTMLElement;
  private canvas: HTMLCanvasElement;
  private barText: HTMLElement;
  private lanes: Lane[] = [];
  private recorder!: Recorder;
  measures: Measures = { laneHeights: [], laneGap: 0, chromeHeight: 0, columnWidth: 0, columnGap: 0 };

  constructor(
    private host: HTMLElement,
    /** Rebuilt when the interface language changes: titles are in it. */
    public modules: ModuleDef[],
    private backdrop: Backdrop,
    onGear: () => void,
  ) {
    this.panel = element(`
      <main class="panel">
        <div class="acrylic"></div>
        <canvas class="chart"></canvas>
        <div class="lanes"></div>
        <footer class="bar">
          <span class="bar-text"></span>
          <button type="button" class="bar-button" aria-label="${t('settings')}">${GEAR}</button>
        </footer>
      </main>`);
    this.lanesHost = this.panel.querySelector('.lanes')!;
    this.canvas = this.panel.querySelector('canvas')!;
    this.barText = this.panel.querySelector('.bar-text')!;
    this.panel.querySelector('.bar-button')!.addEventListener('click', onGear);
    host.append(this.panel);
  }

  /** Rebuilds the lanes for these settings, and applies skin and edge. */
  build(settings: Settings, prefs: ViewPrefs) {
    this.host.dataset.skin = settings.skin;
    this.host.dataset.edge = settings.edge;
    const byId = new Map(this.modules.map((module) => [module.id, module]));
    this.lanes = prefs.modules.filter((entry) => entry.on).map((entry) => byId.get(entry.id)!.build(prefs));
    this.deal([this.lanes.map((lane) => lane.root)]);
    const plots = this.lanes.flatMap((lane) => lane.plots);
    this.recorder = new Recorder(this.canvas, this.panel, plots, settings.interval_ms + 100, prefs.chartSeconds * 1000);
    this.showLatest();
  }

  push(sample: Sample, keep: number) {
    this.samples.push(sample);
    while (this.samples.length > keep) this.samples.shift();
    this.showLatest();
  }

  showLatest() {
    const latest = this.samples[this.samples.length - 1];
    if (!latest) return;
    const scale = (plot: Plot) => this.recorder.scale(plot, this.samples);
    for (const lane of this.lanes) lane.update(latest, scale);
    this.barText.textContent = t('uptime', fmt.duration(latest.system.uptime_s));
  }

  /**
   * Spreads the lanes over as many columns as they need to show at full size
   * in the room (worked out from the measures, which this also takes), then
   * centres the panel on the focus along its edge, as far as the host allows.
   */
  layout(room: (measures: Measures) => Room, focus: { x: number; y: number }) {
    // Lanes are measured where they are, at their own height (not stretched
    // to their column's): moving them would reset what is scrolled in them.
    this.panel.classList.add('measuring');
    const style = getComputedStyle(this.panel);
    const laneGap = parseFloat(style.getPropertyValue('--lane-gap')) || 0;
    this.measures = {
      laneHeights: this.lanes.map((lane) => lane.root.offsetHeight),
      laneGap,
      chromeHeight: this.panel.offsetHeight - this.lanesHost.offsetHeight,
      columnWidth: parseFloat(style.getPropertyValue('--column-width')),
      columnGap: parseFloat(style.getPropertyValue('--column-gap')) || 0,
    };
    this.panel.classList.remove('measuring');
    const { height: roomHeight, max_columns } = room(this.measures);
    const space = roomHeight - 2 * GAP - this.measures.chromeHeight;
    // The fewest columns whose tallest fits; failing that, the most.
    const heights = this.measures.laneHeights;
    const most = Math.max(Math.min(max_columns, heights.length), 1);
    let columns = 1;
    while (columns < most && balancedCuts(heights, columns, laneGap).tallest > space) columns++;
    this.panel.style.setProperty('--columns', String(columns));
    const { cuts } = balancedCuts(heights, columns, laneGap);
    this.deal(cuts.map((start, i) => this.lanes.slice(start, cuts[i + 1]).map((lane) => lane.root)));

    const along = (focus: number, size: number, extent: number) =>
      Math.max(Math.min(focus - size / 2, extent - GAP - size), GAP);
    if (this.host.dataset.edge === 'top') {
      this.panel.style.top = '';
      this.panel.style.left = `${along(focus.x, this.panel.offsetWidth, this.host.clientWidth)}px`;
    } else {
      this.panel.style.left = '';
      this.panel.style.top = `${along(focus.y, this.panel.offsetHeight, this.host.clientHeight)}px`;
    }
    this.recorder.layout();
  }

  /** Puts each group of lanes in a column of its own, in order. */
  private deal(groups: HTMLElement[][]) {
    const current = [...this.lanesHost.children].map((column) => [...column.children]);
    const same =
      current.length === groups.length &&
      current.every((column, i) => column.length === groups[i].length && column.every((lane, j) => lane === groups[i][j]));
    if (same) return;
    this.lanesHost.replaceChildren(
      ...groups.map((group) => {
        const column = document.createElement('div');
        column.className = 'column';
        column.append(...group);
        return column;
      }),
    );
  }

  /**
   * Lays the captured desktop under the panel for the active skin. With the
   * theme following the backdrop, the whole panel turns light or dark by how
   * bright the desktop behind it is.
   */
  dress(theme: ViewPrefs['theme']) {
    if (theme === 'backdrop' && this.backdrop.present) {
      this.host.dataset.theme = this.backdrop.isLight(this.rect(), this.host) ? 'light' : 'dark';
      this.recorder.layout();
    }
    this.backdrop.dress(this.panel, this.host.dataset.skin!, this.host);
  }

  /** A newer capture of the same desktop: the glass takes it as it is. */
  retexture() {
    this.backdrop.retexture(this.panel, this.host.dataset.skin!, this.host);
  }

  draw(now: number) {
    this.recorder.draw(this.samples, now);
  }

  /** Recolours the plots, after a theme change. */
  recolor() {
    this.recorder.layout();
  }

  /** Where the panel sits in its host, ignoring the entrance transform. */
  rect(): Box {
    const { left, top } = offsetWithin(this.panel, this.host);
    return { left, top, width: this.panel.offsetWidth, height: this.panel.offsetHeight };
  }
}

/**
 * Where each column starts, and the tallest column's height, when `heights`
 * are split in order into `columns` columns so the tallest is as short as it
 * can be. Lanes of height zero are hidden and take no gap. The backend splits
 * by the same rule.
 */
export function balancedCuts(heights: number[], columns: number, gap: number): { cuts: number[]; tallest: number } {
  const n = heights.length;
  const span = (from: number, to: number) => {
    const shown = heights.slice(from, to).filter((h) => h > 0);
    return shown.reduce((sum, h) => sum + h, 0) + Math.max(shown.length - 1, 0) * gap;
  };
  const best = Array.from({ length: columns + 1 }, () => new Array<number>(n + 1).fill(Infinity));
  const from = Array.from({ length: columns + 1 }, () => new Array<number>(n + 1).fill(0));
  best[0][0] = 0;
  for (let k = 1; k <= columns; k++) {
    for (let i = k; i <= n; i++) {
      for (let j = k - 1; j < i; j++) {
        const candidate = Math.max(best[k - 1][j], span(j, i));
        if (candidate < best[k][i]) {
          best[k][i] = candidate;
          from[k][i] = j;
        }
      }
    }
  }
  const cuts: number[] = [];
  for (let k = columns, i = n; k > 0; i = from[k][i], k--) cuts.unshift(from[k][i]);
  return { cuts, tallest: best[columns][n] };
}
