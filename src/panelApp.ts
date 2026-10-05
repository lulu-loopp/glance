// The panel window: a column along the screen edge that the backend shows
// and hides. This page lays the panel out inside it and animates it.

import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

import { Backdrop } from './glass';
import { setLanguage } from './i18n';
import { catalog } from './modules';
import { onSystemTheme, resolveTheme } from './theme';
import { PanelView, type Room } from './panelView';
import { resolvePrefs } from './settings';
import type { Bootstrap, Sample, Settings } from './types';

/** Matches the closing transition in base.css. */
const CLOSE_MS = 180;

export async function panelApp(boot: Bootstrap) {
  let settings: Settings = boot.settings;
  let prefs = resolvePrefs(settings.view, catalog(boot.info));
  setLanguage(prefs.language);
  const modules = catalog(boot.info);
  const body = document.body;
  const backdrop = new Backdrop();
  const view = new PanelView(body, modules, backdrop, () => void invoke('open_settings'));

  body.style.setProperty('--accent-on-light', boot.accent.on_light);
  body.style.setProperty('--accent-on-dark', boot.accent.on_dark);
  body.dataset.state = 'hidden';

  let epoch = 0;
  let room: Room = { height: window.innerHeight, max_columns: 3 };
  let focus = { x: window.innerWidth / 2, y: window.innerHeight / 2 };
  let drawing = false;

  /** Tells the window host how the current skin wants its window. */
  let reportedSurface = '';
  const reportSurface = () => {
    const style = getComputedStyle(body);
    const token = (name: string) => style.getPropertyValue(name).trim();
    const { laneHeights, laneGap, chromeHeight, columnWidth, columnGap } = view.measures;
    const surface = {
      lane_heights: laneHeights,
      lane_gap: laneGap,
      chrome_height: chromeHeight,
      column_width: columnWidth,
      column_gap: columnGap,
      margin: Number(token('--margin')),
      inset: Number(token('--inset')),
      backdrop: token('--backdrop'),
    };
    const key = JSON.stringify(surface);
    if (key === reportedSurface) return;
    reportedSurface = key;
    void invoke('set_surface', { surface });
  };

  /** Lets go of the captured desktop and the glass built on it while hidden. */
  const release = () => {
    void backdrop.load(null);
    view.dress(prefs.theme);
  };

  /** Lays the panel out and, while it is up, tells the host where it is. */
  const relayout = () => {
    const centre = { x: window.innerWidth / 2, y: window.innerHeight / 2 };
    view.layout(() => room, settings.anchor === 'center' ? centre : focus);
    if (body.dataset.state === 'hidden') return;
    view.dress(prefs.theme);
    void invoke('set_panel_rect', { epoch, rect: view.rect() });
  };

  const build = () => {
    view.build(settings, prefs);
    body.dataset.theme = resolveTheme(prefs.theme);
    relayout();
    reportSurface();
  };

  const frame = () => {
    drawing = body.dataset.state !== 'hidden';
    if (!drawing) return;
    view.draw(Date.now());
    requestAnimationFrame(frame);
  };

  await listen<{ epoch: number; history: Sample[]; room: Room; focus_x: number; focus_y: number; backdrop: string | null }>(
    'panel-open',
    async ({ payload }) => {
      const reopening = body.dataset.state === 'closing';
      epoch = payload.epoch;
      view.samples = payload.history;
      room = payload.room;
      if (!reopening) {
        const ratio = window.devicePixelRatio;
        focus = { x: payload.focus_x / ratio, y: payload.focus_y / ratio };
      }
      body.dataset.state = 'loading';
      view.showLatest();
      await backdrop.load(payload.backdrop);
      // Superseded, or asked to close while the capture was loading: the
      // close is already under way and must not be undone.
      if (epoch !== payload.epoch || body.dataset.state !== 'loading') return;
      relayout();
      body.dataset.state = 'open';
      if (!drawing) frame();
    },
  );

  await listen<{ epoch: number; room: Room; backdrop: string | null }>('panel-backdrop', async ({ payload }) => {
    room = payload.room;
    await backdrop.load(payload.backdrop);
    if (epoch === payload.epoch) relayout();
  });

  await listen<Sample>('sample', ({ payload }) => {
    view.push(payload, 300_000 / settings.interval_ms + 16);
    // A hidden page gets a sample only when it changes the layout (rows come
    // or go). The window is hidden then, so nothing that waits on rendering
    // (a resize observer) runs: measure now, so the window is sized right
    // before the panel next opens.
    if (body.dataset.state === 'hidden') {
      relayout();
      reportSurface();
    }
  });

  await listen<number>('panel-close', ({ payload: closing }) => {
    body.dataset.state = 'closing';
    setTimeout(() => {
      // A reopen during the slide-out leaves the panel on screen.
      if (epoch !== closing || body.dataset.state !== 'closing') return;
      body.dataset.state = 'hidden';
      void invoke('panel_hidden', { epoch: closing });
      release();
    }, CLOSE_MS);
  });

  // A choice made on the panel itself (sorting the processes).
  window.addEventListener('prefs-changed', () => {
    settings.view = prefs;
    void invoke('save_settings', { settings });
  });

  // A live backdrop, captured again while the panel is open.
  await listen<{ epoch: number; backdrop: string | null }>('backdrop-frame', async ({ payload }) => {
    await backdrop.load(payload.backdrop);
    if (epoch === payload.epoch && body.dataset.state === 'open') view.retexture();
  });

  // Taken off the screen at once, as the settings window opens.
  await listen<number>('panel-dismissed', () => {
    body.dataset.state = 'hidden';
    release();
  });

  // Changed in the settings window.
  await listen<Settings>('settings-changed', ({ payload }) => {
    const moved = payload.skin !== settings.skin || payload.edge !== settings.edge;
    settings = payload;
    prefs = resolvePrefs(settings.view, view.modules);
    setLanguage(prefs.language);
    view.modules = catalog(boot.info);
    build();
    if (moved && body.dataset.state === 'open') void invoke('relocate', { epoch });
  });

  onSystemTheme(() => {
    body.dataset.theme = resolveTheme(prefs.theme);
    view.recolor();
    if (body.dataset.state === 'open') view.dress(prefs.theme);
  });
  // A new zoom, or the window moved to another monitor.
  window.addEventListener('resize', relayout);

  await Promise.all(['Archivo Variable', 'Inter Variable'].map((family) => document.fonts.load(`13px "${family}"`)));
  build();
  // Rows come and go (drives, details switched on), and the window follows.
  // A zoom change resizes the panel by rounding alone; reporting that would
  // feed back into the zoom. Only a size change at a steady zoom is content.
  let measuredAt = window.devicePixelRatio;
  new ResizeObserver(() => {
    if (window.devicePixelRatio === measuredAt) reportSurface();
    measuredAt = window.devicePixelRatio;
    relayout();
  }).observe(view.panel);
}
