import '@fontsource-variable/archivo/wdth.css';
import '@fontsource-variable/inter/index.css';
import './base.css';
import './skins/paper.css';
import './skins/glass.css';
import './skins/fluent.css';

import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

import { type Plot, Recorder } from './chart';
import * as fmt from './format';
import { Backdrop } from './glass';
import { catalog, type Lane } from './modules';
import { buildSettings, resolvePrefs } from './settings';
import type { Bootstrap, Sample, Settings, Skin, ViewName } from './types';

/** Closest the panel gets to the top and bottom of the window (px); the backend keeps the same. */
const GAP = 12;
/** Matches the closing transition in base.css. */
const CLOSE_MS = 180;

/** The Settings glyph of the system icon font. */
const GEAR = '<span class="glyph" aria-hidden="true"></span>';

async function start() {
  const boot = await invoke<Bootstrap>('bootstrap');
  const settings: Settings = boot.settings;
  const modules = catalog(boot.info);
  const prefs = resolvePrefs(settings.view, modules);
  settings.view = prefs;
  let autostart = boot.autostart;

  const body = document.body;
  const panel = document.getElementById('panel')!;
  const lanesHost = document.getElementById('lanes')!;
  const settingsHost = document.getElementById('settings')!;
  const canvas = document.getElementById('chart') as HTMLCanvasElement;
  const barText = document.getElementById('bar-text')!;
  const barButton = document.getElementById('bar-button') as HTMLButtonElement;
  const backdrop = new Backdrop();

  body.style.setProperty('--accent-on-light', boot.accent.on_light);
  body.style.setProperty('--accent-on-dark', boot.accent.on_dark);
  body.dataset.edge = settings.edge;
  body.dataset.skin = settings.skin;
  body.dataset.state = 'hidden';
  body.dataset.view = 'monitor';
  body.dataset.tone = matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';

  let samples: Sample[] = [];
  let epoch = 0;
  let focusY = 0;
  let drawing = false;
  let lanes: Lane[] = [];
  let recorder: Recorder;

  const save = () => void invoke('save_settings', { settings });

  /** Rebuilds the lanes from the preferences. */
  const buildLanes = () => {
    const byId = new Map(modules.map((module) => [module.id, module]));
    lanes = prefs.modules.filter((entry) => entry.on).map((entry) => byId.get(entry.id)!.build(prefs));
    lanesHost.replaceChildren(...lanes.map((lane) => lane.root));
    const plots = lanes.flatMap((lane) => lane.plots);
    recorder = new Recorder(canvas, panel, plots, settings.interval_ms + 100, prefs.chartSeconds * 1000);
    showLatest();
  };

  const showLatest = () => {
    const latest = samples[samples.length - 1];
    if (!latest) return;
    const scale = (plot: Plot) => recorder.scale(plot, samples);
    for (const lane of lanes) lane.update(latest, scale);
    barText.textContent =
      body.dataset.view === 'settings' ? '设置' : `已开机 ${fmt.duration(latest.system.uptime_s)}`;
  };

  /** Tells the window host how the current skin wants its window. */
  let reportedSurface = '';
  const reportSurface = () => {
    const style = getComputedStyle(body);
    const token = (name: string) => style.getPropertyValue(name).trim();
    const surface = {
      width: panel.offsetWidth,
      height: panel.offsetHeight,
      margin: Number(token('--margin')),
      inset: Number(token('--inset')),
      backdrop: token('--backdrop'),
    };
    const key = JSON.stringify(surface);
    if (key === reportedSurface) return;
    reportedSurface = key;
    void invoke('set_surface', { surface });
  };

  /** Puts the panel at its height in the window and tells the host where it is. */
  const place = () => {
    const height = panel.offsetHeight;
    const wanted = settings.anchor === 'center' ? window.innerHeight / 2 : focusY;
    const top = Math.min(Math.max(wanted - height / 2, GAP), window.innerHeight - GAP - height);
    panel.style.top = `${Math.max(top, 0)}px`;
    if (body.dataset.state === 'hidden') return;
    void invoke('set_panel_rect', {
      epoch,
      rect: { left: panel.offsetLeft, top: panel.offsetTop, width: panel.offsetWidth, height },
    });
  };

  /** Everything that depends on where things are: plots, glass, tone. */
  const relayout = () => {
    place();
    recorder.layout();
    if (body.dataset.state === 'hidden') return;
    backdrop.dress(panel, settings.skin);
    if (settings.skin === 'glass' && backdrop.present) {
      body.dataset.tone = backdrop.tone({
        left: panel.offsetLeft,
        top: panel.offsetTop,
        width: panel.offsetWidth,
        height: panel.offsetHeight,
      });
      recorder.layout();
    }
  };

  const setView = (view: ViewName) => {
    body.dataset.view = view;
    void invoke('set_hold', { hold: view === 'settings' });
    if (view === 'settings') {
      settingsHost.replaceChildren(buildSettings(settings, prefs, modules, autostart, settingsActions));
      barButton.innerHTML = '完成';
      barButton.setAttribute('aria-label', '完成');
    } else {
      settingsHost.replaceChildren();
      barButton.innerHTML = GEAR;
      barButton.setAttribute('aria-label', '设置');
    }
    showLatest();
    reportSurface();
    relayout();
  };

  const settingsActions = {
    changed(layout: boolean) {
      save();
      if (layout) buildLanes();
    },
    skin(skin: Skin) {
      settings.skin = skin;
      save();
      body.dataset.skin = skin;
      if (skin !== 'glass') body.dataset.tone = matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
      reportSurface();
      void invoke('relocate', { epoch });
    },
    edge() {
      save();
      body.dataset.edge = settings.edge;
      void invoke('relocate', { epoch });
    },
    async autostart(enabled: boolean) {
      autostart = await invoke<boolean>('set_autostart', { enabled });
      return autostart;
    },
    quit() {
      void invoke('quit');
    },
  };

  barButton.addEventListener('click', () => setView(body.dataset.view === 'settings' ? 'monitor' : 'settings'));

  const frame = () => {
    drawing = body.dataset.state !== 'hidden';
    if (!drawing) return;
    if (body.dataset.view === 'monitor') recorder.draw(samples, Date.now());
    requestAnimationFrame(frame);
  };

  await listen<{ epoch: number; history: Sample[]; view: ViewName; focus_y: number; backdrop: string | null }>(
    'panel-open',
    async ({ payload }) => {
      const reopening = body.dataset.state === 'closing';
      epoch = payload.epoch;
      samples = payload.history;
      if (!reopening) focusY = payload.focus_y / window.devicePixelRatio;
      body.dataset.state = 'loading';
      setView(payload.view);
      await backdrop.load(payload.backdrop);
      if (epoch !== payload.epoch) return;
      relayout();
      body.dataset.state = 'open';
      if (!drawing) frame();
    },
  );

  await listen<{ epoch: number; backdrop: string | null }>('panel-backdrop', async ({ payload }) => {
    await backdrop.load(payload.backdrop);
    if (epoch === payload.epoch) relayout();
  });

  await listen<ViewName>('panel-view', ({ payload }) => setView(payload));

  await listen<Sample>('sample', ({ payload }) => {
    samples.push(payload);
    const keep = (300_000 / settings.interval_ms) + 16;
    while (samples.length > keep) samples.shift();
    showLatest();
  });

  await listen<number>('panel-close', ({ payload: closing }) => {
    body.dataset.state = 'closing';
    setTimeout(() => {
      // A reopen during the slide-out leaves the panel on screen.
      if (epoch !== closing || body.dataset.state !== 'closing') return;
      body.dataset.state = 'hidden';
      void invoke('panel_hidden', { epoch: closing });
    }, CLOSE_MS);
  });

  matchMedia('(prefers-color-scheme: dark)').addEventListener('change', (event) => {
    if (settings.skin !== 'glass') body.dataset.tone = event.matches ? 'dark' : 'light';
    recorder.layout();
  });
  // A new zoom, or the window moved to another monitor.
  window.addEventListener('resize', relayout);

  await Promise.all(['Archivo Variable', 'Inter Variable'].map((family) => document.fonts.load(`13px "${family}"`)));
  buildLanes();
  setView('monitor');
  // Rows come and go (drives, details switched on), and the window follows.
  // A zoom change resizes the panel by rounding alone; reporting that would
  // feed back into the zoom. Only a size change at a steady zoom is content.
  let measuredAt = window.devicePixelRatio;
  new ResizeObserver(() => {
    if (window.devicePixelRatio === measuredAt) reportSurface();
    measuredAt = window.devicePixelRatio;
    relayout();
  }).observe(panel);
}

void start();
