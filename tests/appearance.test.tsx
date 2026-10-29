// V1-FR-005: AC01/AC03 (apply), AC21 (read side, plus the write queue), AC25 (auto).
// Every host call in this file is a mocked `invoke`: it proves the frontend state machine and the wire
// shapes (command names, patch-only arguments, envelope unwrapping) and nothing about persistence.
// Persistence evidence (AC02/AC04, AC21 host side) comes from the host side:
//   cargo test -p dgos-db appearance, cargo test -p dgos --lib appearance, and the real-WebView
//   src-tauri/src/smoke/appearance_host.rs (npm run smoke:tauri).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import {
  DEFAULTS,
  createAppearanceController,
  disposeAppearance,
  getCurrentDensity,
  getCurrentTheme,
  initAppearance,
  updateAppearance,
  type Appearance,
  type AppearanceController,
} from '@dgos/ui';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
const mockedInvoke = vi.mocked(invoke);

function makeRoot(): HTMLElement {
  const root = document.createElement('html');
  root.setAttribute('data-appearance-pending', '');
  return root;
}

class FakeMedia extends EventTarget {
  matches: boolean;
  constructor(matches: boolean) {
    super();
    this.matches = matches;
  }
  set(matches: boolean) {
    this.matches = matches;
    const event = new Event('change') as Event & { matches: boolean };
    event.matches = matches;
    this.dispatchEvent(event);
  }
  listeners = 0;
  override addEventListener(...args: Parameters<EventTarget['addEventListener']>) {
    this.listeners += 1;
    super.addEventListener(...args);
  }
  override removeEventListener(...args: Parameters<EventTarget['removeEventListener']>) {
    this.listeners -= 1;
    super.removeEventListener(...args);
  }
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

const attrs = (root: HTMLElement) => ({ theme: root.getAttribute('data-theme'), density: root.getAttribute('data-density') });

let controller: AppearanceController | undefined;
afterEach(() => {
  controller?.dispose();
  controller = undefined;
  vi.useRealTimers();
});

describe('reading preferences (AC02 read side, AC21)', () => {
  it('applies the stored theme and density before the gate lifts', async () => {
    const root = makeRoot();
    controller = createAppearanceController({ root, read: async () => ({ theme: 'dark', density: 125 }), write: vi.fn() });
    expect(root.hasAttribute('data-appearance-pending')).toBe(true);
    await controller.init();
    expect(attrs(root)).toEqual({ theme: 'dark', density: '125' });
    expect(root.hasAttribute('data-appearance-pending')).toBe(false);
  });

  it.each([
    ['a rejected read', () => Promise.reject(new Error('host down'))],
    ['null data', () => Promise.resolve(null)],
    ['an unknown theme and a string density', () => Promise.resolve({ theme: 'neon', density: '125' })],
    ['an out-of-set density', () => Promise.resolve({ theme: 'dark', density: 110 })],
  ])('falls back per field on %s and still lifts the gate', async (_name, read) => {
    const root = makeRoot();
    controller = createAppearanceController({ root, read, write: vi.fn() });
    await controller.init();
    const expectedTheme = _name === 'an out-of-set density' ? 'dark' : DEFAULTS.theme;
    expect(attrs(root)).toEqual({ theme: expectedTheme, density: String(DEFAULTS.density) });
    expect(root.hasAttribute('data-appearance-pending')).toBe(false);
  });

  it('uses the defaults after 1 s without an answer and ignores a late reply', async () => {
    vi.useFakeTimers();
    const root = makeRoot();
    const late = deferred<unknown>();
    controller = createAppearanceController({ root, read: () => late.promise, write: vi.fn() });
    const started = controller.init();
    await vi.advanceTimersByTimeAsync(999);
    expect(root.hasAttribute('data-appearance-pending')).toBe(true);
    await vi.advanceTimersByTimeAsync(1);
    await started;
    expect(attrs(root)).toEqual({ theme: 'light', density: '100' });
    expect(root.hasAttribute('data-appearance-pending')).toBe(false);
    late.resolve({ theme: 'dark', density: 175 });
    await vi.advanceTimersByTimeAsync(0);
    expect(attrs(root)).toEqual({ theme: 'light', density: '100' });
  });

  it('reads once however often init is called', async () => {
    const read = vi.fn(async () => ({ theme: 'light', density: 100 }));
    controller = createAppearanceController({ root: makeRoot(), read, write: vi.fn() });
    await Promise.all([controller.init(), controller.init()]);
    expect(read).toHaveBeenCalledTimes(1);
  });
});

describe('switching (AC01, AC03)', () => {
  it('writes only attributes: theme and density change the root at once, before the host answers', async () => {
    const root = makeRoot();
    const write = deferred<unknown>();
    controller = createAppearanceController({ root, read: async () => DEFAULTS, write: () => write.promise });
    await controller.init();
    const saving = controller.update({ theme: 'dark', density: 150 });
    await Promise.resolve();
    expect(attrs(root)).toEqual({ theme: 'dark', density: '150' });
    expect(root.getAttribute('style')).toBeNull();
    write.resolve({ theme: 'dark', density: 150 });
    await saving;
    expect(attrs(root)).toEqual({ theme: 'dark', density: '150' });
  });
});

describe('write queue (AC21, mocked host)', () => {
  const settled = async () => {
    for (let i = 0; i < 5; i += 1) await Promise.resolve();
  };

  it('one write in flight; changes made meanwhile merge into one follow-up patch', async () => {
    const root = makeRoot();
    const first = deferred<Appearance>();
    const write = vi
      .fn<(patch: Partial<Appearance>) => Promise<unknown>>()
      .mockReturnValueOnce(first.promise)
      .mockResolvedValueOnce({ theme: 'light', density: 150 });
    controller = createAppearanceController({ root, read: async () => DEFAULTS, write });
    await controller.init();

    const done = controller.update({ theme: 'dark' });
    await settled();
    void controller.update({ density: 125 });
    void controller.update({ theme: 'light' });
    void controller.update({ density: 150 });
    await settled();
    expect(write).toHaveBeenCalledTimes(1);

    first.resolve({ theme: 'dark', density: 100 });
    await done;
    expect(write.mock.calls).toEqual([[{ theme: 'dark' }], [{ theme: 'light', density: 150 }]]);
    expect(attrs(root)).toEqual({ theme: 'light', density: '150' });
  });

  it('ten quick theme switches end on the last choice with at most two writes', async () => {
    const root = makeRoot();
    const first = deferred<Appearance>();
    // A host stores what it is given and answers with the complete preferences.
    const write = vi
      .fn<(patch: Partial<Appearance>) => Promise<unknown>>()
      .mockReturnValueOnce(first.promise)
      .mockImplementation(async (patch) => ({ ...DEFAULTS, ...patch }));
    controller = createAppearanceController({ root, read: async () => DEFAULTS, write });
    await controller.init();
    const pending: Promise<void>[] = [];
    for (let i = 0; i < 10; i += 1) {
      pending.push(controller.update({ theme: i % 2 === 0 ? 'dark' : 'light' }));
      await settled();
    }
    first.resolve({ theme: 'dark', density: 100 });
    await Promise.all(pending);
    expect(write.mock.calls.length).toBeLessThanOrEqual(2);
    expect(attrs(root).theme).toBe('light');
  });

  it('a failed write rolls back to the last confirmed value, drops the queued patch and reports once', async () => {
    const root = makeRoot();
    const onWriteError = vi.fn();
    const first = deferred<Appearance>();
    const write = vi.fn<(patch: Partial<Appearance>) => Promise<unknown>>().mockReturnValueOnce(first.promise);
    controller = createAppearanceController({ root, read: async () => ({ theme: 'dark', density: 125 }), write, onWriteError });
    await controller.init();

    const done = controller.update({ theme: 'light' });
    await settled();
    void controller.update({ density: 175 });
    await settled();
    expect(attrs(root)).toEqual({ theme: 'light', density: '175' });

    first.reject(new Error('disk full'));
    await done;
    expect(write).toHaveBeenCalledTimes(1);
    expect(attrs(root)).toEqual({ theme: 'dark', density: '125' });
    expect(onWriteError).toHaveBeenCalledTimes(1);
  });

  it('after a failure the next change writes again and a success becomes the new rollback point', async () => {
    const root = makeRoot();
    const write = vi
      .fn<(patch: Partial<Appearance>) => Promise<unknown>>()
      .mockRejectedValueOnce(new Error('boom'))
      .mockResolvedValueOnce({ theme: 'dark', density: 100 })
      .mockRejectedValueOnce(new Error('boom again'));
    controller = createAppearanceController({ root, read: async () => DEFAULTS, write, onWriteError: vi.fn() });
    await controller.init();
    await controller.update({ theme: 'dark' });
    expect(attrs(root).theme).toBe('light');
    await controller.update({ theme: 'dark' });
    expect(attrs(root).theme).toBe('dark');
    await controller.update({ density: 175 });
    expect(attrs(root)).toEqual({ theme: 'dark', density: '100' });
  });

  it.each([
    ['an incomplete reply', { theme: 'dark' }],
    ['an unknown theme', { theme: 'neon', density: 100 }],
    ['a string density', { theme: 'dark', density: '100' }],
  ])('treats %s as a failed write, not as defaults', async (_name, reply) => {
    const root = makeRoot();
    const onWriteError = vi.fn();
    controller = createAppearanceController({ root, read: async () => DEFAULTS, write: async () => reply, onWriteError });
    await controller.init();
    await controller.update({ theme: 'dark' });
    expect(attrs(root).theme).toBe('light');
    expect(onWriteError).toHaveBeenCalledTimes(1);
  });

  it('an update made before init finishes is not overwritten by the late read', async () => {
    const root = makeRoot();
    const read = deferred<unknown>();
    controller = createAppearanceController({
      root,
      read: () => read.promise,
      write: async () => ({ theme: 'dark', density: 100 }),
    });
    const early = controller.update({ theme: 'dark' });
    read.resolve({ theme: 'light', density: 100 });
    await early;
    expect(attrs(root).theme).toBe('dark');
  });
});

describe('auto follows the system only while selected (AC25)', () => {
  it('tracks the OS under auto, ignores it under light, and releases the listener on dispose', async () => {
    const root = makeRoot();
    const media = new FakeMedia(true);
    controller = createAppearanceController({
      root,
      read: async () => ({ theme: 'auto', density: 100 }),
      write: async (patch) => ({ theme: 'light', density: 100, ...patch }),
      matchMedia: () => media as unknown as MediaQueryList,
    });
    await controller.init();
    expect(attrs(root).theme).toBe('dark');
    expect(controller.getTheme()).toBe('auto');
    expect(controller.getResolvedTheme()).toBe('dark');

    media.set(false);
    expect(attrs(root).theme).toBe('light');

    await controller.update({ theme: 'light' });
    media.set(true);
    expect(attrs(root).theme).toBe('light');

    await controller.update({ theme: 'auto' });
    expect(attrs(root).theme).toBe('dark');

    expect(media.listeners).toBe(1);
    controller.dispose();
    expect(media.listeners).toBe(0);
  });

  it('works without matchMedia (treated as a light system)', async () => {
    const root = makeRoot();
    controller = createAppearanceController({ root, read: async () => ({ theme: 'auto', density: 100 }), write: vi.fn() });
    await controller.init();
    expect(attrs(root).theme).toBe('light');
  });
});

describe('application controller and host contract', () => {
  beforeEach(() => {
    document.documentElement.setAttribute('data-appearance-pending', '');
    document.documentElement.removeAttribute('data-theme');
    document.documentElement.removeAttribute('data-density');
  });
  afterEach(() => disposeAppearance());

  it('reads through get_appearance_preferences and unwraps the command envelope', async () => {
    mockedInvoke.mockResolvedValueOnce({ success: true, data: { theme: 'dark', density: 150 } });
    await initAppearance();
    expect(mockedInvoke).toHaveBeenCalledWith('get_appearance_preferences');
    expect(attrs(document.documentElement)).toEqual({ theme: 'dark', density: '150' });
    expect(getCurrentTheme()).toBe('dark');
    expect(getCurrentDensity()).toBe(150);
  });

  it.each([
    ['success:false', { success: false, error: { code: 1000000, message: 'x' } }],
    ['an unregistered command', undefined],
  ])('uses the defaults on %s', async (_name, reply) => {
    if (reply) mockedInvoke.mockResolvedValueOnce(reply);
    else mockedInvoke.mockRejectedValueOnce(new Error('command get_appearance_preferences not found'));
    await initAppearance();
    expect(attrs(document.documentElement)).toEqual({ theme: 'light', density: '100' });
    expect(document.documentElement.hasAttribute('data-appearance-pending')).toBe(false);
  });

  it('saves through set_appearance_preferences with only the patch (no user id) and treats success:false as failure', async () => {
    mockedInvoke.mockResolvedValueOnce({ success: true, data: { theme: 'light', density: 100 } });
    await initAppearance();

    mockedInvoke.mockResolvedValueOnce({ success: true, data: { theme: 'dark', density: 100 } });
    await updateAppearance({ theme: 'dark' });
    expect(mockedInvoke).toHaveBeenLastCalledWith('set_appearance_preferences', { patch: { theme: 'dark' } });
    expect(getCurrentTheme()).toBe('dark');

    mockedInvoke.mockResolvedValueOnce({ success: false, error: { code: 1005001 } });
    await updateAppearance({ theme: 'light' });
    expect(getCurrentTheme()).toBe('dark');
  });

  it('rolls density back when the host rejects the value with 1005006, and when the call itself rejects', async () => {
    mockedInvoke.mockResolvedValueOnce({ success: true, data: { theme: 'light', density: 125 } });
    await initAppearance();
    expect(getCurrentDensity()).toBe(125);

    mockedInvoke.mockResolvedValueOnce({ success: false, error: { code: 1005006, message: 'Unsupported interface density' } });
    await updateAppearance({ density: 150 });
    expect(getCurrentDensity()).toBe(125);
    expect(attrs(document.documentElement).density).toBe('125');

    mockedInvoke.mockRejectedValueOnce(new Error('ipc closed'));
    await updateAppearance({ density: 175 });
    expect(getCurrentDensity()).toBe(125);
  });
});
