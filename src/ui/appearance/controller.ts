import {
  DEFAULTS,
  parseConfirmed,
  sanitize,
  type Appearance,
  type Density,
  type ResolvedTheme,
  type Theme,
} from './model';

const DARK_QUERY = '(prefers-color-scheme: dark)';
const PENDING_ATTRIBUTE = 'data-appearance-pending';

export interface AppearanceDeps {
  /** The element that carries data-theme / data-density. Only attributes are written (D068 R1). */
  root: HTMLElement;
  /** get_appearance_preferences. Resolves the raw data; rejects on any failure. */
  read: () => Promise<unknown>;
  /** set_appearance_preferences. Resolves the host's complete preferences; rejects on any failure. */
  write: (patch: Partial<Appearance>) => Promise<unknown>;
  matchMedia?: (query: string) => MediaQueryList;
  /** D065: the host must answer within this time or the defaults are used. */
  readTimeoutMs?: number;
  /** Called once per failed write, after the interface has rolled back (AC21). */
  onWriteError?: (error: unknown) => void;
}

export interface AppearanceController {
  init(): Promise<void>;
  /** Applies at once, then saves. Resolves when the write queue is idle. */
  update(patch: Partial<Appearance>): Promise<void>;
  dispose(): void;
  getTheme(): Theme;
  getResolvedTheme(): ResolvedTheme;
  getDensity(): Density;
}

function withTimeout<T>(promise: Promise<T>, ms: number): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('timeout')), ms);
    promise.then(
      (value) => {
        clearTimeout(timer);
        resolve(value);
      },
      (error) => {
        clearTimeout(timer);
        reject(error);
      },
    );
  });
}

export function createAppearanceController(deps: AppearanceDeps): AppearanceController {
  const { root, read, write, onWriteError } = deps;
  const readTimeoutMs = deps.readTimeoutMs ?? 1000;

  // B05-02 B05-08: confirmed is what the host saved, applied is what the interface shows.
  let confirmed: Appearance = { ...DEFAULTS };
  let applied: Appearance = { ...DEFAULTS };
  let systemTheme: ResolvedTheme = 'light';
  let pendingPatch: Partial<Appearance> | null = null; // B05-23
  let drain: Promise<void> | null = null; // one write in flight at a time
  let initPromise: Promise<void> | null = null;
  let disposeSystemListener: (() => void) | null = null;
  let disposed = false;

  // B05-04
  function resolveTheme(theme: Theme): ResolvedTheme {
    return theme === 'auto' ? systemTheme : theme;
  }

  // B05-05 B05-10: styles come from CSS attribute selectors, not from here.
  function render(): void {
    root.setAttribute('data-theme', resolveTheme(applied.theme));
    root.setAttribute('data-density', String(applied.density));
  }

  // B05-26: the OS appearance only matters while the preference is auto.
  function subscribeSystemTheme(): void {
    const query = deps.matchMedia?.(DARK_QUERY);
    if (!query) return;
    systemTheme = query.matches ? 'dark' : 'light';
    const onChange = (event: MediaQueryListEvent) => {
      systemTheme = event.matches ? 'dark' : 'light';
      if (applied.theme === 'auto') render();
    };
    query.addEventListener('change', onChange);
    disposeSystemListener = () => query.removeEventListener('change', onChange);
  }

  // B05-03 B05-09 B05-25
  function init(): Promise<void> {
    initPromise ??= (async () => {
      try {
        let stored: unknown = null;
        try {
          stored = await withTimeout(read(), readTimeoutMs);
        } catch {
          stored = null; // B05-22: unreadable, slow or failed -> defaults, never an error
        }
        confirmed = sanitize(stored);
        applied = { ...confirmed };
        if (!disposed) subscribeSystemTheme();
        render();
      } finally {
        root.removeAttribute(PENDING_ATTRIBUTE);
      }
    })();
    return initPromise;
  }

  async function runDrain(): Promise<void> {
    while (pendingPatch !== null) {
      const next = pendingPatch;
      pendingPatch = null;
      try {
        confirmed = parseConfirmed(await write(next));
      } catch (error) {
        // B05-24: drop whatever was queued and go back to the last value the host confirmed.
        pendingPatch = null;
        applied = { ...confirmed };
        render();
        onWriteError?.(error);
        return;
      }
    }
    applied = { ...confirmed };
    render();
  }

  // B05-06 B05-11: apply first, then save; changes made while a write is in flight merge into one patch.
  async function update(patch: Partial<Appearance>): Promise<void> {
    await init();
    applied = { ...applied, ...patch };
    render();
    pendingPatch = { ...pendingPatch, ...patch };
    drain ??= runDrain().finally(() => {
      drain = null;
    });
    return drain;
  }

  return {
    init,
    update,
    dispose() {
      disposed = true;
      disposeSystemListener?.();
      disposeSystemListener = null;
    },
    getTheme: () => applied.theme,
    getResolvedTheme: () => resolveTheme(applied.theme),
    getDensity: () => applied.density, // B05-12
  };
}
