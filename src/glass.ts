// Draws the captured desktop behind the panel: as blurred acrylic, or bent
// through glass at the rim of each piece.

import { offsetWithin } from './chart';

const SVG = 'http://www.w3.org/2000/svg';

/**
 * Width of the curved rim of a piece of glass, and how far at most it bends
 * what is behind (px). The bend falls off as (1 - depth/RIM)², whose steepest
 * slope, at the very edge, is 2·BEND/RIM px of shift per px. Kept below one,
 * the image behind is squeezed toward the edge as a convex rim does; above
 * one it folds back on itself and reads as a hard break.
 */
const RIM = 40;
const BEND = 16;
/**
 * Backdrop luminance (0–1) that each theme's glass reads well over without
 * help, and the distance from it at which glass is fully frosted. Light
 * glass carries dark text and wants a light backdrop, dark glass the reverse.
 */
const READS_WELL = { light: 0.6, dark: 0.4 };
const CONTRAST_SPAN = 0.35;
/**
 * Spread of backdrop luminance (standard deviation, 0–0.5) at which glass is
 * fully frosted, and below which it stays clear. Text, icons and photos
 * behind the glass spread wide; an empty page or a plain wallpaper hardly.
 */
const BUSY = 0.14;
const CALM = 0.03;
/** Blur of fully frosted glass (px). */
const FROST_BLUR = 9;
/** Samples taken per px of backdrop when measuring it: fine enough to see text strokes. */
const SAMPLING = 1;

type Box = { left: number; top: number; width: number; height: number };

export class Backdrop {
  private image: HTMLImageElement | null = null;
  /** Where the current capture came from, and the blob address it is held under. */
  private source: string | null = null;
  private url: string | null = null;
  private defs: SVGDefsElement;
  private maps = new Map<string, string>();
  private stage: HTMLElement | null = null;

  constructor() {
    const svg = document.createElementNS(SVG, 'svg');
    svg.setAttribute('width', '0');
    svg.setAttribute('height', '0');
    svg.style.position = 'absolute';
    this.defs = document.createElementNS(SVG, 'defs');
    svg.append(this.defs);
    document.body.append(svg);
    document.body.style.setProperty('--noise', `url(${noiseTile()})`);
  }

  get present(): boolean {
    return this.image !== null;
  }

  /** Loads a capture, resolving once it can be drawn without a delay. */
  /**
   * Loads a capture, resolving once it is decoded. The picture is held in
   * memory under a blob address, which the glass layers then use: they find
   * it decoded and draw it at once, with no blank frame in between.
   */
  async load(source: string | null) {
    if (source === this.source) return;
    this.source = source;
    if (source === null) {
      this.drop(null, null);
      return;
    }
    const response = await fetch(source).catch(() => null);
    // A live capture can be replaced before it is fetched; keep the last one.
    if (!response?.ok || this.source !== source) return;
    const url = URL.createObjectURL(await response.blob());
    const image = new Image();
    image.src = url;
    const decoded = await image.decode().then(() => true, () => false);
    // A newer capture may have been asked for meanwhile.
    if (!decoded || this.source !== source) {
      URL.revokeObjectURL(url);
      return;
    }
    this.drop(url, image);
  }

  /** Takes `url` as the current picture, letting go of the one before. */
  private drop(url: string | null, image: HTMLImageElement | null) {
    const previous = this.url;
    this.url = url;
    this.image = image;
    // The layers still show the old picture until they are given the new one.
    if (previous) setTimeout(() => URL.revokeObjectURL(previous), 1000);
  }

  /**
   * Puts a newer capture of the same desktop into the layers `dress` built,
   * without rebuilding them, and frosts the glass for what it now shows.
   */
  retexture(panel: HTMLElement, skin: string, stage: HTMLElement) {
    if (!this.image) return;
    const url = `url(${this.url})`;
    for (const layer of panel.querySelectorAll<HTMLElement>('.lens, .acrylic')) {
      if (layer.style.backgroundImage) layer.style.backgroundImage = url;
    }
    if (skin === 'glass') {
      const frost = this.frost(panel, stage);
      panel.style.setProperty('--frost', frost.toFixed(3));
      for (const blur of this.defs.querySelectorAll('feGaussianBlur[result="frosted"]')) {
        blur.setAttribute('stdDeviation', (frost * FROST_BLUR).toFixed(2));
      }
    }
  }

