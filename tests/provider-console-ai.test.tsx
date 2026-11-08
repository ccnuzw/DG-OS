// Evidence: jsdom, real bundled source + ModuleRuntimeHost/host.ui; scripted IPC only.
// The production WebView/local protocol smoke is intentionally a separate verification layer.
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
import { act, cleanup, fireEvent, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ModuleRuntimeHost, type HostInstruction, type Receipt } from '../src/modules/moduleRuntimeHost';
import { setLocaleForTesting } from '../src/i18n/runtime';
import { evaluateFixtureSource } from './helpers/moduleFixtures';
import type { AiEvent, AiRunInput, AiRunSummary, AiTerminal } from '../src/types/ai';

const source = readFileSync(join(dirname(fileURLToPath(import.meta.url)), '..', 'bundled-modules', 'provider-console', '1.1.0', 'entry.js'), 'utf8');
const skill = { skill_id: 's1', name: 'Outline', version: '2.0', content_hash: 'skill-hash' };
const tool = { server_id: 'server-1', server_name: 'Local tools', tool_name: 'echo', schema_hash: 'schema-hash', description: 'Echo selected text', input_schema: { type: 'object', properties: { text: { type: 'string' } } } };
const terminal: AiTerminal = { status: 'succeeded', content: 'done', error: null, result_unknown: false, audit_warning: false, total_tokens: 3 };
const summary = (id: string): AiRunSummary => ({ run_id: id, request_id: id, provider_id: 'p1', model_id: 'm1', created_at: 1, finished_at: 2, status: 'interrupted', error: null, result_unknown: true });
const hosts: ModuleRuntimeHost[] = [];

async function fixture(denyOptional = false) {
  const receipts: Receipt[] = [];
  const instruction = { onmessage: (_message: HostInstruction): void => undefined };
  const providers = [{ provider_id: 'p1', display_name: 'Local provider', config_revision: 0, models: [{ model_id: 'm1', display_name: 'Local model', kind: 'chat' }] }];
  const runs: Array<{ input: AiRunInput; channel: { onmessage: (event: AiEvent) => void }; resolve: (value: unknown) => void }> = [];
  let history = [summary('old-run')];
  const invoke = vi.fn(async (command: string, args?: Record<string, unknown>): Promise<unknown> => {
    const ok = (data: unknown) => ({ success: true, data, error: null });
    if (command === 'attach_module_host') return ok({ attachmentId: 'att-ai' });
    if (command === 'report_module_host') { receipts.push(args?.receipt as Receipt); return ok({ disposition: 'applied' }); }
    if (command === 'module_list_ai_providers') return ok(providers);
    if (command === 'module_list_ai_skills' || command === 'module_list_ai_tools') {
      if (denyOptional) return { success: false, data: null, error: { code: 1004004, message: 'denied' } };
      return ok(command === 'module_list_ai_skills' ? [skill] : [tool]);
    }
    if (command === 'module_list_ai_runs') return ok({ items: history, next_before: null });
    if (command === 'module_get_ai_run') return ok(summary(String(args?.runId)));
    if (command === 'module_cancel_ai_run' || command === 'cancel_module_call') return ok(true);
    if (command === 'module_run_ai') {
      const { promise, resolve } = Promise.withResolvers<unknown>();
      runs.push({ input: args?.input as AiRunInput, channel: args?.onEvent as { onmessage: (event: AiEvent) => void }, resolve });
      return promise;
    }
    throw new Error(`Unexpected command: ${command}`);
  });
  const host = new ModuleRuntimeHost({ invoke: invoke as never, createChannel: () => instruction,
    createAiChannel: () => ({ onmessage: (_event: AiEvent) => undefined }), createObjectUrl: () => 'blob:console', revokeObjectUrl: () => undefined,
    importModule: async () => evaluateFixtureSource(source) });
  hosts.push(host);
  await host.ensureAttached();
  instruction.onmessage({ kind: 'route', runtimeId: 'rt-console', route: 'route-console' });
  instruction.onmessage({ kind: 'load', ticket: 'load-console', runtimeId: 'rt-console', moduleId: 'provider-console', version: '1.1.0', source, hookTimeoutMs: 500, views: { layout: 'document', capability: 'multiple' } });
  await vi.waitFor(() => expect(receipts).toEqual([{ kind: 'loaded' }]));
  const container = (await host.openView('rt-console', 'default-view', 'default-document')).container;
  document.body.appendChild(container);
  const emit = (index: number, event: AiEvent) => act(() => runs[index].channel.onmessage(event));
  const finish = async (index: number, result = terminal) => {
    history = [summary(`run-${index}`), ...history];
    await act(async () => runs[index].resolve({ success: true, data: result, error: null }));
  };
  return { host, container, invoke, runs, providers, emit, finish };
}

