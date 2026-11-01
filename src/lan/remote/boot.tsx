import { StrictMode } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { App } from './App';

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
  const root = createRoot(container);
  root.render(
    <StrictMode>
      <App initialCode={initialCode} />
    </StrictMode>,
  );
  return root;
}
