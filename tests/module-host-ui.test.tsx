// AC23 (FR-005 Phase 6, D067): `host.ui` as a dynamic module sees it.
//
// Evidence level: jsdom. Modules are loaded through the real `ModuleRuntimeHost` (`readContract` builds `host`),
// but `import(blobUrl)` is replaced by evaluating the source (see tests/helpers/moduleFixtures.ts) and
// `invoke` is a double. NOT proven here: the real WebView import path, theme/density switching of the shared
// components (jsdom has no layout or computed theme), and what a screen reader announces.
//
// Why the `@dgos/ui` components are recorded: most of them drop `style` themselves (`omitStyle`), so looking for
// the value in the DOM would pass even without the `host.ui` wrapper. The recorder sits where the wrapper hands
// its props over, so it fails when the wrapper stops filtering.
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createElement, type ComponentType } from 'react';
import type * as DgosUi from '@dgos/ui';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';

const seen = vi.hoisted(() => [] as Array<{ name: string; props: Record<string, unknown> }>);

vi.mock('@dgos/ui', async (importOriginal) => {
  const actual = await importOriginal<typeof DgosUi>();
  // The component types differ per component; the recorder only needs "something React can render".
  // One named cast here keeps every call site below typed.
  const record = <T extends object>(name: string, Component: T): T => {
    const Original = Component as unknown as ComponentType<Record<string, unknown>>;
    const Recorded = (props: Record<string, unknown>) => {
      seen.push({ name, props: { ...props } });
      return createElement(Original, props);
    };
    const statics: Record<string, unknown> = {};
    for (const [key, value] of Object.entries(Component)) {
      if (typeof value === 'function') statics[key] = record(`${name}.${key}`, value);
    }
    return Object.assign(Recorded, statics) as unknown as T;
  };
  return {
    ...actual,
    Button: record('Button', actual.Button),
    Input: record('Input', actual.Input),
    Select: record('Select', actual.Select),
    Checkbox: record('Checkbox', actual.Checkbox),
    RadioGroup: record('RadioGroup', actual.RadioGroup),
    Switch: record('Switch', actual.Switch),
    Card: record('Card', actual.Card),
    Tooltip: record('Tooltip', actual.Tooltip),
    Loading: record('Loading', actual.Loading),
    Divider: record('Divider', actual.Divider),
    Tabs: record('Tabs', actual.Tabs),
    Stack: record('Stack', actual.Stack),
  };
});

import { Button, ModalHost, ToastViewport, clearToasts, Modal as HostModal } from '@dgos/ui';
import { moduleHostUi } from '../src/modules/moduleHostUi';
import {
  ModuleRuntimeHost,
  type HostInstruction,
  type InstructionChannel,
  type LoadInstruction,
  type Receipt,
} from '../src/modules/moduleRuntimeHost';
import { evaluateFixtureSource, readFixture, resetModuleProbe } from './helpers/moduleFixtures';

const ui = moduleHostUi;
const MARK = 'rgb(11, 22, 33)';

// ---- a real ModuleRuntimeHost with a scripted Tauri side ----

type Invoke = (command: string, args?: Record<string, unknown>) => Promise<unknown>;

function createRuntime(providerCall?: Invoke) {
  const sources = new Map<string, string>();
  const receipts: Receipt[] = [];
  const channel: InstructionChannel = { onmessage: () => undefined };
  const providerCalls: Array<Record<string, unknown>> = [];
  const invoke = vi.fn(async (command: string, args?: Record<string, unknown>): Promise<unknown> => {
    if (command === 'attach_module_host') return { success: true, data: { attachmentId: 'att-1' }, error: null };
    if (command === 'report_module_host') {
      receipts.push(args?.receipt as Receipt);
      return { success: true, data: { disposition: 'applied' }, error: null };
    }
    if (command === 'module_call_provider') {
      providerCalls.push({ ...args });
      return providerCall ? providerCall(command, args) : { success: true, data: { content: 'pong' }, error: null };
    }
    throw new Error(`unexpected command ${command}`);
  });
  const host = new ModuleRuntimeHost({
    invoke: invoke as never,
    createChannel: () => channel,
    createObjectUrl: (source) => {
      const url = `blob:test/${sources.size + 1}`;
      sources.set(url, source);
      return url;
    },
    revokeObjectUrl: () => undefined,
    importModule: async (url) => evaluateFixtureSource(sources.get(url) ?? ''),
  });
  let ticket = 0;
  const send = (instruction: HostInstruction) => channel.onmessage(instruction);
  const load = async (source: string, moduleId: string, runtimeId = `rt-${moduleId}`) => {
    await host.ensureAttached();
    send({ kind: 'route', runtimeId, route: `route-${runtimeId}` });
    const instruction: LoadInstruction = {
      kind: 'load',
      ticket: `t${++ticket}`,
      runtimeId,
      moduleId,
      version: '1.0.0',
      source,
      hookTimeoutMs: 500,
    };
    const before = receipts.length;
    send(instruction);
    await vi.waitFor(() => expect(receipts.length).toBe(before + 1));
    expect(receipts[before]).toEqual({ kind: 'loaded' });
    const view = host.getSnapshot().find((candidate) => candidate.runtimeId === runtimeId)!;
    document.body.appendChild(view.container);
    return view.container;
  };
  return { host, invoke, providerCalls, load };
}

