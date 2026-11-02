import { useSyncExternalStore } from 'react';
import { enUS } from './locales/en-US';
import { zhCN } from './locales/zh-CN';
import { phase2EnUS } from './locales/phase2.en-US';
import { phase2ZhCN } from './locales/phase2.zh-CN';
import { detailsZh, detailsEn } from './locales/details';
import { chatZh, chatEn } from './locales/chat';
import {
  getLocaleSnapshot,
  subscribeLocale,
  getLocale,
  type Locale,
} from './runtime';

export type { LanguagePreference, Locale } from './runtime';
export { LANGUAGE_PREFERENCES } from './runtime';
export { formatDate, formatDateTime, formatNumber, formatTime } from './format';
export { getLanguagePreference, getLocale, initLocale, resetLocaleForTesting, setLocaleForTesting, updateLanguage } from './runtime';

const catalogs = {
  'zh-CN': { ...flatten(zhCN), ...phase2ZhCN, ...detailsZh, ...chatZh },
  'en-US': { ...flatten(enUS), ...phase2EnUS, ...detailsEn, ...chatEn },
} as const;
type Key = string;

function flatten(catalog: Record<string, unknown>): Record<string, string> {
  return Object.fromEntries(Object.entries(catalog).flatMap(([key, value]) => {
    if (typeof value === 'string') return [[key, value]];
    if (value && typeof value === 'object') {
      return Object.entries(value as Record<string, unknown>).filter(([, item]) => typeof item === 'string').map(([item, text]) => [`${key}.${item}`, text as string]);
    }
    return [];
  }));
}

function valueAt(catalog: Record<string, string>, key: Key): string {
  const direct = catalog[key];
  if (typeof direct === 'string') return direct;
  // Keep the missing-key marker deterministic in development and tests. Production callers use only
  // keys checked by the static catalog scanner.
  return `⟦${key}⟧`;
}

function interpolate(value: string, variables?: Record<string, string | number>): string {
  if (!variables) return value;
  return value.replace(/\{([a-zA-Z0-9_]+)\}/g, (match, name: string) => (
    Object.prototype.hasOwnProperty.call(variables, name) ? String(variables[name]) : match
  ));
}

export function translate(locale: Locale, key: Key, variables?: Record<string, string | number>): string {
  return interpolate(valueAt(catalogs[locale], key), variables);
}

/** For non-React callbacks. Components also subscribe with useLocale(). */
export function t(key: Key, variables?: Record<string, string | number>): string {
  return translate(getLocale(), key, variables);
}

export function useLocale() {
  const snapshot = useSyncExternalStore(subscribeLocale, getLocaleSnapshot, getLocaleSnapshot);
  const locale = snapshot.locale;
  return {
    locale,
    preference: snapshot.preference,
    initialized: snapshot.initialized,
    t: (key: Key, variables?: Record<string, string | number>) => translate(locale, key, variables),
  };
}

export type TranslationKey = Key;
