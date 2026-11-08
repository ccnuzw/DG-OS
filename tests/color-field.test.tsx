import { setTimeout as delay } from 'node:timers/promises';
import { useState } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { ColorField, type ColorFieldProps } from '../src/ui/appearance/ColorField';
import { PRESETS } from '../src/ui/theme/kernel';
import { t } from '../src/i18n';
import { resetLocaleForTesting, setLocaleForTesting } from '../src/i18n/runtime';

const inherited = PRESETS.studio.light.accent.toUpperCase();
const recommended = PRESETS.forest.light.accent.toUpperCase();
const second = PRESETS.slate.light.accent.toUpperCase();
const pickerName = () => `Accent · ${t('appearance.colorPicker')}`;

function Harness({ initial, onChange = () => {}, onValidityChange }: {
  initial?: string;
  onChange?: ColorFieldProps['onChange'];
  onValidityChange?: ColorFieldProps['onValidityChange'];
}) {
  const [override, setOverride] = useState(initial);
  const [renders, setRenders] = useState(0);
  return <>
    <ColorField label="Accent" value={override ?? inherited} override={override} recommendations={[recommended, second]}
      onChange={next => { setOverride(next); onChange(next); }} onValidityChange={onValidityChange} />
    <output aria-label="Override">{override ?? 'inherited'}</output>
    <button onClick={() => setRenders(renders + 1)}>Render again {renders}</button>
  </>;
}

beforeEach(() => setLocaleForTesting('en-US'));
afterEach(() => { cleanup(); resetLocaleForTesting(); });

async function openPicker() {
  const trigger = screen.getByRole('button', { name: pickerName() });
  trigger.focus();
  fireEvent.click(trigger);
  const dialog = await screen.findByRole('dialog', { name: pickerName() });
  return { trigger, dialog, hex: within(dialog).getByRole('textbox', { name: 'Accent HEX' }) as HTMLInputElement };
}

function currentOverride() {
  return screen.getByLabelText('Override').textContent;
}