  /**
   * How frosted the glass over `panel` should be, 0–1: more when what is
   * behind is busy, and more when it clashes with the theme (bright under
   * dark glass, dark under light glass).
   */
  private frost(panel: HTMLElement, stage: HTMLElement): number {
    this.stage = stage;
    const theme = stage.dataset.theme === 'dark' ? 'dark' : 'light';
    const origin = offsetWithin(panel, stage);
    const { mean, spread } = this.measure({ ...origin, width: panel.offsetWidth, height: panel.offsetHeight });
    const clamp = (value: number) => Math.min(Math.max(value, 0), 1);
    const busy = clamp((spread - CALM) / (BUSY - CALM));
    const against = theme === 'light' ? READS_WELL.light - mean : mean - READS_WELL.dark;
    return Math.max(busy, clamp(against / CONTRAST_SPAN));
  }

  /**
   * Lays the capture under `panel` for the active skin. The capture covers
   * `stage` exactly. Positions are taken
   * at rest, ignoring the entrance transform: the capture is aligned with the
   * screen where the panel will settle.
   */
  dress(panel: HTMLElement, skin: string, stage: HTMLElement) {
    this.stage = stage;
    const acrylic = panel.querySelector<HTMLElement>('.acrylic')!;
    const pieces = [...panel.querySelectorAll<HTMLElement>('.lane, .bar, .settings')];
    for (const lens of panel.querySelectorAll('.lens')) lens.remove();
    this.defs.replaceChildren();
    if (!this.image) {
      acrylic.style.backgroundImage = '';
      return;
    }

    const viewport = `${stage.clientWidth}px ${stage.clientHeight}px`;
    const url = `url(${this.url})`;
    const origin = offsetWithin(panel, stage);

    if (skin === 'fluent') {
      acrylic.style.backgroundImage = url;
      acrylic.style.backgroundSize = viewport;
      acrylic.style.backgroundPosition = `${-origin.left}px ${-origin.top}px`;
      this.defs.append(acrylicFilter(panel.offsetWidth, panel.offsetHeight));
    }

    if (skin === 'glass') {
      // The pieces are one material: they frost alike, by what is behind the
      // panel as a whole. Deciding per piece made neighbours look unrelated.
      const frost = this.frost(panel, stage);
      panel.style.setProperty('--frost', frost.toFixed(3));
      pieces.forEach((piece, index) => {
        if (piece.offsetParent === null) return;
        const width = piece.offsetWidth;
        const height = piece.offsetHeight;
        const radius = parseFloat(getComputedStyle(piece).borderTopLeftRadius);
        const within = offsetWithin(piece, panel);
        const left = origin.left + within.left;
        const top = origin.top + within.top;
        const id = `lens-${index}`;
        this.defs.append(lensFilter(id, width, height, this.map(width, height, radius), frost * FROST_BLUR));

        const lens = document.createElement('div');
        lens.className = 'lens';
        Object.assign(lens.style, {
          left: `${-BEND}px`,
          top: `${-BEND}px`,
          width: `${width + 2 * BEND}px`,
          height: `${height + 2 * BEND}px`,
          backgroundImage: url,
          backgroundSize: viewport,
          backgroundPosition: `${BEND - left}px ${BEND - top}px`,
          filter: `url(#${id})`,
        });
        piece.prepend(lens);
      });
    }
  }

  /** Whether the capture behind a box of `stage` is light rather than dark. */
  isLight(box: Box, stage: HTMLElement): boolean {
    this.stage = stage;
    return this.measure(box).mean > 0.5;
  }

  /** Mean and standard deviation of the capture's luminance behind a box of the viewport, 0–1. */
  measure(box: Box): { mean: number; spread: number } {
    const image = this.image!;
    const scale = image.naturalWidth / this.stage!.clientWidth;
    const canvas = document.createElement('canvas');
    canvas.width = Math.max(1, Math.round(box.width * SAMPLING));
    canvas.height = Math.max(1, Math.round(box.height * SAMPLING));
    const ctx = canvas.getContext('2d', { willReadFrequently: true })!;
    ctx.drawImage(image, box.left * scale, box.top * scale, box.width * scale, box.height * scale, 0, 0, canvas.width, canvas.height);
    const { data } = ctx.getImageData(0, 0, canvas.width, canvas.height);
    let sum = 0;
    let squares = 0;
    for (let i = 0; i < data.length; i += 4) {
      const y = (0.2126 * data[i] + 0.7152 * data[i + 1] + 0.0722 * data[i + 2]) / 255;
      sum += y;
      squares += y * y;
    }
    const count = data.length / 4;
    const mean = sum / count;
    return { mean, spread: Math.sqrt(Math.max(squares / count - mean * mean, 0)) };
  }


