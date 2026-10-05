// The settings window: choices on the left, and on the right the panel as it
// will look, over the desktop the window was opened on, running live.

import './settings-window.css';

import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

import { element } from './format';
import { Backdrop } from './glass';
import { catalog } from './modules';
import { PanelView } from './panelView';
import { buildSettings, resolvePrefs } from './settings';
import type { Bootstrap, Sample, Settings, Skin } from './types';

/** Desktop shown beside the panel in the preview (px of screen). */
const PREVIEW_MARGIN = 160;

interface PreviewInfo {
  url: string;
  width: number;
  height: number;
}

export async function settingsApp(boot: Bootstrap) {
  const settings: Settings = boot.settings;
  const modules = catalog(boot.info);
  const prefs = resolvePrefs(settings.view, modules);
  settings.view = prefs;
  let autostart = boot.autostart;

  const body = document.body;
  body.classList.add('settings-app');
  body.style.setProperty('--accent-on-light', boot.accent.on_light);
  body.style.setProperty('--accent-on-dark', boot.accent.on_dark);

  const layout = element(`
    <div class="settings-layout">
      <div class="settings-pane">
        <h1>设置</h1>
      </div>
      <div class="preview-pane">
        <div class="preview-frame"></div>
      </div>
    </div>`);
  body.append(layout);
  const pane = layout.querySelector<HTMLElement>('.settings-pane')!;
  const frame = layout.querySelector<HTMLElement>('.preview-frame')!;

  // The preview stage is the captured work area at its real size, scaled to
  // fit the frame; the panel inside it is laid out exactly as on screen.
  const preview = await invoke<PreviewInfo | null>('preview');
  const size = preview ?? { width: 1920, height: 1040 };
  const stage = element(`<div class="stage" data-state="open"></div>`);
  stage.style.width = `${size.width}px`;
  stage.style.height = `${size.height}px`;
  if (preview) stage.style.backgroundImage = `url(${preview.url})`;
  frame.append(stage);

  const backdrop = new Backdrop();
  await backdrop.load(preview?.url ?? null);
  const view = new PanelView(stage, modules, backdrop, () => {});
  const systemTone = () => (matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light');

  /**
   * Scales the stage to the frame, anchored at the edge the panel opens
   * from: the whole height of the screen, and the whole panel with a strip of
   * desktop beside it.
   */
  const fit = () => {
    const shown = view.rect().width + PREVIEW_MARGIN;
    const scale = Math.min(frame.clientHeight / size.height, frame.clientWidth / Math.min(shown, size.width), 1);
    stage.style.transform = `scale(${scale})`;
    const right = settings.edge === 'right';
    stage.style.transformOrigin = right ? 'top right' : 'top left';
    stage.style.left = right ? '' : '0';
    stage.style.right = right ? '0' : '';
  };

  const render = () => {
    view.build(settings, prefs);
    if (settings.skin !== 'glass') stage.dataset.tone = systemTone();
    view.layout(size.height, size.height / 2);
    view.dress();
    fit();
  };

  const save = () => void invoke('save_settings', { settings });

  pane.append(
    buildSettings(settings, prefs, modules, autostart, {
      changed() {
        save();
        render();
      },
      skin(skin: Skin) {
        settings.skin = skin;
        save();
        render();
      },
      edge() {
        save();
        render();
      },
      async autostart(enabled: boolean) {
        autostart = await invoke<boolean>('set_autostart', { enabled });
        return autostart;
      },
      quit() {
        void invoke('quit');
      },
    }),
  );

  view.samples = await invoke<Sample[]>('history');
  await listen<Sample>('sample', ({ payload }) => view.push(payload, 300_000 / settings.interval_ms + 16));

  await Promise.all(['Archivo Variable', 'Inter Variable'].map((family) => document.fonts.load(`13px "${family}"`)));
  render();
  new ResizeObserver(fit).observe(frame);
  matchMedia('(prefers-color-scheme: dark)').addEventListener('change', render);

  const draw = () => {
    view.draw(Date.now());
    requestAnimationFrame(draw);
  };
  draw();
}
