import type { Sample } from './types';

export interface Plot {
  el: HTMLElement;
  /** First series is drawn as the main trace, the rest as lighter lines. */
  series: ((sample: Sample) => number)[];
  /** Full-scale value, or 'auto' to fit the busiest moment on screen. */
  max: number | 'auto';
  /** Values above this are drawn in the signal colour. */
  hotAbove?: number;
}

/** Where a plot sits on the canvas, and the colours its skin gives it. */
interface Frame {
  left: number;
  top: number;
  width: number;
  height: number;
  trace: string;
  trace2: string;
  rule: string;
  wash: string;
  washEnd: string;
  signal: string;
}

/** Rounds a byte rate up to 1, 2 or 5 times a power of ten of its unit. */
function roundUpRate(value: number): number {
  const unit = 1024 ** Math.floor(Math.log(value) / Math.log(1024));
  const magnitude = 10 ** Math.floor(Math.log10(value / unit));
  const leading = value / unit / magnitude;
  return (leading <= 1 ? 1 : leading <= 2 ? 2 : leading <= 5 ? 5 : 10) * magnitude * unit;
}

/** Smallest full scale for an auto-scaled plot, so idle chatter stays near the baseline. */
const MIN_RATE_SCALE = 10 * 1024;
/** Time rules are drawn at six even divisions of the span. */
const RULES = 6;

/**
 * Draws every plot as a strip chart on one canvas. The paper feeds
 * continuously: drawing runs one sample interval behind the clock, so the pen
 * always has the next sample to move towards.
 */
export class Recorder {
  private ctx: CanvasRenderingContext2D;
  private frames: Frame[] = [];

  constructor(
    private canvas: HTMLCanvasElement,
    private panel: HTMLElement,
    private plots: Plot[],
    private delayMs: number,
    private spanMs: number,
  ) {
    this.ctx = canvas.getContext('2d')!;
  }

  /** Re-reads geometry, pixel density and the skin's colours. */
  layout() {
    const ratio = window.devicePixelRatio;
    this.canvas.width = Math.round(this.panel.offsetWidth * ratio);
    this.canvas.height = Math.round(this.panel.offsetHeight * ratio);
    this.ctx.setTransform(ratio, 0, 0, ratio, 0, 0);

    // Offsets rather than client rects: the panel may be mid-animation.
    const probe = document.createElement('i');
    probe.hidden = true;
    this.frames = this.plots.map(({ el }) => {
      el.append(probe);
      // Tokens can be any CSS colour expression; a probe resolves them in the
      // context of each plot to something the canvas accepts.
      const color = (token: string) => {
        probe.style.color = `var(${token})`;
        return getComputedStyle(probe).color;
      };
      const { left, top } = offsetWithin(el, this.panel);
      return {
        left,
        top,
        width: el.offsetWidth,
        height: el.offsetHeight,
        trace: color('--trace'),
        trace2: color('--trace-2'),
        rule: color('--rule'),
        wash: color('--wash'),
        washEnd: color('--wash-end'),
        signal: color('--signal'),
      };
    });
    probe.remove();
  }

  /** Full-scale value of a plot for the samples currently on screen. */
  scale(plot: Plot, samples: Sample[]): number {
    if (plot.max !== 'auto') return plot.max;
    const oldest = Date.now() - this.delayMs - this.spanMs;
    let peak = MIN_RATE_SCALE;
    for (const sample of samples) {
      if (sample.t < oldest) continue;
      for (const read of plot.series) peak = Math.max(peak, read(sample));
    }
    return roundUpRate(peak);
  }

