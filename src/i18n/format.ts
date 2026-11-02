import { getLocale, type Locale } from './runtime';

export function formatNumber(value: number, locale: Locale = getLocale()): string {
  return new Intl.NumberFormat(locale).format(value);
}

export function formatDate(value: Date | number, locale: Locale = getLocale()): string {
  return new Intl.DateTimeFormat(locale, { dateStyle: 'medium' }).format(value);
}

export function formatDateTime(value: Date | number, locale: Locale = getLocale()): string {
  return new Intl.DateTimeFormat(locale, { dateStyle: 'medium', timeStyle: 'short' }).format(value);
}

export function formatTime(value: Date | number, locale: Locale = getLocale()): string {
  return new Intl.DateTimeFormat(locale, { timeStyle: 'short' }).format(value);
}
