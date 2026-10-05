// The settings window: choices on the left, and on the right the panel as it
// will look, over the desktop the window was opened on, running live.

import './settings-window.css';

import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';

import { element } from './format';
import { Backdrop } from './glass';
import { setLanguage, t } from './i18n';
import { catalog } from './modules';
import { onSystemTheme, resolveTheme } from './theme';
import { PanelView, roomFor } from './panelView';
import { buildSettings, resolvePrefs } from './settings';
import type { Bootstrap, Sample, Settings } from './types';

/** Desktop shown beside the panel in the preview (px of screen). */
const PREVIEW_MARGIN = 160;

interface PreviewInfo {
  wallpaper: string | null;
  color: string;
  width: number;
  height: number;
}

/**
 * The desktop the preview stands in for, at the size of the work area:
 * the wallpaper scaled to fill it as Windows does by default, or the plain
 * desktop colour.
 */
async function desktopImage(info: PreviewInfo): Promise<string> {
  const ratio = window.devicePixelRatio;
  const canvas = document.createElement('canvas');
  canvas.width = Math.round(info.width * ratio);
  canvas.height = Math.round(info.height * ratio);
  const ctx = canvas.getContext('2d')!;
  ctx.fillStyle = info.color;
  ctx.fillRect(0, 0, canvas.width, canvas.height);
  if (info.wallpaper) {
    const image = new Image();
    image.crossOrigin = 'anonymous';
    image.src = info.wallpaper;
    await image.decode();
    const cover = Math.max(canvas.width / image.naturalWidth, canvas.height / image.naturalHeight);
    const width = image.naturalWidth * cover;
    const height = image.naturalHeight * cover;
    ctx.drawImage(image, (canvas.width - width) / 2, (canvas.height - height) / 2, width, height);
  }
  const blob = await new Promise<Blob>((resolve) => canvas.toBlob((b) => resolve(b!)));
  return URL.createObjectURL(blob);
}

export async function settingsApp(boot: Bootstrap) {
  const settings: Settings = boot.settings;
  const prefs = resolvePrefs(settings.view, catalog(boot.info));
  settings.view = prefs;
  setLanguage(prefs.language);
  let modules = catalog(boot.info);
  let autostart = boot.autostart;

  const body = document.body;
  body.classList.add('settings-app');
  body.style.setProperty('--accent-on-light', boot.accent.on_light);
  body.style.setProperty('--accent-on-dark', boot.accent.on_dark);

  const layout = element(`
    <div class="settings-layout">
      <div class="settings-pane"></div>
      <div class="preview-pane">
        <div class="preview-frame"></div>
      </div>
    </div>`);
  body.append(layout);
  const pane = layout.querySelector<HTMLElement>('.settings-pane')!;
  const frame = layout.querySelector<HTMLElement>('.preview-frame')!;

  // The preview stage is the work area at its real size, scaled to fit the
  // frame; the panel inside it is laid out exactly as on screen.
  const stage = element(`<div class="stage" data-state="open"></div>`);
  frame.append(stage);
  const backdrop = new Backdrop();
  const view = new PanelView(stage, modules, backdrop, () => {});
  let size: PreviewInfo = { wallpaper: null, color: '#000000', width: 1920, height: 1040 };

  /** Takes the desktop and work area of the monitor the window opens on. */
  const loadDesktop = async () => {
    size = await invoke<PreviewInfo>('preview');
    const desktop = await desktopImage(size);
    stage.style.width = `${size.width}px`;
    stage.style.height = `${size.height}px`;
    stage.style.backgroundImage = `url(${desktop})`;
    await backdrop.load(desktop);
  };

  /** Asks to be shown once what is drawn has reached the screen. */
  const ready = () => requestAnimationFrame(() => void invoke('settings_ready'));

  /**
   * Scales the stage to the frame, anchored at the edge the panel opens
   * from: the whole height of the screen, and the whole panel with a strip of
   * desktop beside it.
   */
  const fit = () => {
    const shown = view.rect().width + PREVIEW_MARGIN;
    const scale = Math.min(frame.clientHeight / size.height, frame.clientWidth / Math.min(shown, size.width), 1);
    stage.style.transform = `scale(${scale})`;
    // Centred top to bottom when the width is what limits the scale.
    stage.style.top = `${(frame.clientHeight - size.height * scale) / 2}px`;
    // Along the top, the stage is centred and shows the panel and the top
    // part of the screen below it.
    const edge = settings.edge;
    stage.style.transformOrigin = edge === 'right' ? 'top right' : edge === 'left' ? 'top left' : 'top center';
    stage.style.left = edge === 'left' ? '0' : edge === 'top' ? `${(frame.clientWidth - size.width) / 2}px` : '';
    stage.style.right = edge === 'right' ? '0' : '';
    if (edge === 'top') {
      const shownHeight = view.rect().height + PREVIEW_MARGIN;
      const topScale = Math.min(frame.clientWidth / size.width, frame.clientHeight / shownHeight, 1);
      stage.style.transform = `scale(${topScale})`;
      stage.style.top = `${Math.max((frame.clientHeight - shownHeight * topScale) / 2, 0)}px`;
    }
  };

  const render = () => {
    const theme = resolveTheme(prefs.theme);
    body.dataset.theme = theme;
    stage.dataset.theme = theme;
    // The title bar is drawn by the system; it follows the window's theme.
    void getCurrentWindow().setTheme(theme);
    view.build(settings, prefs);
    view.layout((measures) => roomFor(settings.edge, size.width, size.height, measures), {
      x: size.width / 2,
      y: size.height / 2,
    });
    view.dress(prefs.theme);
    fit();
  };

  const save = () => void invoke('save_settings', { settings });

  /** Draws the choices, in the current language. */
  const fill = () => {
    void getCurrentWindow().setTitle(t('windowTitle'));
    const scroll = pane.scrollTop;
    pane.replaceChildren(
      element(`<h1>${t('settings')}</h1>`),
      buildSettings(settings, prefs, modules, autostart, {
        changed() {
          save();
          render();
        },
        moved() {
          save();
          render();
        },
        relabel() {
          save();
          setLanguage(prefs.language);
          modules = catalog(boot.info);
          view.modules = modules;
          fill();
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
    pane.scrollTop = scroll;
  };
  fill();

  await Promise.all([
    loadDesktop(),
    invoke<Sample[]>('history').then((history) => (view.samples = history)),
    ...['Archivo Variable', 'Inter Variable'].map((family) => document.fonts.load(`13px "${family}"`)),
  ]);
  await listen<Sample>('sample', ({ payload }) => view.push(payload, 300_000 / settings.interval_ms + 16));
  render();
  new ResizeObserver(fit).observe(frame);
  onSystemTheme(render);
  ready();

  const draw = () => {
    view.draw(Date.now());
    requestAnimationFrame(draw);
  };
  draw();
}