interface ProbedHost {
  ui: unknown;
  [key: string]: unknown;
}

const probeHost = (moduleId: string): ProbedHost => {
  const hosts: unknown = Reflect.get(globalThis, '__dgosUiHosts');
  const host: unknown = hosts !== null && typeof hosts === 'object' ? Reflect.get(hosts, moduleId) : undefined;
  if (host === null || typeof host !== 'object') throw new Error(`module ${moduleId} did not expose its host`);
  return host as ProbedHost;
};

beforeEach(() => {
  seen.length = 0;
  resetModuleProbe();
  vi.spyOn(console, 'error').mockImplementation(() => undefined);
  vi.spyOn(console, 'warn').mockImplementation(() => undefined);
});

afterEach(() => {
  act(() => {
    clearToasts();
    HostModal.closeAll();
  });
  cleanup();
  document.body.replaceChildren();
  vi.restoreAllMocks();
});

const recorded = (name: string) => seen.filter((entry) => entry.name === name);

// ---- injection point and API shape ----

describe('host.ui injection and shape', () => {
  it('readContract hands every module the one frozen ui next to the existing host fields', async () => {
    const runtime = createRuntime();
    await runtime.load(readFixture('ui-probe.js'), 'probe-a');
    await runtime.load(readFixture('ui-probe.js'), 'probe-b');
    const a = probeHost('probe-a');
    const b = probeHost('probe-b');

    expect(Object.keys(a).sort()).toEqual(['React', 'apiVersion', 'moduleId', 'providers', 'ui', 'version']);
    expect(a.apiVersion).toBe(1);
    expect(Object.isFrozen(a)).toBe(true);
    expect(a.ui).toBe(ui);
    expect(b.ui).toBe(ui);
  });

  it('exposes exactly the FR-005 §13.3 members', () => {
    expect(Object.keys(ui).sort()).toEqual(
      [
        'Button', 'Card', 'Checkbox', 'Divider', 'Input', 'Loading', 'Modal', 'Radio',
        'Select', 'Stack', 'Switch', 'Tabs', 'Tooltip', 'toast',
      ].sort(),
    );
    expect(Object.keys(ui.Card).sort()).toEqual(['Body', 'Footer', 'Header']);
    expect(Object.keys(ui.Tabs).sort()).toEqual(['Content', 'List', 'Trigger']);
    expect(Object.keys(ui.toast).sort()).toEqual(['error', 'info', 'success', 'warning']);
  });

  it('is frozen all the way down, so one module cannot change what another receives', () => {
    const targets: unknown[] = [ui];
    for (const member of Object.values(ui)) {
      targets.push(member);
      // Compound components carry their sub-components as own properties.
      if (typeof member === 'function') targets.push(...Object.values(member));
    }
    expect(targets.length).toBeGreaterThan(Object.keys(ui).length + 6);
    for (const target of targets) {
      expect(Object.isFrozen(target)).toBe(true);
    }
    expect(() => {
      (ui as unknown as Record<string, unknown>).Button = null;
    }).toThrow(TypeError);
    expect(() => {
      (ui.Modal as unknown as Record<string, unknown>).confirm = null;
    }).toThrow(TypeError);
    expect(() => {
      (ui.toast as unknown as Record<string, unknown>).info = null;
    }).toThrow(TypeError);
  });

  it('exposes Modal as three commands only: no component, no ModalHost, no closeAll', () => {
    expect(Object.keys(ui.Modal).sort()).toEqual(['alert', 'confirm', 'error']);
    for (const name of ['ModalHost', 'ModalView', 'closeAll']) {
      expect(name in ui).toBe(false);
      expect(name in ui.Modal).toBe(false);
    }
    expect(typeof (ui.Modal as unknown as ComponentType)).toBe('object');
  });
});

// ---- style never reaches a component ----

