import { invoke } from '@tauri-apps/api/core';

export const LANGUAGE_PREFERENCES = ['zh-CN', 'en-US', 'system'] as const;
export type LanguagePreference = (typeof LANGUAGE_PREFERENCES)[number];
export type Locale = 'zh-CN' | 'en-US';

export interface LanguagePreferences {
  preference: LanguagePreference;
  locale: Locale;
  catalog_version: string;
}

interface CommandResponse<T> {
  success: boolean;
  data?: T;
  error?: { code?: number; message?: string };
}

interface LocaleState extends LanguagePreferences {
  initialized: boolean;
}

const DEFAULT_STATE: LocaleState = {
  preference: 'zh-CN',
  locale: 'zh-CN',
  catalog_version: 'v1',
  initialized: false,
};

let state: LocaleState = { ...DEFAULT_STATE };
let initPromise: Promise<void> | null = null;
let writePromise: Promise<void> | null = null;
const listeners = new Set<() => void>();

function emit(): void {
  if (typeof document !== 'undefined') document.documentElement.setAttribute('data-locale', state.locale);
  for (const listener of listeners) listener();
}

function isPreference(value: unknown): value is LanguagePreference {
  return typeof value === 'string' && (LANGUAGE_PREFERENCES as readonly string[]).includes(value);
}

function isLocale(value: unknown): value is Locale {
  return value === 'zh-CN' || value === 'en-US';
}

function resolveBrowserLocale(): Locale {
  const language = typeof navigator === 'undefined' ? '' : navigator.language.toLowerCase();
  return language === 'zh' || language.startsWith('zh-') ? 'zh-CN' : 'en-US';
}

function parseResponse(raw: unknown): LanguagePreferences {
  const response = raw as CommandResponse<unknown> | null;
  if (!response?.success || !response.data || typeof response.data !== 'object') {
    const code = response?.error?.code;
    throw Object.assign(new Error('language preference command failed'), code === undefined ? {} : { code });
  }
  const data = response.data as Partial<LanguagePreferences>;
  if (!isPreference(data.preference) || !isLocale(data.locale) || typeof data.catalog_version !== 'string') {
    throw new Error('language preference command returned invalid data');
  }
  return { preference: data.preference, locale: data.locale, catalog_version: data.catalog_version };
}

export function getLanguagePreference(): LanguagePreference {
  return state.preference;
}

export function getLocale(): Locale {
  return state.locale;
}

export function subscribeLocale(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function getLocaleSnapshot(): LocaleState {
  return state;
}

/** Test/support boundary for a new WebView lifecycle. Not part of the UI API. */
export function resetLocaleForTesting(): void {
  state = { ...DEFAULT_STATE };
  initPromise = null;
  writePromise = null;
  emit();
}

export function setLocaleForTesting(locale: Locale): void {
  state = { preference: locale, locale, catalog_version: 'v1', initialized: true };
  initPromise = Promise.resolve();
  emit();
}

export async function initLocale(): Promise<void> {
  initPromise ??= (async () => {
    try {
      const result = parseResponse(await invoke<CommandResponse<unknown>>('get_language_preferences'));
      state = { ...result, initialized: true };
      if (typeof document !== 'undefined') document.documentElement.setAttribute('data-locale', state.locale);
      emit();
    } catch {
      // Older smoke hosts and old development harnesses do not register the
      // new command. The deterministic default keeps them usable.
      state = { ...DEFAULT_STATE, initialized: true };
      if (typeof document !== 'undefined') document.documentElement.setAttribute('data-locale', state.locale);
      emit();
    }
  })();
  return initPromise;
}

export async function updateLanguage(preference: LanguagePreference): Promise<void> {
  await initLocale();
  const previous = state;
  // Apply the selected preference immediately. The host response remains the
  // source of truth for `system`, whose resolved locale is platform-dependent.
  state = {
    ...state,
    preference,
    locale: preference === 'zh-CN' ? 'zh-CN' : preference === 'en-US' ? 'en-US' : resolveBrowserLocale(),
  };
  emit();
  const request = async () => {
    try {
      const result = parseResponse(await invoke<CommandResponse<unknown>>('set_language_preferences', { language: preference }));
      state = { ...result, initialized: true };
      emit();
    } catch (error) {
      state = previous;
      emit();
      throw error;
    }
  };
  writePromise = (writePromise ?? Promise.resolve()).then(request, request);
  return writePromise;
}