describe('ColorField draft boundaries', () => {
  it('keeps invalid field text through blur and unrelated renders, emits only normalized valid colors, and restores inheritance', async () => {
    const change = vi.fn(), validity = vi.fn();
    render(<Harness onChange={change} onValidityChange={validity} />);
    const hex = screen.getByRole('textbox', { name: 'Accent HEX' }) as HTMLInputElement;
    expect(hex.value).toBe(inherited);
    fireEvent.change(hex, { target: { value: '12' } });
    fireEvent.blur(hex);
    expect(hex.value).toBe('12');
    expect(hex.getAttribute('aria-invalid')).toBe('true');
    expect(validity).toHaveBeenLastCalledWith(true);
    expect(change).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'Render again 0' }));
    expect(hex.value).toBe('12');
    fireEvent.change(hex, { target: { value: recommended.slice(1).toLowerCase() } });
    expect(change).toHaveBeenLastCalledWith(recommended);
    expect(hex.value).toBe(recommended.slice(1).toLowerCase());
    fireEvent.keyDown(hex, { key: 'Enter' });
    expect(hex.value).toBe(recommended);
    expect(validity).toHaveBeenLastCalledWith(false);
    fireEvent.click(screen.getByRole('switch', { name: `Accent ${t('appearance.customColor')}` }));
    expect(change).toHaveBeenLastCalledWith(undefined);
    expect(hex.value).toBe(inherited);
  });

  it('resets intermediate text for an external value or override change', () => {
    const props = { label: 'Accent', value: inherited, recommendations: [], onChange: vi.fn() };
    const view = render(<ColorField {...props} />);
    const hex = screen.getByRole('textbox', { name: 'Accent HEX' }) as HTMLInputElement;
    fireEvent.change(hex, { target: { value: 'invalid' } });
    view.rerender(<ColorField {...props} value={recommended} override={recommended} />);
    expect(hex.value).toBe(recommended);
    expect(hex.getAttribute('aria-invalid')).toBeNull();
  });

  it.each([undefined, second])('Escape restores the captured override %s and consumes the outer cancel', async initial => {
    const change = vi.fn();
    render(<Harness initial={initial} onChange={change} />);
    const outer = vi.fn();
    window.addEventListener('keydown', outer);
    try {
      const { trigger, dialog } = await openPicker();
      fireEvent.click(within(within(dialog).getByRole('region', { name: t('appearance.colorRecommendations') })).getByRole('button', { name: recommended }));
      expect(currentOverride()).toBe(recommended);
      fireEvent.keyDown(dialog, { key: 'Escape' });
      await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
      expect(change).toHaveBeenLastCalledWith(initial);
      expect(currentOverride()).toBe(initial ?? 'inherited');
      expect(outer).not.toHaveBeenCalled();
      await waitFor(() => expect(document.activeElement).toBe(trigger));
    } finally {
      window.removeEventListener('keydown', outer);
    }
  });

  it('explicit cancellation restores only the opening override and reopening captures the newest draft', async () => {
    render(<Harness initial={second} />);
    let picker = await openPicker();
    fireEvent.click(within(within(picker.dialog).getByRole('region', { name: t('appearance.colorRecommendations') })).getByRole('button', { name: recommended }));
    fireEvent.click(within(picker.dialog).getByRole('button', { name: t('appearance.colorPickerDone') }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(currentOverride()).toBe(recommended);
    picker = await openPicker();
    fireEvent.change(picker.hex, { target: { value: second.toLowerCase() } });
    expect(currentOverride()).toBe(second);
    fireEvent.click(within(picker.dialog).getByRole('button', { name: t('appearance.colorPickerCancel') }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(currentOverride()).toBe(recommended);
    await waitFor(() => expect(document.activeElement).toBe(picker.trigger));
  });

  it('outside dismissal keeps the latest valid draft, discards incomplete picker text and returns focus', async () => {
    const validity = vi.fn();
    render(<Harness onValidityChange={validity} />);
    const picker = await openPicker();
    fireEvent.change(picker.hex, { target: { value: recommended.slice(1).toLowerCase() } });
    expect(currentOverride()).toBe(recommended);
    fireEvent.change(picker.hex, { target: { value: 'bad' } });
    fireEvent.blur(picker.hex);
    expect(picker.hex.value).toBe('bad');
    expect(picker.hex.getAttribute('aria-invalid')).toBe('true');
    expect(validity).toHaveBeenLastCalledWith(true);
    // Radix arms its outside-press listener after opening, and closes on pointer down/up.
    await act(async () => { await delay(10); });
    fireEvent.pointerDown(document.body, { pointerType: 'mouse', button: 0 });
    fireEvent.pointerUp(document.body, { pointerType: 'mouse', button: 0 });
    fireEvent.click(document.body);
    expect(currentOverride()).toBe(recommended);
    expect(validity).toHaveBeenLastCalledWith(false);
    await waitFor(() => expect(document.activeElement).toBe(picker.trigger));
    const reopened = await openPicker();
    expect(reopened.hex.value).toBe(recommended);
  });

  it('keeps an invalid outer field invalid when an untouched picker is completed', async () => {
    const validity = vi.fn();
    render(<Harness onValidityChange={validity} />);
    const outer = screen.getByRole('textbox', { name: 'Accent HEX' }) as HTMLInputElement;
    fireEvent.change(outer, { target: { value: 'bad' } });
    const { dialog } = await openPicker();
    fireEvent.click(within(dialog).getByRole('button', { name: t('appearance.colorPickerDone') }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(outer.value).toBe('bad');
    expect(validity).toHaveBeenLastCalledWith(true);
  });

  it('uses mature keyboard hue controls and writes valid colors in realtime', async () => {
    const change = vi.fn();
    render(<Harness onChange={change} />);
    const { dialog } = await openPicker();
    const sliders = within(dialog).getAllByRole('slider');
    const hue = sliders.find(slider => slider.getAttribute('aria-label') === 'Hue');
    expect(hue).toBeTruthy();
    fireEvent.keyDown(hue!, { key: 'ArrowRight', keyCode: 39 });
    expect(change).toHaveBeenCalled();
    expect(change.mock.lastCall?.[0]).toMatch(/^#[\dA-F]{6}$/);
  });
});
