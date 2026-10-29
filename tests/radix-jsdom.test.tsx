// D066 spike: which Radix primitives behave under jsdom, and which jsdom gaps tests/setup.ts must fill.
import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { useState } from 'react';
import { Dialog, Popover, Select, Tabs, Tooltip } from 'radix-ui';

afterEach(cleanup);

function DialogCase() {
  return (
    <Dialog.Root>
      <Dialog.Trigger>open dialog</Dialog.Trigger>
      <Dialog.Portal>
        <Dialog.Overlay />
        <Dialog.Content>
          <Dialog.Title>Delete provider</Dialog.Title>
          <Dialog.Description>This cannot be undone.</Dialog.Description>
          <button>confirm</button>
          <Dialog.Close>cancel</Dialog.Close>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

describe('Radix under jsdom', () => {
  it('Dialog opens with a named dialog role, moves focus inside, closes on Escape and restores focus', async () => {
    render(<DialogCase />);
    const trigger = screen.getByRole('button', { name: 'open dialog' });
    trigger.focus();
    fireEvent.click(trigger);
    const dialog = screen.getByRole('dialog', { name: 'Delete provider' });
    expect(dialog.contains(document.activeElement)).toBe(true);
    fireEvent.keyDown(document.activeElement ?? dialog, { key: 'Escape' });
    expect(screen.queryByRole('dialog')).toBeNull();
    // Radix restores focus from a deferred callback, so it is observed after the close, not during it.
    await waitFor(() => expect(document.activeElement).toBe(trigger));
  });

  it('Tabs switch on click and expose tab/tabpanel state', () => {
    function TabsCase() {
      const [value, setValue] = useState('a');
      return (
        <Tabs.Root value={value} onValueChange={setValue}>
          <Tabs.List>
            <Tabs.Trigger value="a">Alpha</Tabs.Trigger>
            <Tabs.Trigger value="b">Beta</Tabs.Trigger>
          </Tabs.List>
          <Tabs.Content value="a">panel a</Tabs.Content>
          <Tabs.Content value="b">panel b</Tabs.Content>
        </Tabs.Root>
      );
    }
    render(<TabsCase />);
    expect(screen.getByRole('tab', { name: 'Alpha' }).getAttribute('aria-selected')).toBe('true');
    fireEvent.mouseDown(screen.getByRole('tab', { name: 'Beta' }));
    expect(screen.getByRole('tab', { name: 'Beta' }).getAttribute('aria-selected')).toBe('true');
    expect(screen.getByRole('tabpanel').textContent).toBe('panel b');
  });

  it('Tooltip shows on focus', async () => {
    render(
      <Tooltip.Provider delayDuration={0}>
        <Tooltip.Root>
          <Tooltip.Trigger>hint target</Tooltip.Trigger>
          <Tooltip.Portal>
            <Tooltip.Content>hint text</Tooltip.Content>
          </Tooltip.Portal>
        </Tooltip.Root>
      </Tooltip.Provider>,
    );
    fireEvent.focus(screen.getByRole('button', { name: 'hint target' }));
    expect(await screen.findByRole('tooltip')).toBeTruthy();
  });

  it('Select opens on keyboard, lists options and commits the chosen value', async () => {
    function SelectCase() {
      const [value, setValue] = useState('');
      return (
        <>
          <Select.Root value={value} onValueChange={setValue}>
            <Select.Trigger aria-label="provider">
              <Select.Value placeholder="pick" />
            </Select.Trigger>
            <Select.Portal>
              <Select.Content>
                <Select.Viewport>
                  <Select.Item value="openai"><Select.ItemText>OpenAI</Select.ItemText></Select.Item>
                  <Select.Item value="anthropic"><Select.ItemText>Anthropic</Select.ItemText></Select.Item>
                </Select.Viewport>
              </Select.Content>
            </Select.Portal>
          </Select.Root>
          <output>{value}</output>
        </>
      );
    }
    render(<SelectCase />);
    const trigger = screen.getByRole('combobox', { name: 'provider' });
    trigger.focus();
    fireEvent.keyDown(trigger, { key: 'Enter' });
    const listbox = await screen.findByRole('listbox');
    fireEvent.click(within(listbox).getByRole('option', { name: 'Anthropic' }));
    expect(document.querySelector('output')?.textContent).toBe('anthropic');
  });

  it('Select opens from a mouse pointerdown (needs the pointer-capture stubs)', async () => {
    render(
      <Select.Root>
        <Select.Trigger aria-label="provider"><Select.Value placeholder="pick" /></Select.Trigger>
        <Select.Portal>
          <Select.Content>
            <Select.Viewport>
              <Select.Item value="a"><Select.ItemText>A</Select.ItemText></Select.Item>
            </Select.Viewport>
          </Select.Content>
        </Select.Portal>
      </Select.Root>,
    );
    fireEvent.pointerDown(screen.getByRole('combobox', { name: 'provider' }), { button: 0, pointerType: 'mouse' });
    expect(await screen.findByRole('listbox')).toBeTruthy();
  });

  it('Tooltip opens from a pointer move', async () => {
    render(
      <Tooltip.Provider delayDuration={0}>
        <Tooltip.Root>
          <Tooltip.Trigger>hover target</Tooltip.Trigger>
          <Tooltip.Portal><Tooltip.Content>hint text</Tooltip.Content></Tooltip.Portal>
        </Tooltip.Root>
      </Tooltip.Provider>,
    );
    fireEvent.pointerMove(screen.getByRole('button', { name: 'hover target' }), { pointerType: 'mouse' });
    expect(await screen.findByRole('tooltip')).toBeTruthy();
  });

  it('Popper content with an arrow renders (needs ResizeObserver)', async () => {
    render(
      <Popover.Root defaultOpen>
        <Popover.Trigger>anchor</Popover.Trigger>
        <Popover.Portal><Popover.Content>popover body<Popover.Arrow /></Popover.Content></Popover.Portal>
      </Popover.Root>,
    );
    expect(await screen.findByText('popover body')).toBeTruthy();
  });
});
