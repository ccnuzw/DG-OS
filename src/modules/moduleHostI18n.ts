import { getLocale, subscribeLocale, type Locale } from '../i18n/runtime';

export interface HostI18n {
  readonly locale: Locale;
  subscribe(listener: (locale: Locale) => void): () => void;
}

/** A separate scope per runtime: an abandoned factory or unloaded module cannot leak listeners. */
export function createModuleI18n(): { api: Readonly<HostI18n>; dispose: () => void } {
  const subscriptions = new Set<() => void>();
  let disposed = false;
  const api = Object.freeze({
    get locale() { return getLocale(); },
    subscribe(listener: (locale: Locale) => void): () => void {
      if (disposed || typeof listener !== 'function') return () => {};
      let previous = getLocale();
      const unsubscribe = subscribeLocale(() => {
        const next = getLocale();
        if (disposed || previous === next) return;
        previous = next;
        try { listener(next); } catch { console.warn('module locale listener failed'); }
      });
      const remove = () => { unsubscribe(); subscriptions.delete(remove); };
      subscriptions.add(remove);
      return remove;
    },
  });
  return {
    api,
    dispose() {
      disposed = true;
      for (const unsubscribe of [...subscriptions]) unsubscribe();
    },
  };
}
