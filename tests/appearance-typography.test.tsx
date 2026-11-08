import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { useState } from 'react';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { AppearanceTypography } from '../src/pages/AppearanceTypography';
import type { Appearance } from '../src/ui/appearance/model';
import { resetLocaleForTesting, setLocaleForTesting } from '../src/i18n/runtime';
import { resolveTypography } from '../src/ui/theme/typography';

afterEach(() => { cleanup(); resetLocaleForTesting(); });
beforeEach(() => setLocaleForTesting('zh-CN'));

type Typography = Appearance['typography'];
const initial: Typography = {
  uiFont: { kind: 'system-ui' }, readingFont: { kind: 'local', family: 'Georgia' }, codeFont: { kind: 'system-mono' },
  uiSize: 16, readingSize: 21, codeSize: 14, lineHeight: 2, readingFollowUi: true,
};

function StatefulTypography({ onValue }: { onValue?: (value: Typography) => void }) {
  const [value, setValue] = useState(initial);
  const change = (next: Typography) => { setValue(next); onValue?.(next); };
  return <AppearanceTypography value={value} disabled={false} onChange={change} />;
}

describe('appearance typography controls', () => {
  it('keeps independent reading preferences through follow mode and resolves inherited values live', () => {
    const current = { value: structuredClone(initial) };
    render(<StatefulTypography onValue={next => { current.value = next; }} />);
    fireEvent.change(screen.getByRole('spinbutton', { name: '界面字号 px' }), { target: { value: '18' } });
    fireEvent.click(screen.getByRole('radio', { name: '独立设置' }));
    expect(screen.getByRole('spinbutton', { name: '阅读字号 px' })).toHaveProperty('value', '21');
    expect(current.value.readingFont).toEqual({ kind: 'local', family: 'Georgia' });
    expect(resolveTypography(current.value)['--font-size-reading']).toBe('21px');

    fireEvent.click(screen.getByRole('radio', { name: '跟随界面' }));
    expect(current.value).toMatchObject({ readingFollowUi: true, readingSize: 21, readingFont: { kind: 'local', family: 'Georgia' } });
    expect(resolveTypography(current.value)['--font-size-reading']).toBe('18px');
    expect(resolveTypography(current.value)['--font-family-reading']).toBe(resolveTypography(current.value)['--font-family-ui']);
    fireEvent.click(screen.getByRole('radio', { name: '独立设置' }));
    expect(screen.getByRole('spinbutton', { name: '阅读字号 px' })).toHaveProperty('value', '21');
    expect(current.value.readingFont).toEqual({ kind: 'local', family: 'Georgia' });
  });

  it('preserves control state and selection when the locale rerenders', () => {
    const view = render(<StatefulTypography />);
    fireEvent.click(screen.getByRole('radio', { name: '独立设置' }));
    const uiSize = screen.getByRole('spinbutton', { name: '界面字号 px' });
    fireEvent.change(uiSize, { target: { value: '19' } });
    fireEvent.change(screen.getByRole('spinbutton', { name: '阅读字号 px' }), { target: { value: '22' } });
    fireEvent.change(screen.getByRole('combobox', { name: '内容行高' }), { target: { value: '1.5' } });

    act(() => setLocaleForTesting('en-US'));
    expect(screen.getByRole('spinbutton', { name: 'Interface size px' })).toHaveProperty('value', '19');
    expect(screen.getByRole('spinbutton', { name: 'Reading size px' })).toHaveProperty('value', '22');
    expect(screen.getByRole('radio', { name: 'Independent' }).getAttribute('aria-checked')).toBe('true');
    expect((screen.getByRole('combobox', { name: 'Reading line height' }) as HTMLSelectElement).value).toBe('1.5');
    expect(screen.getByTestId('appearance-reading-font').textContent).toContain('Georgia');
    view.unmount();
  });
});