async function fill(container: HTMLElement, prompt = 'hello') {
  for (const [label, name] of [['Provider', 'Local provider'], ['Model', 'Local model']]) {
    await vi.waitFor(() => expect((within(container).getByRole('combobox', { name: label }) as HTMLButtonElement).disabled).toBe(false));
    fireEvent.click(within(container).getByRole('combobox', { name: label }));
    fireEvent.click(await screen.findByRole('option', { name }));
  }
  fireEvent.change(within(container).getByLabelText('Prompt'), { target: { value: prompt } });
}
const click = (container: HTMLElement, name: string) => fireEvent.click(within(container).getByRole('button', { name }));
const started = (id: string): AiEvent => ({ kind: 'started', run_id: id, request_id: id, seq: 1, provider_revision: 0, skill_refs: [], tools: [] });
beforeEach(() => setLocaleForTesting('en-US'));
afterEach(() => { act(() => { for (const host of hosts.splice(0)) host.resetViews(); }); cleanup(); document.body.replaceChildren(); });

describe('provider-console 1.1 public AI consumer', () => {
  it('sends exact revision zero, selected content pins and tool schemas, with streamed private output and host approval phase', async () => {
    const f = await fixture();
    await fill(f.container);
    fireEvent.click(within(f.container).getByRole('checkbox', { name: 'Outline (2.0)' }));
    fireEvent.click(within(f.container).getByRole('checkbox', { name: 'Local tools: echo' }));
    expect(f.container.textContent).toContain('schema-hash');
    click(f.container, 'Send');
    await vi.waitFor(() => expect(f.runs).toHaveLength(1));
    expect(f.runs[0].input).toEqual({ provider_id: 'p1', model_id: 'm1', provider_revision: 0, prompt: 'hello',
      skill_refs: [{ skill_id: 's1', version: '2.0', content_hash: 'skill-hash' }], tools: [{ server_id: 'server-1', tool_name: 'echo', schema_hash: 'schema-hash' }], generation: {} });
    f.emit(0, started('run-0'));
    f.emit(0, { kind: 'delta', run_id: 'run-0', request_id: 'run-0', seq: 2, text: 'partial' });
    expect(within(f.container).getByTestId('provider-console-reply').textContent).toBe('partial');
    f.emit(0, { kind: 'phase', run_id: 'run-0', request_id: 'run-0', seq: 3, phase: 'awaiting_confirmation' });
    expect(within(f.container).getByRole('status').textContent).toContain('host approval');
    expect(f.invoke.mock.calls.some(([command]) => command.includes('resolve') || command.includes('grant'))).toBe(false);
    await f.finish(0);
    expect(within(f.container).getByTestId('provider-console-reply').textContent).toBe('done');
    expect(f.invoke.mock.calls.some(([command]) => command === 'module_call_provider')).toBe(false);
  });

  it('keeps optional permission denial visible while allowing a plain model run without granting anything', async () => {
    const f = await fixture(true);
    await fill(f.container);
    await vi.waitFor(() => expect(within(f.container).getAllByRole('alert')).toHaveLength(2));
    click(f.container, 'Send');
    await vi.waitFor(() => expect(f.runs).toHaveLength(1));
    expect(f.runs[0].input.skill_refs).toEqual([]);
    expect(f.runs[0].input.tools).toEqual([]);
    await f.finish(0);
  });

  it('does not settle on cancellation acknowledgement and retains an unknown result for status lookup', async () => {
    const f = await fixture();
    await fill(f.container);
    click(f.container, 'Send');
    await vi.waitFor(() => expect(f.runs).toHaveLength(1));
    f.emit(0, started('run-0'));
    click(f.container, 'Cancel run');
    await vi.waitFor(() => expect(f.invoke.mock.calls.some(([command]) => command === 'module_cancel_ai_run')).toBe(true));
    expect((within(f.container).getByRole('button', { name: 'Send' }) as HTMLButtonElement).disabled).toBe(true);
    expect(within(f.container).getByRole('status').textContent).toContain('Waiting for the final state');
    await f.finish(0, { ...terminal, status: 'interrupted', content: null, result_unknown: true });
    expect(within(f.container).getByRole('alert').textContent).toContain('external result is unknown');
    fireEvent.click(within(f.container).getAllByRole('button', { name: 'Check status' })[0]);
    await vi.waitFor(() => expect(f.invoke.mock.calls.some(([command, args]) => command === 'module_get_ai_run' && args?.runId === 'run-0')).toBe(true));
    expect(f.runs).toHaveLength(1);
  });

  it('preserves draft through locale change and requires explicit reselection after refresh', async () => {
    const f = await fixture();
    await fill(f.container, 'my draft');
    act(() => setLocaleForTesting('zh-CN'));
    expect((within(f.container).getByLabelText('提示词') as HTMLInputElement).value).toBe('my draft');
    act(() => setLocaleForTesting('en-US'));
    click(f.container, 'Refresh selections');
    expect((within(f.container).getByLabelText('Prompt') as HTMLInputElement).value).toBe('my draft');
    expect((within(f.container).getByRole('button', { name: 'Send' }) as HTMLButtonElement).disabled).toBe(true);
  });

  it('isolates document output, blocks close until terminal, and does not cancel a hidden document', async () => {
    const f = await fixture();
    const view = await f.host.openView('rt-console', 'view-a', 'document-a');
    document.body.appendChild(view.container);
    await fill(view.container, 'document prompt');
    click(view.container, 'Send');
    await vi.waitFor(() => expect(f.runs).toHaveLength(1));
    f.emit(0, started('run-0'));
    f.emit(0, { kind: 'delta', run_id: 'run-0', request_id: 'run-0', seq: 2, text: 'document-only' });
    expect(within(f.container).queryByTestId('provider-console-reply')).toBeNull();
    expect(await f.host.prepareViewClose('rt-console', 'view-a')).toMatchObject({ dirty: true, blocked: true });
    view.container.remove();
    expect(f.invoke.mock.calls.some(([command]) => command === 'module_cancel_ai_run' || command === 'cancel_module_call')).toBe(false);
    await f.finish(0);
    expect(await f.host.prepareViewClose('rt-console', 'view-a')).toMatchObject({ dirty: true, blocked: false });
  });

  it('reports admission refusal without replay and lets the user retain the draft', async () => {
    const f = await fixture();
    await fill(f.container, 'retain this draft');
    click(f.container, 'Send');
    await vi.waitFor(() => expect(f.runs).toHaveLength(1));
    await act(async () => f.runs[0].resolve({ success: false, data: null, error: { code: 1004004, message: 'denied' } }));
    expect(within(f.container).getByRole('alert').textContent).toContain('1004004');
    expect((within(f.container).getByLabelText('Prompt') as HTMLInputElement).value).toBe('retain this draft');
    expect(within(f.container).queryByTestId('provider-console-reply')).toBeNull();
    expect(f.runs).toHaveLength(1);
  });

  it('cancels before Started by the original call and treats a lost result as unknown even without a run id', async () => {
    const f = await fixture();
    await fill(f.container);
    click(f.container, 'Send');
    await vi.waitFor(() => expect(f.runs).toHaveLength(1));
    click(f.container, 'Cancel run');
    await vi.waitFor(() => expect(f.invoke.mock.calls.some(([command]) => command === 'cancel_module_call')).toBe(true));
    await act(async () => f.runs[0].resolve({ success: false, data: null, error: { code: 1000000, message: 'lost response' } }));
    expect(within(f.container).getAllByRole('alert').some(alert => alert.textContent?.includes('external result is unknown'))).toBe(true);
    click(f.container, 'Refresh history');
    await vi.waitFor(() => expect(f.invoke.mock.calls.filter(([command]) => command === 'module_list_ai_runs').length).toBeGreaterThan(1));
    expect(f.runs).toHaveLength(1);
  });
});