  draw(samples: Sample[], now: number) {
    const { ctx } = this;
    const penTime = now - this.delayMs;
    const ruleMs = this.spanMs / RULES;
    // In device pixels: the drawing transform scales by less than one at low zoom.
    ctx.save();
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.clearRect(0, 0, this.canvas.width, this.canvas.height);
    ctx.restore();
    ctx.lineJoin = 'round';

    this.plots.forEach((plot, index) => {
      const frame = this.frames[index];
      if (frame.width === 0) return;
      const right = frame.left + frame.width;
      const bottom = frame.top + frame.height;
      const pxPerMs = frame.width / this.spanMs;
      const max = this.scale(plot, samples);
      const x = (t: number) => right - (penTime - t) * pxPerMs;
      const y = (value: number) => bottom - Math.min(Math.max(value / max, 0), 1) * (frame.height - 2);

      // Time rules travel with the paper.
      ctx.strokeStyle = frame.rule;
      ctx.lineWidth = 1;
      ctx.beginPath();
      for (let t = Math.floor(penTime / ruleMs) * ruleMs; x(t) > frame.left; t -= ruleMs) {
        ctx.moveTo(Math.round(x(t)) + 0.5, frame.top);
        ctx.lineTo(Math.round(x(t)) + 0.5, bottom);
      }
      ctx.moveTo(frame.left, bottom + 0.5);
      ctx.lineTo(right, bottom + 0.5);
      ctx.stroke();

      // One sample beyond each end of the window, so the trace runs off both sides.
      let first = samples.findIndex((sample) => sample.t >= penTime - this.spanMs);
      if (first === -1) return;
      first = Math.max(first - 1, 0);
      const visible = samples.slice(first);

      ctx.save();
      ctx.beginPath();
      ctx.rect(frame.left, frame.top - 2, frame.width, frame.height + 2);
      ctx.clip();
      plot.series.forEach((read, order) => {
        const line = new Path2D();
        visible.forEach((sample, i) => {
          if (i === 0) line.moveTo(x(sample.t), y(read(sample)));
          else line.lineTo(x(sample.t), y(read(sample)));
        });
        const last = visible[visible.length - 1];
        // The pen holds its position if the next sample is late.
        if (last.t < penTime) line.lineTo(right, y(read(last)));

        if (order === 0) {
          const area = new Path2D(line);
          area.lineTo(Math.max(x(last.t), right), bottom);
          area.lineTo(x(visible[0].t), bottom);
          area.closePath();
          const fill = ctx.createLinearGradient(0, frame.top, 0, bottom);
          fill.addColorStop(0, frame.wash);
          fill.addColorStop(1, frame.washEnd);
          ctx.fillStyle = fill;
          ctx.fill(area);
        }
        ctx.strokeStyle = order === 0 ? frame.trace : frame.trace2;
        ctx.lineWidth = order === 0 ? 1.5 : 1;
        ctx.stroke(line);

        if (order === 0 && plot.hotAbove !== undefined) {
          ctx.save();
          ctx.beginPath();
          ctx.rect(frame.left, frame.top - 2, frame.width, y(plot.hotAbove) - frame.top + 2);
          ctx.clip();
          ctx.strokeStyle = frame.signal;
          ctx.stroke(line);
          ctx.restore();
        }
      });
      ctx.restore();

      // The pen tip sits at the moment being drawn.
      const read = plot.series[0];
      const next = visible.findIndex((sample) => sample.t >= penTime);
      const after = visible[next === -1 ? visible.length - 1 : next];
      const before = visible[next <= 0 ? (next === -1 ? visible.length - 1 : 0) : next - 1];
      const progress = after.t === before.t ? 0 : (penTime - before.t) / (after.t - before.t);
      const value = read(before) + (read(after) - read(before)) * progress;
      ctx.fillStyle = plot.hotAbove !== undefined && value > plot.hotAbove ? frame.signal : frame.trace;
      ctx.beginPath();
      ctx.arc(right, y(value), 2.5, 0, Math.PI * 2);
      ctx.fill();
    });
  }
}

/** Position of `el` relative to `ancestor`, ignoring transforms. */
export function offsetWithin(el: HTMLElement, ancestor: HTMLElement): { left: number; top: number } {
  let left = 0;
  let top = 0;
  let node: HTMLElement | null = el;
  while (node && node !== ancestor) {
    left += node.offsetLeft;
    top += node.offsetTop;
    node = node.offsetParent as HTMLElement | null;
  }
  return { left, top };
}
