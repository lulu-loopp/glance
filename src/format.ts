import { t } from './i18n';

const GIB = 1024 ** 3;

export function rate(bytesPerSecond: number, bits: boolean): string {
  if (bits) {
    const units = ['bps', 'Kbps', 'Mbps', 'Gbps'];
    return scaled(bytesPerSecond * 8, 1000, units);
  }
  return scaled(bytesPerSecond, 1024, ['B/s', 'KB/s', 'MB/s', 'GB/s']);
}

export function bytes(value: number): string {
  return scaled(value, 1024, ['B', 'KB', 'MB', 'GB', 'TB']);
}

/** Three significant figures at most, in the largest unit that keeps the value above one. */
function scaled(value: number, base: number, units: string[]): string {
  let unit = 0;
  while (value >= 1000 && unit < units.length - 1) {
    value /= base;
    unit += 1;
  }
  const digits = unit === 0 || value >= 100 ? 0 : 1;
  return `${value.toFixed(digits)} ${units[unit]}`;
}

export function usage(used: number, total: number): string {
  const decimals = total >= 100 * GIB ? 0 : 1;
  return `${(used / GIB).toFixed(decimals)} / ${(total / GIB).toFixed(decimals)} GB`;
}

export function size(value: number): string {
  return value >= GIB ? `${(value / GIB).toFixed(1)} GB` : `${Math.round(value / 1024 ** 2)} MB`;
}

export function percent(value: number): string {
  return `${value < 10 ? value.toFixed(1) : Math.round(value)}%`;
}

export function duration(seconds: number): string {
  const days = Math.floor(seconds / 86400);
  const hours = Math.floor((seconds % 86400) / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  if (days > 0) return t('days', days, hours);
  if (hours > 0) return t('hours', hours, minutes);
  return t('minutes', minutes);
}

export function linkSpeed(bitsPerSecond: number): string {
  return bitsPerSecond >= 1e9 ? `${+(bitsPerSecond / 1e9).toFixed(1)} Gbps` : `${Math.round(bitsPerSecond / 1e6)} Mbps`;
}

export function escapeHtml(text: string): string {
  return text.replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c]!);
}

export function element<T extends HTMLElement = HTMLElement>(html: string): T {
  const template = document.createElement('template');
  template.innerHTML = html.trim();
  return template.content.firstElementChild as T;
}
