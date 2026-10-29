// AC07, AC15, AC16 (FR-005 Phase 5): Modal and Tooltip of `@dgos/ui`.
//
// What this file can and cannot prove (jsdom: no layout, no painting, no CSS variable resolution, no screen reader):
//  - Proven here: roles and names, `aria-labelledby`/`aria-describedby`, siblings hidden while a dialog is open,
//    removal from the DOM, Tab/Shift+Tab wrapping inside the dialog, Esc closing only the top dialog, focus restored
//    to the opener, one confirm callback per confirm, no dismissal while busy, tooltip open/close by hover, focus,
//    Esc and blur, and that the class literals the components use are real Tailwind utilities.
//  - NOT proven here, left to a real WebView (Phase 8): that focus really cannot leave the dialog with a physical
//    Tab key (jsdom has no default Tab action, so Tab is checked through Radix's keydown handler plus a programmatic
//    focus-escape), that scrims visibly darken per layer, the 400-600px width and viewport fit, that the Select
//    popup is clickable above a Modal, that animations run, and VoiceOver announcements.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { useState } from 'react';
import { Button, Modal, ModalHost, Select, ToastViewport, Tooltip, clearToasts, toast } from '@dgos/ui';
import { CONFIRM_FAILED_TEXT } from '../src/ui/components/Modal';
import { compileUi, contrast, themeTokens } from './helpers/designTokens';

afterEach(() => {
  act(() => Modal.closeAll());
  act(() => clearToasts());
  cleanup();
  vi.restoreAllMocks();
});

/** A dialog driven by a button, like a page would use it. */
function Opener(props: Partial<React.ComponentProps<typeof Modal>> & { label?: string }) {
  const { label = 'open', ...modal } = props;
  const [open, setOpen] = useState(false);
  return (
    <>
      <button onClick={() => setOpen(true)}>{label}</button>
      <main>background content</main>
      <Modal title="Delete provider" description="This cannot be undone." {...modal} open={open} onOpenChange={setOpen} />
    </>
  );
}

/** The global layer (FR-005 §4.8) an element paints at: its own or nearest ancestor's `z-*` token, resolved. */
function layerOf(element: Element | null): number {
  for (let node = element; node; node = node.parentElement) {
    const name = /(?:^|\s)z-([a-z]+)(?=\s|$)/.exec(node.getAttribute('class') ?? '')?.[1];
    if (name) return Number(themeTokens().light.get(`--z-index-${name}`));
  }
  return NaN;
}

function openDialog(label = 'open') {
  const trigger = screen.getByRole('button', { name: label });
  trigger.focus();
  fireEvent.click(trigger);
  return trigger;
}

