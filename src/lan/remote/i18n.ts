import { useSyncExternalStore } from 'react';
import { phase2EnUS } from '../../i18n/locales/phase2.en-US';
import { phase2ZhCN } from '../../i18n/locales/phase2.zh-CN';
import type { Locale } from '../../i18n/runtime';

type RemoteKey = keyof typeof phase2EnUS;

const catalogs = { 'zh-CN': phase2ZhCN, 'en-US': phase2EnUS } as const;
let locale: Locale = 'zh-CN';
const listeners = new Set<() => void>();

export function getRemoteLocale(): Locale { return locale; }
export function setRemoteLocale(next: Locale): void {
  document.documentElement.lang = next;
  document.documentElement.setAttribute('data-locale', next);
  if (locale === next) return;
  locale = next;
  for (const listener of [...listeners]) listener();
}
export function subscribeRemoteLocale(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}
export function t(key: RemoteKey, variables?: Record<string, string | number>): string {
  const value = catalogs[locale][key] ?? key;
  return value.replace(/\{([a-zA-Z0-9_]+)\}/g, (match, name: string) => (
    variables && Object.prototype.hasOwnProperty.call(variables, name) ? String(variables[name]) : match
  ));
}

export function useRemoteLocale(): { locale: Locale; t: typeof t } {
  useSyncExternalStore(subscribeRemoteLocale, () => locale, () => locale);
  return { locale, t };
}
