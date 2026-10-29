// AC08, AC09, AC17 (FR-005 Phase 5): Toast of `@dgos/ui`.
//
// What this file can and cannot prove (jsdom, fake timers, no layout, no animation engine, no screen reader):
//  - Proven here: type -> role/icon/class, plain-text rendering, the 3-visible cap and queue order, the 3 s / 5 s
//    timers, the 250 ms fade-then-remove sequence, pause on hover and on focus-within with the remaining time kept,
//    and that the width/wrapping/animation class literals are real Tailwind utilities.
//  - NOT proven here, left to a real WebView (Phase 8): the rendered width (300-400px), that long text wraps inside
//    it, the slide-in and fade animations, the 80px/20px offsets, stacking above Modal and Select, and what
//    VoiceOver announces for `role="status"` / `role="alert"` regions that appear after load.
//    jsdom fires no `animationend`, so removal is timed (FADE_OUT_MS) rather than driven by the animation.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { ToastViewport, clearToasts, toast } from '@dgos/ui';
import { FADE_OUT_MS, MAX_VISIBLE } from '../src/ui/feedback/toastStore';
import { compileUi, contrast, themeTokens } from './helpers/designTokens';

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  act(() => clearToasts());
  cleanup();
  vi.useRealTimers();
});

const advance = (ms: number) => act(() => void vi.advanceTimersByTime(ms));

/** Every toast on screen, in order. */
const shown = () => [...document.querySelectorAll('[data-toast-type]')] as HTMLElement[];
const texts = () => shown().map((el) => el.querySelector('p')?.textContent);