describe('component wrapper drops style', () => {
  const style = { color: MARK };

  it('the recorder would see style if nothing stripped it', () => {
    render(createElement(Button, { style, className: 'x' } as never, 'plain'));
    expect(recorded('Button')[0]!.props.style).toEqual(style);
  });

  it('removes style from every component and sub-component and keeps className and the rest', () => {
    const h = createElement;
    render(
      h(
        ui.Stack,
        { gap: 2, style, className: 'k-stack' } as never,
        h(ui.Button, { style, className: 'k-button', 'data-k': '1' } as never, 'Go'),
        h(ui.Input, { style, className: 'k-input', label: 'Name', value: '', onChange: () => undefined } as never),
        h(ui.Input, { style, multiline: true, label: 'Notes', value: '', onChange: () => undefined } as never),
        h(ui.Select, { style, label: 'Pick', options: [{ value: 'a', label: 'A' }], value: 'a', onChange: () => undefined } as never),
        h(ui.Checkbox, { style, label: 'Check', checked: false, onChange: () => undefined } as never),
        h(ui.Radio, { style, label: 'Radio', options: [{ value: 'a', label: 'A' }], value: 'a', onChange: () => undefined } as never),
        h(ui.Switch, { style, label: 'Switch', checked: false, onChange: () => undefined } as never),
        h(
          ui.Card,
          { style } as never,
          h(ui.Card.Header, { style } as never, 'head'),
          h(ui.Card.Body, { style } as never, 'body'),
          h(ui.Card.Footer, { style } as never, 'foot'),
        ),
        h(ui.Tooltip, { style, content: 'tip' } as never, h('button', { type: 'button' }, 'anchor')),
        h(ui.Loading, { style, label: 'Loading' } as never),
        h(ui.Divider, { style } as never),
        h(
          ui.Tabs,
          { style, value: 'a', onValueChange: () => undefined } as never,
          h(ui.Tabs.List, { style } as never, h(ui.Tabs.Trigger, { style, value: 'a' } as never, 'A')),
          h(ui.Tabs.Content, { style, value: 'a' } as never, 'panel'),
        ),
      ),
    );

    const names = [
      'Stack', 'Button', 'Input', 'Select', 'Checkbox', 'RadioGroup', 'Switch', 'Card', 'Card.Header', 'Card.Body',
      'Card.Footer', 'Tooltip', 'Loading', 'Divider', 'Tabs', 'Tabs.List', 'Tabs.Trigger', 'Tabs.Content',
    ];
    for (const name of names) {
      const entries = recorded(name);
      expect(entries.length, `${name} was rendered`).toBeGreaterThan(0);
      for (const entry of entries) expect('style' in entry.props, `${name} received style`).toBe(false);
    }
    expect(recorded('Stack')[0]!.props.className).toBe('k-stack');
    expect(recorded('Button')[0]!.props['data-k']).toBe('1');
    expect(recorded('Input').map((entry) => entry.props.label)).toEqual(['Name', 'Notes']);
    expect(recorded('Tooltip')[0]!.props.content).toBe('tip');
    expect(document.body.innerHTML).not.toContain(MARK);
  });

  it('a module loaded through the runtime host renders without the inline style it passed', async () => {
    const runtime = createRuntime();
    const container = await runtime.load(readFixture('ui-probe.js'), 'probe-style');
    const stack = container.querySelector('[data-testid="ui-probe-stack"]')!;
    const button = container.querySelector('[data-testid="ui-probe-button"]')!;
    expect(stack.className).toContain('flex');
    expect(button.className).toContain('probe-class');
    expect(container.innerHTML).not.toContain(MARK);
    expect(container.querySelector('[style]')).toBeNull();
    for (const entry of [...recorded('Stack'), ...recorded('Button')]) expect('style' in entry.props).toBe(false);
  });
});

// ---- toast ----