describe('Modal (AC07)', () => {
  it('shows a dialog named by its title and described by its text, with a scrim', () => {
    render(<Opener />);
    expect(screen.queryByRole('dialog')).toBeNull();
    openDialog();
    const dialog = screen.getByRole('dialog', { name: 'Delete provider' });
    const titleId = dialog.getAttribute('aria-labelledby');
    expect(titleId).toBeTruthy();
    expect(document.getElementById(titleId!)?.textContent).toBe('Delete provider');
    const describedBy = dialog.getAttribute('aria-describedby');
    expect(document.getElementById(describedBy!)?.textContent).toBe('This cannot be undone.');
    // The scrim is a sibling layer of the content, drawn from the --color-overlay token at --opacity-overlay.
    const scrim = document.querySelector('.bg-overlay');
    expect(scrim).not.toBeNull();
    expect(scrim!.className).toContain('opacity-overlay');
  });

  it('hides the page behind it from assistive technology while open', () => {
    render(<Opener />);
    openDialog();
    // Radix marks the app root aria-hidden; the page content is then not exposed by role queries.
    expect(screen.queryByRole('main')).toBeNull();
    expect(screen.getByRole('main', { hidden: true })).toBeTruthy();
    const appRoot = screen.getByRole('main', { hidden: true }).parentElement!;
    expect(appRoot.getAttribute('aria-hidden')).toBe('true');
  });

  it('is removed from the DOM when closed, and the page is exposed again', () => {
    render(<Opener />);
    openDialog();
    fireEvent.click(screen.getByRole('button', { name: '取消' }));
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(document.querySelector('.bg-overlay')).toBeNull();
    expect(screen.getByRole('main')).toBeTruthy();
  });

  it('renders no description element when there is no description, and does not warn', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => undefined);
    const error = vi.spyOn(console, 'error').mockImplementation(() => undefined);
    render(<Opener description={undefined} />);
    openDialog();
    expect(screen.getByRole('dialog').hasAttribute('aria-describedby')).toBe(false);
    expect(warn).not.toHaveBeenCalled();
    expect(error).not.toHaveBeenCalled();
  });

  it('confirm: Cancel and Confirm; the confirm callback runs and the dialog closes', async () => {
    const onConfirm = vi.fn();
    render(<Opener onConfirm={onConfirm} confirmText="删除" />);
    openDialog();
    const dialog = screen.getByRole('dialog');
    expect(within(dialog).getAllByRole('button').map((b) => b.textContent || b.getAttribute('aria-label'))).toEqual([
      '取消',
      '删除',
      '关闭',
    ]);
    fireEvent.click(within(dialog).getByRole('button', { name: '删除' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(onConfirm).toHaveBeenCalledTimes(1);
  });

  it('alert: one OK button that closes; there is no Cancel', async () => {
    const onConfirm = vi.fn();
    render(<Opener kind="alert" onConfirm={onConfirm} />);
    openDialog();
    const dialog = screen.getByRole('dialog');
    expect(within(dialog).queryByRole('button', { name: '取消' })).toBeNull();
    fireEvent.click(within(dialog).getByRole('button', { name: '确定' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(onConfirm).toHaveBeenCalledTimes(1);
  });

  it('form: shows its children and a custom footer in place of the default buttons', () => {
    render(
      <Opener kind="form" footer={<button>保存</button>}>
        <label>
          名称
          <input />
        </label>
      </Opener>,
    );
    openDialog();
    const dialog = screen.getByRole('dialog');
    expect(within(dialog).getByLabelText('名称')).toBeTruthy();
    expect(within(dialog).getByRole('button', { name: '保存' })).toBeTruthy();
    expect(within(dialog).queryByRole('button', { name: '取消' })).toBeNull();
  });

  it('the × and Cancel dismiss through onCancel; the title icon is decorative', () => {
    const onCancel = vi.fn();
    render(<Opener onCancel={onCancel} icon="warning" />);
    openDialog();
    expect(document.querySelector('svg[data-icon="warning"]')?.getAttribute('aria-hidden')).toBe('true');
    fireEvent.click(screen.getByRole('button', { name: '关闭' }));
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  it('a click on the scrim does not dismiss it', async () => {
    const onCancel = vi.fn();
    render(<Opener onCancel={onCancel} />);
    openDialog();
    // Radix arms its outside-press listener on a timer after mount; a press before that proves nothing.
    const armed = Promise.withResolvers<void>();
    setTimeout(armed.resolve, 10);
    await act(async () => armed.promise);
    const scrim = document.querySelector('.bg-overlay')!;
    fireEvent.pointerDown(scrim);
    fireEvent.pointerUp(scrim);
    fireEvent.click(scrim);
    expect(screen.getByRole('dialog')).toBeTruthy();
    expect(onCancel).not.toHaveBeenCalled();
  });
});

describe('Modal accessibility and stacking (AC16)', () => {
  it('moves focus into the dialog and lands on an action, not on the ×', () => {
    render(<Opener />);
    openDialog();
    const dialog = screen.getByRole('dialog');
    expect(dialog.contains(document.activeElement)).toBe(true);
    expect(document.activeElement).not.toBe(screen.getByRole('button', { name: '关闭' }));
  });

  it('Tab and Shift+Tab wrap between the first and last tab stop inside the dialog', () => {
    render(<Opener />);
    openDialog();
    const dialog = screen.getByRole('dialog');
    const stops = within(dialog).getAllByRole('button');
    const first = stops[0];
    const last = stops[stops.length - 1];
    last.focus();
    fireEvent.keyDown(last, { key: 'Tab' });
    expect(document.activeElement).toBe(first);
    first.focus();
    fireEvent.keyDown(first, { key: 'Tab', shiftKey: true });
    expect(document.activeElement).toBe(last);
  });

  it('pulls focus back when it is moved outside the dialog', async () => {
    render(
      <>
        <button>outside</button>
        <Opener />
      </>,
    );
    openDialog();
    const dialog = screen.getByRole('dialog');
    act(() => screen.getByRole('button', { name: 'outside', hidden: true }).focus());
    await waitFor(() => expect(dialog.contains(document.activeElement)).toBe(true));
  });

  it('Esc closes the dialog and focus returns to the element that opened it', async () => {
    render(<Opener />);
    const trigger = openDialog();
    fireEvent.keyDown(document.activeElement!, { key: 'Escape' });
    expect(screen.queryByRole('dialog')).toBeNull();
    // Radix restores focus from a deferred callback, so it is observed after the close.
    await waitFor(() => expect(document.activeElement).toBe(trigger));
  });

  it('focus returns to the opener after Cancel as well', async () => {
    render(<Opener />);
    const trigger = openDialog();
    fireEvent.click(screen.getByRole('button', { name: '取消' }));
    await waitFor(() => expect(document.activeElement).toBe(trigger));
  });

  it('with two dialogs open, Esc closes only the top one, then the next', async () => {
    const first = Modal.confirm({ title: 'first' });
    const second = Modal.confirm({ title: 'second' });
    render(<ModalHost />);
    expect(screen.getAllByRole('dialog', { hidden: true })).toHaveLength(2);
    // Only the top dialog is exposed; the one under it is hidden with the rest of the page.
    expect(screen.getAllByRole('dialog')).toHaveLength(1);
    expect(screen.getByRole('dialog', { name: 'second' })).toBeTruthy();
    fireEvent.keyDown(document.activeElement!, { key: 'Escape' });
    await waitFor(() => expect(screen.getAllByRole('dialog', { hidden: true })).toHaveLength(1));
    expect(screen.getByRole('dialog', { name: 'first' })).toBeTruthy();
    fireEvent.keyDown(document.activeElement!, { key: 'Escape' });
    await waitFor(() => expect(screen.queryByRole('dialog', { hidden: true })).toBeNull());
    expect(first.id).not.toBe(second.id);
  });

  it('every layer adds its own scrim, so the page darkens with each dialog', () => {
    Modal.confirm({ title: 'one' });
    render(<ModalHost />);
    expect(document.querySelectorAll('.bg-overlay')).toHaveLength(1);
    act(() => void Modal.confirm({ title: 'two' }));
    expect(document.querySelectorAll('.bg-overlay')).toHaveLength(2);
    act(() => void Modal.alert({ title: 'three' }));
    expect(document.querySelectorAll('.bg-overlay')).toHaveLength(3);
  });

  it('a later dialog is later in the DOM, so it paints above the earlier one at the same z-index', () => {
    Modal.confirm({ title: 'under' });
    render(<ModalHost />);
    act(() => void Modal.confirm({ title: 'over' }));
    const dialogs = screen.getAllByRole('dialog', { hidden: true });
    expect(dialogs.map((d) => d.textContent?.startsWith('under'))).toEqual([true, false]);
    expect(dialogs[0].compareDocumentPosition(dialogs[1]) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it('two clicks in the same tick, before React re-renders, still run the confirm callback once', async () => {
    const gate = Promise.withResolvers<void>();
    const onConfirm = vi.fn(() => gate.promise);
    render(<Opener onConfirm={onConfirm} confirmText="删除" />);
    openDialog();
    const confirm = screen.getByRole('button', { name: '删除' });
    // One act() batches both clicks, so the second sees the render from before the first: only the ref guard stops it.
    act(() => {
      confirm.click();
      confirm.click();
    });
    expect(onConfirm).toHaveBeenCalledTimes(1);
    await act(async () => gate.resolve());
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  });

  it('the confirm callback runs once however many times Confirm is clicked while it is pending', async () => {
    const gate = Promise.withResolvers<void>();
    const onConfirm = vi.fn(() => gate.promise);
    render(<Opener onConfirm={onConfirm} confirmText="删除" />);
    openDialog();
    const confirm = screen.getByRole('button', { name: '删除' });
    fireEvent.click(confirm);
    fireEvent.click(confirm);
    fireEvent.click(confirm);
    expect(onConfirm).toHaveBeenCalledTimes(1);
    expect(confirm.getAttribute('aria-busy')).toBe('true');
    await act(async () => gate.resolve());
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(onConfirm).toHaveBeenCalledTimes(1);
  });

  it('cannot be dismissed while the confirm callback is pending', async () => {
    const gate = Promise.withResolvers<void>();
    const onCancel = vi.fn();
    render(
      <Opener
        onCancel={onCancel}
        onConfirm={() => gate.promise}
        confirmText="删除"
      />,
    );
    openDialog();
    fireEvent.click(screen.getByRole('button', { name: '删除' }));
    fireEvent.keyDown(document.activeElement!, { key: 'Escape' });
    expect((screen.getByRole('button', { name: '取消' }) as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByRole('button', { name: '关闭' }) as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByRole('dialog')).toBeTruthy();
    expect(onCancel).not.toHaveBeenCalled();
    await act(async () => gate.resolve());
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  });

  it('stays open with a fixed message when the confirm callback rejects, and can be retried', async () => {
    const secret = 'token=abc123 at /Users/x/secret.rs:10';
    const onConfirm = vi.fn().mockRejectedValueOnce(new Error(secret)).mockResolvedValueOnce(undefined);
    render(<Opener onConfirm={onConfirm} confirmText="删除" />);
    openDialog();
    fireEvent.click(screen.getByRole('button', { name: '删除' }));
    const alert = await screen.findByRole('alert');
    expect(alert.textContent).toBe(CONFIRM_FAILED_TEXT);
    expect(document.body.textContent).not.toContain('abc123');
    expect(screen.getByRole('dialog')).toBeTruthy();
    expect(screen.getByRole('button', { name: '删除' }).hasAttribute('aria-busy')).toBe(false);
    fireEvent.click(screen.getByRole('button', { name: '删除' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(onConfirm).toHaveBeenCalledTimes(2);
  });
});

describe('Modal rounds: reopening starts clean', () => {
  /** Open/close from outside the dialog, as a page that owns `open` would. */
  function Page(props: Partial<React.ComponentProps<typeof Modal>>) {
    const [open, setOpen] = useState(false);
    return (
      <>
        <button onClick={() => setOpen(true)}>open</button>
        <button onClick={() => setOpen(false)}>force-close</button>
        <Modal title="Delete provider" {...props} open={open} onOpenChange={setOpen} />
      </>
    );
  }
  const openIt = () => fireEvent.click(screen.getByText('open', { selector: 'button' }));
  const forceClose = () => fireEvent.click(screen.getByText('force-close', { selector: 'button' }));

  it('does not show the previous round\'s confirm failure after cancel and reopen', async () => {
    render(<Page onConfirm={() => Promise.reject(new Error('x'))} confirmText="删除" />);
    openIt();
    fireEvent.click(screen.getByRole('button', { name: '删除' }));
    await screen.findByRole('alert');
    fireEvent.click(screen.getByRole('button', { name: '取消' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    openIt();
    expect(screen.getByRole('dialog')).toBeTruthy();
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('a confirm still pending when the dialog is closed and reopened does not close or taint the new round', async () => {
    const first = Promise.withResolvers<void>();
    const onConfirm = vi.fn().mockReturnValueOnce(first.promise).mockResolvedValue(undefined);
    render(<Page onConfirm={onConfirm} confirmText="删除" />);
    openIt();
    fireEvent.click(screen.getByRole('button', { name: '删除' }));
    forceClose();
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    openIt();
    const confirm = screen.getByRole('button', { name: '删除' });
    // The new round is idle, not busy from the old confirm.
    expect(confirm.hasAttribute('aria-busy')).toBe(false);
    await act(async () => first.resolve());
    expect(screen.getByRole('dialog')).toBeTruthy();
    expect(onConfirm).toHaveBeenCalledTimes(1);
  });

  it('a stale rejection does not put an error into the new round', async () => {
    const first = Promise.withResolvers<void>();
    const onConfirm = vi.fn().mockReturnValueOnce(first.promise);
    render(<Page onConfirm={onConfirm} confirmText="删除" />);
    openIt();
    fireEvent.click(screen.getByRole('button', { name: '删除' }));
    forceClose();
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    openIt();
    await act(async () => first.reject(new Error('late')));
    expect(screen.getByRole('dialog')).toBeTruthy();
    expect(screen.queryByRole('alert')).toBeNull();
    expect(screen.getByRole('button', { name: '删除' }).hasAttribute('aria-busy')).toBe(false);
  });

  it('the new round can confirm normally after a stale confirm', async () => {
    const first = Promise.withResolvers<void>();
    const onConfirm = vi.fn().mockReturnValueOnce(first.promise).mockResolvedValue(undefined);
    render(<Page onConfirm={onConfirm} confirmText="删除" />);
    openIt();
    fireEvent.click(screen.getByRole('button', { name: '删除' }));
    forceClose();
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    openIt();
    fireEvent.click(screen.getByRole('button', { name: '删除' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(onConfirm).toHaveBeenCalledTimes(2);
  });
});

describe('imperative Modal API', () => {
  it('Modal.confirm shows a confirm dialog; Confirm runs onConfirm and closes it', async () => {
    const onConfirm = vi.fn();
    Modal.confirm({ title: '删除确认', content: '确定要删除吗？', icon: 'warning', onConfirm });
    render(<ModalHost />);
    const dialog = screen.getByRole('dialog', { name: '删除确认' });
    expect(within(dialog).getByText('确定要删除吗？')).toBeTruthy();
    fireEvent.click(within(dialog).getByRole('button', { name: '确定' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(onConfirm).toHaveBeenCalledTimes(1);
  });

  it('Modal.error is an alert with the error icon and a custom OK text', async () => {
    const onOk = vi.fn();
    Modal.error({ title: '权限被拒绝', content: '该模块没有权限。', okText: '授予权限', onOk });
    render(<ModalHost />);
    const dialog = screen.getByRole('dialog', { name: '权限被拒绝' });
    expect(dialog.getAttribute('data-modal-kind')).toBe('alert');
    expect(dialog.querySelector('svg[data-icon="error"]')).not.toBeNull();
    expect(within(dialog).queryByRole('button', { name: '取消' })).toBeNull();
    fireEvent.click(within(dialog).getByRole('button', { name: '授予权限' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(onOk).toHaveBeenCalledTimes(1);
  });

  it('Esc on Modal.error closes it without running onOk', async () => {
    const onOk = vi.fn();
    Modal.error({ title: 'denied', onOk });
    render(<ModalHost />);
    fireEvent.keyDown(document.activeElement!, { key: 'Escape' });
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(onOk).not.toHaveBeenCalled();
  });

  it('the returned handle closes its own dialog only', () => {
    const a = Modal.alert({ title: 'a' });
    Modal.alert({ title: 'b' });
    render(<ModalHost />);
    act(() => a.close());
    expect(screen.getAllByRole('dialog', { hidden: true }).map((d) => d.textContent)).toEqual([expect.stringContaining('b')]);
  });

  it('focus returns to the opener when an imperative dialog closes', async () => {
    render(
      <>
        <button onClick={() => void Modal.confirm({ title: 'x' })}>ask</button>
        <ModalHost />
      </>,
    );
    const trigger = openDialog('ask');
    fireEvent.click(screen.getByRole('button', { name: '取消' }));
    await waitFor(() => expect(document.activeElement).toBe(trigger));
  });
});

describe('Toast while a Modal is open (AC08, AC16)', () => {
  // Mounted like main.tsx: the toast viewport and the page share one container, the dialog portals to <body>.
  // Radix hides everything outside an open dialog from assistive technology; a toast must not be hidden with it,
  // or "saved" after a confirm (toast.success inside onConfirm, before the dialog closes) is never announced.
  function App() {
    return (
      <div id="app-root">
        <Opener />
        <ToastViewport />
      </div>
    );
  }

  it('a toast raised while the dialog is open stays in the accessibility tree', () => {
    render(<App />);
    openDialog();
    expect(screen.getByRole('dialog')).toBeTruthy();
    act(() => {
      toast.success('已保存');
    });
    expect(screen.getByRole('status').textContent).toContain('已保存');
    expect(screen.getByRole('region', { name: '通知' }).closest('[aria-hidden="true"]')).toBeNull();
  });

  it('a toast already showing when the dialog opens stays in the accessibility tree, and so does a later one', () => {
    render(<App />);
    act(() => {
      toast.info('同步中');
    });
    openDialog();
    act(() => {
      toast.error('同步失败');
    });
    expect(screen.getByRole('status').textContent).toContain('同步中');
    expect(screen.getByRole('alert').textContent).toContain('同步失败');
  });
});

describe('Select inside a Modal', () => {
  it('opens its list and commits a value without closing the dialog; the list paints above the dialog and its scrim', async () => {
    function Case() {
      const [value, setValue] = useState('');
      return (
        <Opener kind="form" footer={<button>保存</button>}>
          <Select label="Provider" options={[{ value: 'a', label: 'Alpha' }]} value={value} onChange={setValue} />
          <output>{value}</output>
        </Opener>
      );
    }
    render(<Case />);
    openDialog();
    const trigger = screen.getByRole('combobox', { name: 'Provider' });
    trigger.focus();
    fireEvent.keyDown(trigger, { key: 'Enter' });
    const listbox = await screen.findByRole('listbox');
    // An open Select hides everything outside its list from the accessibility tree, the dialog included.
    const dialogLayer = layerOf(screen.getByRole('dialog', { hidden: true }));
    expect(layerOf(document.querySelector('.bg-overlay'))).toBe(dialogLayer);
    expect(layerOf(listbox)).toBeGreaterThan(dialogLayer);
    // 0-49 is the in-page band (a canvas or page stacks inside it); a dialog must never fall into it.
    expect(dialogLayer).toBeGreaterThan(49);
    fireEvent.click(within(listbox).getByRole('option', { name: 'Alpha' }));
    expect(document.querySelector('output')?.textContent).toBe('a');
    expect(screen.getByRole('dialog')).toBeTruthy();
  });
});

describe('Tooltip (AC15)', () => {
  function Hint(props: { content?: string }) {
    return (
      <Tooltip content={props.content ?? 'Copy to clipboard'} delayDuration={0}>
        <button>copy</button>
      </Tooltip>
    );
  }

  it('shows on keyboard focus and links the trigger to the text with aria-describedby', async () => {
    render(<Hint />);
    const trigger = screen.getByRole('button', { name: 'copy' });
    expect(trigger.hasAttribute('aria-describedby')).toBe(false);
    act(() => trigger.focus());
    const tip = await screen.findByRole('tooltip');
    expect(tip.textContent).toBe('Copy to clipboard');
    const content = document.getElementById(trigger.getAttribute('aria-describedby')!);
    expect(content?.textContent).toContain('Copy to clipboard');
  });

  it('shows on pointer hover and hides when the pointer leaves', async () => {
    render(<Hint />);
    const trigger = screen.getByRole('button', { name: 'copy' });
    fireEvent.pointerMove(trigger, { pointerType: 'mouse' });
    expect(await screen.findByRole('tooltip')).toBeTruthy();
    fireEvent.pointerLeave(trigger, { pointerType: 'mouse', clientX: 0, clientY: 0 });
    // Radix keeps the tooltip open inside a grace area between trigger and content (WCAG 1.4.13: hoverable);
    // it closes once the pointer moves away from both.
    fireEvent.pointerMove(document.body, { pointerType: 'mouse', clientX: 900, clientY: 900 });
    await waitFor(() => expect(screen.queryByRole('tooltip')).toBeNull());
  });

  it('hides on Esc', async () => {
    render(<Hint />);
    const trigger = screen.getByRole('button', { name: 'copy' });
    act(() => trigger.focus());
    await screen.findByRole('tooltip');
    fireEvent.keyDown(trigger, { key: 'Escape' });
    await waitFor(() => expect(screen.queryByRole('tooltip')).toBeNull());
    expect(trigger.hasAttribute('aria-describedby')).toBe(false);
  });

  it('hides when focus leaves the trigger', async () => {
    render(
      <>
        <Hint />
        <button>next</button>
      </>,
    );
    const trigger = screen.getByRole('button', { name: 'copy' });
    act(() => trigger.focus());
    await screen.findByRole('tooltip');
    act(() => screen.getByRole('button', { name: 'next' }).focus());
    await waitFor(() => expect(screen.queryByRole('tooltip')).toBeNull());
  });

  it('renders markup in the content as text', async () => {
    const markup = '<img src=x onerror=alert(1)> <b>bold</b>';
    render(<Hint content={markup} />);
    act(() => screen.getByRole('button', { name: 'copy' }).focus());
    const tip = await screen.findByRole('tooltip');
    expect(tip.textContent).toBe(markup);
    expect(document.querySelector('img')).toBeNull();
    expect(document.querySelector('b')).toBeNull();
  });

  it('empty content renders the child alone, with no tooltip wiring', () => {
    render(<Hint content="" />);
    const trigger = screen.getByRole('button', { name: 'copy' });
    act(() => trigger.focus());
    expect(screen.queryByRole('tooltip')).toBeNull();
    expect(trigger.hasAttribute('data-state')).toBe(false);
  });

  it('works around a Button and keeps its own props', async () => {
    const onClick = vi.fn();
    render(
      <Tooltip content="Save" delayDuration={0}>
        <Button onClick={onClick}>save</Button>
      </Tooltip>,
    );
    const button = screen.getByRole('button', { name: 'save' });
    fireEvent.click(button);
    expect(onClick).toHaveBeenCalledTimes(1);
    act(() => button.focus());
    expect(await screen.findByRole('tooltip')).toBeTruthy();
  });
});

describe('overlay tokens and class literals (D066, D069)', () => {
  it('the dialog edge reads against the page surface in both themes even where the scrim does not', () => {
    // A 60% black scrim over the dark surface leaves the dialog nearly the same shade, so the dialog needs its own
    // edge: border-control (>= 3:1 on the surface, WCAG 1.4.11). The scrim token itself is exempt (contrast-pairs.json).
    const { light, dark } = themeTokens();
    for (const [name, tokens] of [['light', light], ['dark', dark]] as const) {
      expect(contrast(tokens.get('--color-border-control')!, tokens.get('--color-bg-primary')!), name).toBeGreaterThanOrEqual(3);
    }
    expect(light.get('--color-overlay')).toBe('#000000');
    expect(light.get('--opacity-overlay')).toBe('0.6');
  });

  it('the dialog text pairs are declared pairs (popover surface is bg-primary)', () => {
    const { light, dark } = themeTokens();
    for (const [name, tokens] of [['light', light], ['dark', dark]] as const) {
      for (const text of ['text-primary', 'text-secondary', 'error-text', 'success-text', 'warning-text', 'info-text']) {
        expect(contrast(tokens.get(`--color-${text}`)!, tokens.get('--color-bg-primary')!), `${name} ${text}`).toBeGreaterThanOrEqual(4.5);
      }
    }
  });

  it('the utilities Modal, Tooltip and Select use are generated by the real Tailwind build', async () => {
    const css = await compileUi([
      'bg-overlay', 'opacity-overlay', 'animate-overlay-in', 'animate-modal-in', 'z-modal', 'z-popover', 'fixed', 'inset-0', 'inset-4',
      'm-auto', 'h-fit', 'w-fit', 'min-w-100', 'max-w-150', 'max-h-9/10', 'max-w-75', 'overflow-y-auto', 'border-input',
      'shadow-xl', 'bg-popover', 'text-popover-foreground', 'text-muted-foreground', 'text-error-text', 'wrap-break-word',
    ]);
    expect(css).toMatch(/\.bg-overlay\s*\{\s*background-color:\s*var\(--color-overlay\)/);
    expect(css).toMatch(/\.opacity-overlay\s*\{\s*opacity:\s*var\(--opacity-overlay\)/);
    expect(css).toMatch(/\.z-modal\s*\{\s*z-index:\s*var\(--z-index-modal\)/);
    expect(css).toMatch(/\.z-popover\s*\{\s*z-index:\s*var\(--z-index-popover\)/);
    expect(css).toMatch(/\.animate-modal-in\s*\{\s*animation:\s*var\(--animate-modal-in\)/);
    expect(css).toMatch(/--animate-modal-in:\s*modal-in var\(--duration-slow\)/);
    expect(css).toMatch(/@keyframes modal-in/);
    expect(css).toMatch(/@keyframes overlay-in\s*\{[^}]*\}[^}]*opacity:\s*var\(--opacity-overlay\)/);
    expect(css).toMatch(/\.min-w-100\s*\{\s*min-width:\s*calc\(var\(--spacing-unit\)\s*\*\s*100\)/);
    expect(css).toMatch(/\.max-w-150\s*\{\s*max-width:\s*calc\(var\(--spacing-unit\)\s*\*\s*150\)/);
    expect(css).toMatch(/\.max-h-9\\\/10\s*\{\s*max-height:\s*calc\(9 \/ 10 \* 100%\)/);
    expect(css).toMatch(/\.wrap-break-word\s*\{\s*overflow-wrap:\s*break-word/);
  });
});
