import { useSyncExternalStore } from 'react';
import { remoteCatalogs } from '../../i18n/remoteCatalogs';
import { formatMessage } from '../../i18n/message';
import type { Locale } from '../../i18n/runtime';

type RemoteKey = keyof typeof remoteCatalogs['en-US'];
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
  return formatMessage(remoteCatalogs[locale], key, variables);
}

export function useRemoteLocale(): { locale: Locale; t: typeof t } {
  useSyncExternalStore(subscribeRemoteLocale, () => locale, () => locale);
  return { locale, t };
}