describe('module toast is plain text', () => {
  const cards = () => [...document.querySelectorAll<HTMLElement>('[data-toast-type]')];

  it('shows markup as text and never creates elements from it', () => {
    render(createElement(ToastViewport));
    const payload = '<b id="injected">bold</b><img src="x" onerror="globalThis.__pwned = 1">';
    act(() => {
      ui.toast.info(payload, { description: '<i id="injected-desc">d</i>' });
    });
    const [card] = cards();
    expect(card!.querySelectorAll('p')[0]!.textContent).toBe(payload);
    expect(card!.querySelectorAll('p')[1]!.textContent).toBe('<i id="injected-desc">d</i>');
    expect(document.querySelector('#injected, #injected-desc, img')).toBeNull();
  });

  it('turns a React element into text instead of rendering it', () => {
    render(createElement(ToastViewport));
    act(() => {
      ui.toast.success(createElement('b', { id: 'as-element' }, 'x') as never);
    });
    expect(document.querySelector('#as-element')).toBeNull();
    expect(cards()[0]!.querySelector('p')!.textContent).toBe('[object Object]');
  });

  it('ignores an action callback, so a module cannot put its own button into the host toast', () => {
    render(createElement(ToastViewport));
    const onClick = vi.fn();
    act(() => {
      ui.toast.warning('careful', { action: { label: 'Run', onClick } } as never);
    });
    const card = cards()[0]!;
    expect(within(card).queryByRole('button', { name: 'Run' })).toBeNull();
    expect(within(card).getAllByRole('button')).toHaveLength(1);
    expect(onClick).not.toHaveBeenCalled();
  });

  it('returns nothing, keeps the four types and shares the host queue limit of three', () => {
    render(createElement(ToastViewport));
    let results: unknown[] = [];
    act(() => {
      results = (['success', 'error', 'warning', 'info', 'info'] as const).map((type) => ui.toast[type](`m-${type}`));
    });
    expect(results.every((value) => value === undefined)).toBe(true);
    expect(cards().map((card) => card.getAttribute('data-toast-type'))).toEqual(['success', 'error', 'warning']);
    expect(cards().find((card) => card.getAttribute('data-toast-type') === 'error')!.getAttribute('role')).toBe('alert');
  });
});

// ---- modal ----

describe('module Modal commands', () => {
  it('confirm opens the host dialog, runs the callback once and closes', async () => {
    render(createElement(ModalHost));
    const onConfirm = vi.fn();
    act(() => {
      ui.Modal.confirm({ title: 'Delete it?', content: 'This cannot be undone.', confirmText: 'Yes', onConfirm });
    });
    const dialog = screen.getByRole('dialog', { name: 'Delete it?' });
    expect(within(dialog).getByRole('button', { name: '取消' })).toBeTruthy();
    fireEvent.click(within(dialog).getByRole('button', { name: 'Yes' }));
    await vi.waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(onConfirm).toHaveBeenCalledTimes(1);
  });

  it('alert and error have one button and call onOk', async () => {
    render(createElement(ModalHost));
    const onOk = vi.fn();
    act(() => {
      ui.Modal.error({ title: 'Failed', content: 'No luck.', onOk });
    });
    const dialog = screen.getByRole('dialog', { name: 'Failed' });
    expect(within(dialog).queryByRole('button', { name: '取消' })).toBeNull();
    fireEvent.click(within(dialog).getByRole('button', { name: '确定' }));
    await vi.waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(onOk).toHaveBeenCalledTimes(1);

    act(() => {
      ui.Modal.alert({ title: 'Heads up', okText: 'Got it' });
    });
    expect(within(screen.getByRole('dialog', { name: 'Heads up' })).getByRole('button', { name: 'Got it' })).toBeTruthy();
  });

  it('the returned handle can only close its own dialog', () => {
    render(createElement(ModalHost));
    let handle!: { close: () => void };
    act(() => {
      handle = ui.Modal.confirm({ title: 'Mine' });
      HostModal.confirm({ title: 'Host owned' });
    });
    expect(Object.keys(handle)).toEqual(['close']);
    expect(Object.isFrozen(handle)).toBe(true);
    act(() => handle.close());
    expect(screen.queryByRole('dialog', { name: 'Mine' })).toBeNull();
    expect(screen.getByRole('dialog', { name: 'Host owned' })).toBeTruthy();
  });

  it('rebuilds the options: entry fields the module supplies do not decide the dialog kind', () => {
    render(createElement(ModalHost));
    act(() => {
      ui.Modal.confirm({ title: 'Still a confirm', kind: 'alert', id: 'modal-1' } as never);
    });
    const dialog = screen.getByRole('dialog', { name: 'Still a confirm' });
    expect(within(dialog).getByRole('button', { name: '取消' })).toBeTruthy();
  });

  it('treats a callback that is not a function as absent', async () => {
    render(createElement(ModalHost));
    act(() => {
      ui.Modal.confirm({ title: 'Odd', onConfirm: 'run(evil)', onCancel: {} } as never);
    });
    fireEvent.click(within(screen.getByRole('dialog', { name: 'Odd' })).getByRole('button', { name: '确定' }));
    await vi.waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  });
});

// ---- provider-console on host.ui ----

