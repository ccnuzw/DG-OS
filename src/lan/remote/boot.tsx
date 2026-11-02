import { StrictMode, useEffect } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { App } from './App';
import { setRemoteLocale } from './i18n';

export async function loadRemoteLocale(signal: AbortSignal): Promise<void> {
  try {
    const response = await fetch('/api/v1/lan/bootstrap', { credentials: 'same-origin', cache: 'no-store', signal });
    if (!response.ok) return;
    const data: unknown = await response.json();
    if (!data || typeof data !== 'object' || !('locale' in data) || !('catalog_version' in data)) return;
    if (data.catalog_version !== 'v1' || (data.locale !== 'zh-CN' && data.locale !== 'en-US')) return;
    if (!signal.aborted) setRemoteLocale(data.locale);
  } catch { /* Keep the deterministic Chinese fallback; never render internal failure details. */ }
}

function LocaleBootstrap({ initialCode }: { initialCode: string | null }) {
  useEffect(() => {
    const controller = new AbortController();
    const timer = window.setTimeout(() => { controller.abort(); }, 3000);
    void loadRemoteLocale(controller.signal).finally(() => {
      window.clearTimeout(timer);
    });
    return () => { controller.abort(); window.clearTimeout(timer); };
  }, []);
  return <App initialCode={initialCode} />;
}

/**
 * Reads `code=` from the URL fragment and removes the fragment at once, so the credential never stays in the
 * address bar or the history. Returns null when there is no code. Runs before any request is made.
 */
export function consumeFragmentCode(): string | null {
  const params = new URLSearchParams(location.hash.replace(/^#/, ''));
  const code = params.get('code');
  if (code === null) return null;
  history.replaceState(null, '', location.pathname + location.search);
  return code;
}

/** Follows the OS light/dark setting through the attribute the token stylesheet reads. */
function applySystemTheme(): void {
  if (typeof matchMedia !== 'function') return;
  const query = matchMedia('(prefers-color-scheme: dark)');
  const apply = () => document.documentElement.setAttribute('data-theme', query.matches ? 'dark' : 'light');
  apply();
  query.addEventListener?.('change', apply);
}

export function mountLanApp(container: HTMLElement): Root {
  applySystemTheme();
  const initialCode = consumeFragmentCode();
  setRemoteLocale('zh-CN');
  const root = createRoot(container);
  root.render(
    <StrictMode>
      <LocaleBootstrap initialCode={initialCode} />
    </StrictMode>,
  );
  return root;
}
