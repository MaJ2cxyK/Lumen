import { format, formatDistanceToNow } from 'date-fns';
import { enUS, zhCN } from 'date-fns/locale';
import { currentLang } from './i18n';

function dateLocale() {
  return currentLang() === 'en' ? enUS : zhCN;
}

export function microsToUsd(micros: number | null | undefined): string {
  const value = (micros ?? 0) / 1_000_000;
  const decimals = Math.abs(value) >= 1 ? 2 : 4;
  return `$${value.toFixed(decimals)}`;
}

export function formatTokens(n: number | null | undefined): string {
  const value = n ?? 0;
  if (Math.abs(value) >= 1_000_000_000) return `${(value / 1_000_000_000).toFixed(2)}B`;
  if (Math.abs(value) >= 1_000_000) return `${(value / 1_000_000).toFixed(2)}M`;
  if (Math.abs(value) >= 1_000) return `${(value / 1_000).toFixed(1)}K`;
  return String(value);
}

export function formatNumber(n: number | null | undefined): string {
  return (n ?? 0).toLocaleString('en-US');
}

export function formatBytes(bytes: number | null | undefined): string {
  const value = Math.max(0, bytes ?? 0);
  if (value < 1024) return `${value} B`;
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KB`;
  return `${(value / 1024 / 1024).toFixed(1)} MB`;
}

export function formatDateTime(ms?: number | null): string {
  if (!ms) return '—';
  return format(new Date(ms), 'MM-dd HH:mm', { locale: dateLocale() });
}

export function formatFullDateTime(ms?: number | null): string {
  if (!ms) return '—';
  return format(new Date(ms), 'yyyy-MM-dd HH:mm:ss', { locale: dateLocale() });
}

export function formatTime(ms?: number | null): string {
  if (!ms) return '—';
  return format(new Date(ms), 'HH:mm', { locale: dateLocale() });
}

export function formatDay(ms?: number | null): string {
  if (!ms) return '—';
  return format(new Date(ms), 'yyyy-MM-dd', { locale: dateLocale() });
}

export function fromNow(ms?: number | null): string {
  if (!ms) return '—';
  return formatDistanceToNow(new Date(ms), { addSuffix: true, locale: dateLocale() });
}

export function formatDuration(startMs?: number | null, endMs?: number | null): string {
  if (!startMs) return '—';
  const end = endMs ?? Date.now();
  const secs = Math.max(0, Math.round((end - startMs) / 1000));
  if (secs < 60) return `${secs}s`;
  const mins = Math.floor(secs / 60);
  if (mins < 60) return `${mins}m ${secs % 60}s`;
  return `${Math.floor(mins / 60)}h ${mins % 60}m`;
}