  /** The displacement map for a rounded rectangle, cached by size. */
  private map(width: number, height: number, radius: number): string {
    const key = `${width}x${height}x${radius}`;
    let url = this.maps.get(key);
    if (!url) {
      url = displacementMap(width, height, radius);
      this.maps.set(key, url);
    }
    return url;
  }
}

/**
 * Encodes, for every point of a rounded rectangle, where the glass makes it
 * look: unchanged in the middle, pulled outward across the rim, most at the
 * very edge, the way a convex edge of glass bends light. Red is the x shift
 * and green the y shift, 128 meaning none.
 */
function displacementMap(width: number, height: number, radius: number): string {
  const w = Math.round(width);
  const h = Math.round(height);
  const canvas = document.createElement('canvas');
  canvas.width = w;
  canvas.height = h;
  const ctx = canvas.getContext('2d')!;
  const image = ctx.createImageData(w, h);
  const hx = w / 2;
  const hy = h / 2;
  const r = Math.min(radius, hx, hy);
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      const px = x + 0.5 - hx;
      const py = y + 0.5 - hy;
      const qx = Math.abs(px) - (hx - r);
      const qy = Math.abs(py) - (hy - r);
      // Distance in from the edge, and the outward direction there.
      let depth: number;
      let nx = 0;
      let ny = 0;
      if (qx > 0 && qy > 0) {
        const length = Math.hypot(qx, qy);
        depth = r - length;
        nx = qx / length;
        ny = qy / length;
      } else if (qx > qy) {
        depth = hx - Math.abs(px);
        nx = 1;
      } else {
        depth = hy - Math.abs(py);
        ny = 1;
      }
      nx *= Math.sign(px);
      ny *= Math.sign(py);
      const t = Math.min(Math.max(depth / RIM, 0), 1);
      const strength = (1 - t) ** 2;
      const i = (y * w + x) * 4;
      image.data[i] = 128 + nx * strength * 127;
      image.data[i + 1] = 128 + ny * strength * 127;
      image.data[i + 2] = 128;
      image.data[i + 3] = 255;
    }
  }
  ctx.putImageData(image, 0, 0);
  return canvas.toDataURL();
}

function lensFilter(id: string, width: number, height: number, map: string, blur: number): SVGFilterElement {
  const filter = document.createElementNS(SVG, 'filter');
  const box = { x: 0, y: 0, width: width + 2 * BEND, height: height + 2 * BEND };
  filter.id = id;
  filter.setAttribute('filterUnits', 'userSpaceOnUse');
  filter.setAttribute('primitiveUnits', 'userSpaceOnUse');
  filter.setAttribute('color-interpolation-filters', 'sRGB');
  for (const [key, value] of Object.entries(box)) filter.setAttribute(key, String(value));
  filter.innerHTML = `
    <feGaussianBlur in="SourceGraphic" stdDeviation="${blur.toFixed(2)}" edgeMode="duplicate" result="frosted"/>
    <feImage href="${map}" x="${BEND}" y="${BEND}" width="${width}" height="${height}" preserveAspectRatio="none" result="map"/>
    <feDisplacementMap in="frosted" in2="map" scale="${2 * BEND}" xChannelSelector="R" yChannelSelector="G" result="bent"/>
    <feColorMatrix in="bent" type="saturate" values="1.3"/>`;
  return filter;
}

/** The Windows acrylic recipe: a wide blur and a lift in saturation. */
function acrylicFilter(width: number, height: number): SVGFilterElement {
  const filter = document.createElementNS(SVG, 'filter');
  filter.id = 'acrylic';
  filter.setAttribute('filterUnits', 'userSpaceOnUse');
  filter.setAttribute('color-interpolation-filters', 'sRGB');
  filter.setAttribute('x', '0');
  filter.setAttribute('y', '0');
  filter.setAttribute('width', String(width));
  filter.setAttribute('height', String(height));
  filter.innerHTML = `
    <feGaussianBlur stdDeviation="30" edgeMode="duplicate"/>
    <feColorMatrix type="saturate" values="1.25"/>`;
  return filter;
}

/** A tile of faint grain, as acrylic carries to keep large blurs from banding. */
function noiseTile(): string {
  const canvas = document.createElement('canvas');
  canvas.width = 64;
  canvas.height = 64;
  const ctx = canvas.getContext('2d')!;
  const image = ctx.createImageData(64, 64);
  for (let i = 0; i < image.data.length; i += 4) {
    const v = Math.random() * 255;
    image.data[i] = v;
    image.data[i + 1] = v;
    image.data[i + 2] = v;
    image.data[i + 3] = 6;
  }
  ctx.putImageData(image, 0, 0);
  return canvas.toDataURL();
}
