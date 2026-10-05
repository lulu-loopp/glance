// The panel itself: lanes, chart, bar, columns and glass, inside a host
// element that carries the skin. The panel window uses its body as the host;
// the settings window uses a stage in its preview.

import { type Plot, Recorder, offsetWithin } from './chart';
import * as fmt from './format';
import { element } from './format';
import type { Backdrop } from './glass';
import type { Lane, ModuleDef } from './modules';
import type { Sample, Settings, ViewPrefs } from './types';

/** Closest the panel gets to the top and bottom of its host (px); the backend keeps the same. */
export const GAP = 12;
/** Columns a panel may spread over before the window has to zoom out instead; the backend keeps the same. */
const MAX_COLUMNS = 3;

/** What the column rule works from, measured with the lanes in one column (px). */
export interface Measures {
  lanesHeight: number;
  chromeHeight: number;
  columnWidth: number;
  columnGap: number;
}

/** The Settings glyph of the system icon font. */
const GEAR = '<span class="glyph" aria-hidden="true"></span>';

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
  measures: Measures = { lanesHeight: 0, chromeHeight: 0, columnWidth: 0, columnGap: 0 };

  constructor(
    private host: HTMLElement,
    private modules: ModuleDef[],
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
          <button type="button" class="bar-button" aria-label="设置">${GEAR}</button>
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
    this.lanesHost.replaceChildren(...this.lanes.map((lane) => lane.root));
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
    this.barText.textContent = `已开机 ${fmt.duration(latest.system.uptime_s)}`;
  }

  /**
   * Spreads the lanes over as many columns as `room` (px of height) needs to
   * show them at full size, then centres the panel on `focus` as far as the
   * host allows.
   */
  layout(room: number, focus: number) {
    this.panel.style.setProperty('--columns', '1');
    const style = getComputedStyle(this.panel);
    this.measures = {
      lanesHeight: this.lanesHost.offsetHeight,
      chromeHeight: this.panel.offsetHeight - this.lanesHost.offsetHeight,
      columnWidth: parseFloat(style.getPropertyValue('--column-width')),
      columnGap: parseFloat(style.getPropertyValue('--column-gap')) || 0,
    };
    const space = room - 2 * GAP - this.measures.chromeHeight;
    const columns = Math.min(Math.max(Math.ceil(this.measures.lanesHeight / space), 1), MAX_COLUMNS);
    this.panel.style.setProperty('--columns', String(columns));

    const height = this.panel.offsetHeight;
    const bottom = this.host.clientHeight - GAP - height;
    this.panel.style.top = `${Math.max(Math.min(focus - height / 2, bottom), GAP)}px`;
    this.recorder.layout();
  }

  /** Lays the captured desktop under the panel for the active skin. */
  dress() {
    const skin = this.host.dataset.skin!;
    this.backdrop.dress(this.panel, skin, this.host);
    if (skin === 'glass' && this.backdrop.present) {
      this.host.dataset.tone = this.backdrop.tone(this.rect());
      this.recorder.layout();
    }
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