const consoleDir = join(dirname(fileURLToPath(import.meta.url)), '..', 'bundled-modules', 'provider-console', '1.0.0');
const consoleSource = readFileSync(join(consoleDir, 'entry.js'), 'utf8');

describe('provider-console on host.ui', () => {
  const field = (container: HTMLElement, label: string) => within(container).getByLabelText(label) as HTMLInputElement;
  const fill = (container: HTMLElement, provider = 'p1', model = 'm1', prompt = 'hi') => {
    fireEvent.change(field(container, 'Provider id'), { target: { value: provider } });
    fireEvent.change(field(container, 'Model id'), { target: { value: model } });
    fireEvent.change(field(container, 'Prompt'), { target: { value: prompt } });
  };
  const send = (container: HTMLElement) => within(container).getByRole('button', { name: 'Send' }) as HTMLButtonElement;

  it('renders through the shared components and carries no inline style or own controls', async () => {
    const runtime = createRuntime();
    const container = await runtime.load(consoleSource, 'provider-console');

    expect(container.querySelector('[data-testid="provider-console"]')).not.toBeNull();
    expect(container.querySelector('[style]')).toBeNull();
    expect(recorded('Card')).toHaveLength(1);
    expect(recorded('Card.Header')).toHaveLength(1);
    expect(recorded('Card.Body')).toHaveLength(1);
    expect(recorded('Input').map((entry) => entry.props.label ?? entry.props['aria-label'])).toEqual([
      'Provider id',
      'Model id',
      'Prompt',
    ]);
    expect(recorded('Input')[2]!.props.multiline).toBe(true);
    expect(recorded('Button').map((entry) => entry.props.children)).toEqual(['Send']);
    // The controls are the shared ones: exactly two single-line fields, one textarea, one button.
    expect(container.querySelectorAll('input')).toHaveLength(2);
    expect(container.querySelectorAll('textarea')).toHaveLength(1);
    expect(container.querySelectorAll('button')).toHaveLength(1);
    expect(consoleSource).not.toMatch(/\bstyle\b/);
    expect(container.querySelector('[role="alert"]')).toBeNull();
  });

  it('sends only after all three fields are filled and shows the reply', async () => {
    const runtime = createRuntime();
    const container = await runtime.load(consoleSource, 'provider-console');
    expect(send(container).disabled).toBe(true);
    fireEvent.change(field(container, 'Provider id'), { target: { value: ' p1 ' } });
    fireEvent.change(field(container, 'Model id'), { target: { value: 'm1' } });
    expect(send(container).disabled).toBe(true);
    fireEvent.change(field(container, 'Prompt'), { target: { value: 'hello' } });
    expect(send(container).disabled).toBe(false);

    fireEvent.click(send(container));
    await vi.waitFor(() => expect(container.querySelector('[data-testid="provider-console-reply"]')?.textContent).toBe('pong'));
    expect(runtime.providerCalls).toHaveLength(1);
    expect(runtime.providerCalls[0]).toMatchObject({
      attachmentId: 'att-1',
      route: 'route-rt-provider-console',
      providerId: 'p1',
      modelId: 'm1',
      request: { messages: [{ role: 'user', content: 'hello' }], stream: false },
    });
  });

  it('shows the shared Loading while pending and cannot send twice', async () => {
    let release!: () => void;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    const runtime = createRuntime(async () => {
      await gate;
      return { success: true, data: { content: 'late' }, error: null };
    });
    const container = await runtime.load(consoleSource, 'provider-console');
    fill(container);
    fireEvent.click(send(container));

    await vi.waitFor(() => expect(within(container).getByRole('status').textContent).toContain('Waiting for the provider'));
    expect(recorded('Loading').length).toBeGreaterThan(0);
    expect(send(container).disabled).toBe(true);
    fireEvent.click(send(container));
    release();
    await vi.waitFor(() => expect(container.querySelector('[data-testid="provider-console-reply"]')?.textContent).toBe('late'));
    expect(runtime.providerCalls).toHaveLength(1);
    expect(within(container).queryByRole('status')).toBeNull();
  });

  it('reports a refused call as an alert with the host code and no reply', async () => {
    const runtime = createRuntime(async () => ({ success: false, data: null, error: { code: 1004004, message: 'denied' } }));
    const container = await runtime.load(consoleSource, 'provider-console');
    fill(container);
    fireEvent.click(send(container));
    await vi.waitFor(() => expect(within(container).getByRole('alert').textContent).toContain('1004004'));
    expect(container.querySelector('[data-testid="provider-console-reply"]')).toBeNull();
    expect(send(container).disabled).toBe(false);
  });
});