describe('Toast display (AC08)', () => {
  const TYPES = ['success', 'error', 'warning', 'info'] as const;

  it('renders nothing while there are no toasts', () => {
    render(<ToastViewport />);
    expect(document.body.querySelector('section')).toBeNull();
  });

  it('each type renders its own role, icon and colour classes', () => {
    render(<ToastViewport />);
    const classes = {
      success: ['border-success', 'bg-success-bg', 'text-success-text'],
      error: ['border-error', 'bg-error-bg', 'text-error-text'],
      warning: ['border-warning', 'bg-warning-bg', 'text-warning-text'],
      info: ['border-info', 'bg-info-bg', 'text-info-text'],
    };
    for (const type of TYPES) {
      act(() => void toast[type](`${type} message`));
      const item = shown().find((el) => el.dataset.toastType === type)!;
      expect(item, type).toBeTruthy();
      expect(item.getAttribute('role'), type).toBe(type === 'error' ? 'alert' : 'status');
      expect(item.querySelector(`svg[data-icon="${type}"]`), type).not.toBeNull();
      for (const name of classes[type]) expect(item.className.split(/\s+/), `${type} ${name}`).toContain(name);
      act(() => clearToasts());
    }
  });

  it('the icon is decorative and the type is carried by role and text, not by colour alone', () => {
    render(<ToastViewport />);
    act(() => void toast.error('Provider 不存在'));
    const alert = screen.getByRole('alert');
    expect(alert.querySelector('svg[data-icon="error"]')?.getAttribute('aria-hidden')).toBe('true');
    expect(within(alert).getByText('Provider 不存在')).toBeTruthy();
  });

  it('shows the description and an action; the action runs and closes the toast', () => {
    const onClick = vi.fn();
    render(<ToastViewport />);
    act(() => void toast.error('Provider 不存在', { description: '请检查 Provider ID', action: { label: '查看列表', onClick } }));
    const alert = screen.getByRole('alert');
    expect(within(alert).getByText('请检查 Provider ID')).toBeTruthy();
    fireEvent.click(within(alert).getByRole('button', { name: '查看列表' }));
    expect(onClick).toHaveBeenCalledTimes(1);
    expect(screen.getByRole('alert').getAttribute('data-state')).toBe('closing');
    advance(FADE_OUT_MS);
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('the close button starts the fade and removes the toast', () => {
    render(<ToastViewport />);
    act(() => void toast.info('syncing'));
    fireEvent.click(screen.getByRole('button', { name: '关闭通知' }));
    expect(screen.getByRole('status').getAttribute('data-state')).toBe('closing');
    advance(FADE_OUT_MS - 1);
    expect(screen.queryByRole('status')).not.toBeNull();
    advance(1);
    expect(screen.queryByRole('status')).toBeNull();
  });

  it('toast.dismiss removes one toast immediately, shown or queued', () => {
    render(<ToastViewport />);
    let ids: string[] = [];
    act(() => {
      ids = [1, 2, 3, 4, 5].map((n) => toast.info(`m${n}`));
    });
    act(() => toast.dismiss(ids[4]));
    act(() => toast.dismiss(ids[0]));
    expect(texts()).toEqual(['m2', 'm3', 'm4']);
  });

  it('each call returns a distinct id', () => {
    const ids = [toast.success('a'), toast.success('a'), toast.error('a')];
    expect(new Set(ids).size).toBe(3);
  });
});

describe('Toast auto dismiss (AC09)', () => {
  it('a success toast fades after 3 s and is gone 250 ms later', () => {
    render(<ToastViewport />);
    act(() => void toast.success('saved'));
    advance(2999);
    expect(screen.getByRole('status').getAttribute('data-state')).toBe('open');
    advance(1);
    expect(screen.getByRole('status').getAttribute('data-state')).toBe('closing');
    advance(FADE_OUT_MS - 1);
    expect(screen.queryByRole('status')).not.toBeNull();
    advance(1);
    expect(screen.queryByRole('status')).toBeNull();
  });

  it('an error toast lasts 5 s', () => {
    render(<ToastViewport />);
    act(() => void toast.error('failed'));
    advance(4999);
    expect(screen.getByRole('alert').getAttribute('data-state')).toBe('open');
    advance(1);
    expect(screen.getByRole('alert').getAttribute('data-state')).toBe('closing');
    advance(FADE_OUT_MS);
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('warning and info use the 3 s default too', () => {
    render(<ToastViewport />);
    act(() => {
      toast.warning('w');
      toast.info('i');
    });
    advance(3000);
    expect(shown().map((el) => el.dataset.state)).toEqual(['closing', 'closing']);
  });

  it('options.duration overrides the default, and a bad duration falls back to it', () => {
    render(<ToastViewport />);
    act(() => {
      toast.success('long', { duration: 8000 });
      toast.success('zero', { duration: 0 });
      toast.success('nan', { duration: Number.NaN });
      toast.success('neg', { duration: -5 });
    });
    // The cap is 3, so `neg` waits in the queue behind `long`, `zero` and `nan`.
    advance(3000);
    expect(shown().map((el) => [el.querySelector('p')?.textContent, el.dataset.state])).toEqual([
      ['long', 'open'],
      ['zero', 'closing'],
      ['nan', 'closing'],
    ]);
    advance(FADE_OUT_MS);
    expect(texts()).toEqual(['long', 'neg']);
    // `neg` starts its own default 3 s only now that it is on screen.
    advance(2999);
    expect(shown().map((el) => el.dataset.state)).toEqual(['open', 'open']);
    advance(1);
    expect(shown().map((el) => el.dataset.state)).toEqual(['open', 'closing']);
    advance(FADE_OUT_MS);
    expect(texts()).toEqual(['long']);
    // t = 3000 + 250 + 3000 + 250 = 6500 now; `long` started closing at 8000.
    advance(1500);
    expect(shown().map((el) => el.dataset.state)).toEqual(['closing']);
    advance(FADE_OUT_MS);
    expect(texts()).toEqual([]);
  });

  it('hovering stops the timer, and leaving resumes it with the time that was left', () => {
    render(<ToastViewport />);
    act(() => void toast.success('saved'));
    advance(1000);
    const item = screen.getByRole('status');
    fireEvent.mouseEnter(item);
    advance(60000);
    expect(screen.getByRole('status').getAttribute('data-state')).toBe('open');
    fireEvent.mouseLeave(item);
    advance(1999);
    expect(screen.getByRole('status').getAttribute('data-state')).toBe('open');
    advance(1);
    expect(screen.getByRole('status').getAttribute('data-state')).toBe('closing');
  });

  it('keyboard focus inside the toast stops the timer, and blur resumes it', () => {
    render(<ToastViewport />);
    act(() => void toast.error('failed', { action: { label: 'retry', onClick: () => undefined } }));
    advance(2000);
    const retry = screen.getByRole('button', { name: 'retry' });
    act(() => retry.focus());
    advance(60000);
    expect(screen.getByRole('alert').getAttribute('data-state')).toBe('open');
    act(() => retry.blur());
    advance(2999);
    expect(screen.getByRole('alert').getAttribute('data-state')).toBe('open');
    advance(1);
    expect(screen.getByRole('alert').getAttribute('data-state')).toBe('closing');
  });

  it('focus moving between the toast\'s own buttons does not resume the timer', () => {
    render(<ToastViewport />);
    act(() => void toast.error('failed', { action: { label: 'retry', onClick: () => undefined } }));
    const retry = screen.getByRole('button', { name: 'retry' });
    const close = screen.getByRole('button', { name: '关闭通知' });
    act(() => retry.focus());
    act(() => close.focus());
    advance(60000);
    expect(screen.getByRole('alert').getAttribute('data-state')).toBe('open');
  });

  it('pausing does not restart the countdown from the full duration', () => {
    render(<ToastViewport />);
    act(() => void toast.success('saved'));
    const item = screen.getByRole('status');
    advance(2500);
    fireEvent.mouseEnter(item);
    fireEvent.mouseLeave(item);
    advance(499);
    expect(screen.getByRole('status').getAttribute('data-state')).toBe('open');
    advance(1);
    expect(screen.getByRole('status').getAttribute('data-state')).toBe('closing');
  });

  it('a toast that is hovered while it fades is still removed', () => {
    render(<ToastViewport />);
    act(() => void toast.success('saved'));
    advance(3000);
    fireEvent.mouseEnter(screen.getByRole('status'));
    advance(FADE_OUT_MS);
    expect(screen.queryByRole('status')).toBeNull();
  });
});

describe('Toast queue and text safety (AC17)', () => {
  it('shows 3 of 10 and queues the rest in order', () => {
    render(<ToastViewport />);
    act(() => {
      for (let n = 1; n <= 10; n += 1) toast.info(`m${n}`, { duration: 1000 });
    });
    expect(MAX_VISIBLE).toBe(3);
    expect(texts()).toEqual(['m1', 'm2', 'm3']);
  });

  it('promotes queued toasts as earlier ones leave, until all 10 have been shown once', () => {
    render(<ToastViewport />);
    act(() => {
      for (let n = 1; n <= 10; n += 1) toast.info(`m${n}`, { duration: 1000 });
    });
    const seen: string[] = [...texts()] as string[];
    // Each round is two steps: the timers fire (fade starts), then the fade ends (toast leaves, next one enters).
    for (let round = 0; round < 4; round += 1) {
      advance(1000);
      advance(FADE_OUT_MS);
      expect(shown().length).toBeLessThanOrEqual(3);
      for (const text of texts()) if (!seen.includes(text as string)) seen.push(text as string);
    }
    expect(seen).toEqual(['m1', 'm2', 'm3', 'm4', 'm5', 'm6', 'm7', 'm8', 'm9', 'm10']);
    expect(shown()).toHaveLength(0);
  });

  it('a queued toast does not start its timer until it is on screen', () => {
    render(<ToastViewport />);
    act(() => {
      toast.info('a', { duration: 1000 });
      toast.info('b', { duration: 1000 });
      toast.info('c', { duration: 1000 });
      toast.info('d', { duration: 3000 });
    });
    advance(1000);
    advance(FADE_OUT_MS);
    expect(texts()).toEqual(['d']);
    advance(2999);
    expect(screen.getByRole('status').getAttribute('data-state')).toBe('open');
    advance(1);
    expect(screen.getByRole('status').getAttribute('data-state')).toBe('closing');
  });

  it('with no viewport mounted, toasts only queue; mounting one then shows them', () => {
    act(() => void toast.info('early'));
    expect(shown()).toHaveLength(0);
    render(<ToastViewport />);
    expect(texts()).toEqual(['early']);
  });

  it('HTML in the message and description is shown as text', () => {
    render(<ToastViewport />);
    const message = '<img src=x onerror=alert(1)> <script>alert(2)</script>';
    const description = '<b>bold</b> & <i>italic</i>';
    act(() => void toast.error(message, { description }));
    const alert = screen.getByRole('alert');
    expect(within(alert).getByText(message)).toBeTruthy();
    expect(within(alert).getByText(description)).toBeTruthy();
    expect(alert.querySelector('img, script, b, i')).toBeNull();
  });

  it('non-string input is coerced to text, never rendered as an element', () => {
    render(<ToastViewport />);
    const element = { $$typeof: Symbol.for('react.element'), type: 'b', props: { children: 'x' } };
    act(() => void toast.info(element as unknown as string));
    act(() => void toast.info(undefined as unknown as string));
    expect(document.querySelector('[data-toast-type] b')).toBeNull();
    expect(texts()).toEqual(['[object Object]', '']);
  });

  it('a long message wraps inside a width-capped card instead of widening it', () => {
    render(<ToastViewport />);
    const long = '很长的提示'.repeat(40) + ' ' + 'x'.repeat(300);
    act(() => void toast.info(long));
    const item = screen.getByRole('status');
    expect(within(item).getByText(long)).toBeTruthy();
    const classes = item.className.split(/\s+/);
    expect(classes).toContain('max-w-100');
    expect(classes).toContain('min-w-75');
    expect(item.querySelector('.wrap-break-word')).not.toBeNull();
    expect(item.querySelector('.min-w-0')).not.toBeNull();
  });

  it('the viewport is a labelled region, does not capture clicks around the cards, and holds at most 3', () => {
    render(<ToastViewport />);
    act(() => {
      for (let n = 0; n < 6; n += 1) toast.info(`m${n}`);
    });
    const region = screen.getByRole('region', { name: '通知' });
    expect(region.className.split(/\s+/)).toEqual(expect.arrayContaining(['pointer-events-none', 'fixed', 'top-20', 'right-5']));
    // Toasts paint above every other global layer (dialogs, and the Select/Tooltip popovers above them).
    const toastLayer = Number(themeTokens().light.get(`--z-index-${/(?:^|\s)z-([a-z]+)(?=\s|$)/.exec(region.className)?.[1]}`));
    expect(toastLayer).toBeGreaterThan(Number(themeTokens().light.get('--z-index-popover')));
    expect(within(region).getAllByRole('status')).toHaveLength(3);
    expect(shown()[0].className.split(/\s+/)).toContain('pointer-events-auto');
  });
});

describe('Toast tokens and class literals (D066, D069)', () => {
  it('every type\'s text reads on its tinted background in both themes (>= 4.5:1), and its border on the page (>= 3:1)', () => {
    const { light, dark } = themeTokens();
    for (const [theme, tokens] of [['light', light], ['dark', dark]] as const) {
      for (const type of ['success', 'error', 'warning', 'info']) {
        const text = tokens.get(`--color-${type}-text`)!;
        const background = tokens.get(`--color-${type}-bg`)!;
        expect(contrast(text, background), `${theme} ${type} text on bg`).toBeGreaterThanOrEqual(4.5);
        expect(contrast(tokens.get(`--color-${type}`)!, tokens.get('--color-bg-primary')!), `${theme} ${type} border`).toBeGreaterThanOrEqual(3);
      }
    }
  });

  it('the secondary action button reads on every tinted background in both themes', () => {
    const { light, dark } = themeTokens();
    for (const [theme, tokens] of [['light', light], ['dark', dark]] as const) {
      // `secondary` is bg-tertiary with text-primary; the pair is declared, the button sits on the toast tint.
      expect(contrast(tokens.get('--color-text-primary')!, tokens.get('--color-bg-tertiary')!), theme).toBeGreaterThanOrEqual(4.5);
    }
  });

  it('the utilities the viewport relies on are generated by the real Tailwind build', async () => {
    const css = await compileUi([
      'fixed', 'right-5', 'top-20', 'z-toast', 'pointer-events-none', 'pointer-events-auto', 'min-w-75', 'max-w-100',
      'animate-toast-in', 'animate-toast-out', 'wrap-break-word', 'border-success', 'bg-success-bg', 'text-success-text',
      'border-error', 'bg-error-bg', 'border-warning', 'bg-warning-bg', 'border-info', 'bg-info-bg', 'hover:opacity-hover',
    ]);
    const unit = (n: number) => `calc\\(var\\(--spacing-unit\\)\\s*\\*\\s*${n}\\)`;
    expect(css).toMatch(new RegExp(`\\.right-5\\s*\\{\\s*right:\\s*${unit(5)}`));
    expect(css).toMatch(new RegExp(`\\.top-20\\s*\\{\\s*top:\\s*${unit(20)}`));
    expect(css).toMatch(/\.z-toast\s*\{\s*z-index:\s*var\(--z-index-toast\)/);
    expect(css).toMatch(new RegExp(`\\.min-w-75\\s*\\{\\s*min-width:\\s*${unit(75)}`));
    expect(css).toMatch(new RegExp(`\\.max-w-100\\s*\\{\\s*max-width:\\s*${unit(100)}`));
    expect(css).toMatch(/--animate-toast-in:\s*toast-in var\(--duration-base\)/);
    expect(css).toMatch(/--animate-toast-out:\s*toast-out var\(--duration-base\)/);
    expect(css).toMatch(/@keyframes toast-in/);
    expect(css).toMatch(/@keyframes toast-out/);
    expect(css).toMatch(/\.bg-success-bg\s*\{\s*background-color:\s*var\(--color-success-bg\)/);
    expect(css).toMatch(/\.wrap-break-word\s*\{\s*overflow-wrap:\s*break-word/);
  });

  it('the spacing token behind the width cap is 100 x 4px at the 14px root, i.e. 400px', () => {
    const { light } = themeTokens();
    expect(light.get('--spacing-unit')).toBe('calc(4rem / 14)');
    // 100 units x 4rem/14 x 14px root = 400px; 75 units = 300px (FR-005 §8.1.1).
    expect((100 * 4) / 14 * 14).toBe(400);
    expect((75 * 4) / 14 * 14).toBe(300);
  });
});
