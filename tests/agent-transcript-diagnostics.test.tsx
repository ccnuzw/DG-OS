import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import type { AgentEvent, Json } from '../src/stores/agentRunnerStore';
import { AgentTranscriptDiagnostics } from '../src/components/development/AgentTranscriptDiagnostics';

vi.mock('../src/i18n', () => ({ useLocale: () => ({ locale: 'en-US', t: (key: string, args?: Record<string, unknown>) => args ? `${key} ${Object.values(args).join(' ')}` : key }) }));
afterEach(cleanup);
const event = (seq: number, kind: string, data: Json = null, run: string | null = 'run-a', text: string | null = null): AgentEvent => ({ seq, kind, data, run, text });

it('keeps all native records behind one collapsed entry and mounts at most one page of evidence', async () => {
  const events = Array.from({ length: 1529 }, (_, index) => event(index + 1, index % 2 ? 'agentMessage' : 'native/unsupported', index % 2 ? { text: `Answer ${index}` } : { native_type: 'hook/completed', custom: `Evidence ${index}` }));
  const original = JSON.stringify(events);
  const view = render(<AgentTranscriptDiagnostics events={events} />);
  const diagnostics = screen.getByTestId('agent-transcript-diagnostics') as HTMLDetailsElement;
  expect(diagnostics.open).toBe(false);
  expect(screen.getByText('agentReply.diagnosticCount 1529 2')).toBeTruthy();
  expect(view.container.querySelectorAll('details')).toHaveLength(1);
  expect(view.container.querySelector('pre')).toBeNull();
  expect(screen.queryByRole('button')).toBeNull();

  fireEvent.click(screen.getByText('agentReply.diagnostics'));
  await waitFor(() => expect(view.container.querySelectorAll('[data-diagnostic-event]')).toHaveLength(50));
  expect(view.container.querySelector('pre')).toBeNull();
  expect(screen.getByText('agentReply.diagnosticPage 1 31')).toBeTruthy();
  expect(screen.getByText('agentReply.diagnosticRange 1 50 1529 1529')).toBeTruthy();
  expect((screen.getByRole('button', { name: 'agentReply.diagnosticPrevious' }) as HTMLButtonElement).disabled).toBe(true);
  for (const row of view.container.querySelectorAll('[data-diagnostic-event]')) fireEvent.click(row.querySelector('summary')!);
  await waitFor(() => expect(view.container.querySelectorAll('pre')).toHaveLength(50));
  expect(view.container.querySelector('pre[data-event-seq="51"]')).toBeNull();

  fireEvent.click(screen.getByRole('button', { name: 'agentReply.diagnosticNext' }));
  expect(screen.getByText('agentReply.diagnosticPage 2 31')).toBeTruthy();
  expect(view.container.querySelectorAll('[data-diagnostic-event]')).toHaveLength(50);
  expect(view.container.querySelector('[data-diagnostic-event]')?.getAttribute('data-event-seq')).toBe('51');
  expect(view.container.querySelector('pre')).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: 'agentReply.diagnosticPrevious' }));
  expect(view.container.querySelector('[data-diagnostic-event]')?.getAttribute('data-event-seq')).toBe('1');
  expect(JSON.stringify(events)).toBe(original);
});

it('summarizes kind and native_type separately and resets pagination when filtering', async () => {
  const events = [
    ...Array.from({ length: 60 }, (_, index) => event(index + 1, 'native/unsupported', { native_type: 'hook/started' })),
    ...Array.from({ length: 60 }, (_, index) => event(index + 61, 'native/unsupported', { native_type: 'hook/completed' })),
    event(121, 'agentMessage', { native_type: 'hook/completed', text: 'Visible native record' }),
  ];
  const view = render(<AgentTranscriptDiagnostics events={events} />);
  fireEvent.click(screen.getByText('agentReply.diagnostics'));
  const completed = await screen.findByRole('button', { name: /^native\/unsupported\s*\/\s*hook\/completed/ });
  expect(screen.getByText('agentReply.diagnosticCount 121 3')).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: 'agentReply.diagnosticNext' }));
  fireEvent.click(completed);
  expect(completed.getAttribute('aria-pressed')).toBe('true');
  expect(screen.getByText('agentReply.diagnosticPage 1 2')).toBeTruthy();
  expect(screen.getByText('agentReply.diagnosticRange 1 50 60 121')).toBeTruthy();
  expect(view.container.querySelector('[data-diagnostic-event]')?.getAttribute('data-event-seq')).toBe('61');
  expect(view.container.querySelectorAll('[data-diagnostic-event]')).toHaveLength(50);
  expect(view.container.querySelector('pre')).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: 'agentReply.diagnosticNext' }));
  expect(view.container.querySelectorAll('[data-diagnostic-event]')).toHaveLength(10);
  expect(screen.getByText('agentReply.diagnosticRange 51 60 60 121')).toBeTruthy();
  expect((screen.getByRole('button', { name: 'agentReply.diagnosticNext' }) as HTMLButtonElement).disabled).toBe(true);
  fireEvent.click(screen.getByRole('button', { name: /^agentMessage\s*\/\s*hook\/completed/ }));
  expect(view.container.querySelectorAll('[data-diagnostic-event]')).toHaveLength(1);
  expect(view.container.querySelector('[data-diagnostic-event]')?.getAttribute('data-event-seq')).toBe('121');
  fireEvent.click(screen.getByRole('button', { name: /^agentReply.diagnosticAllEvents/ }));
  expect(view.container.querySelector('[data-diagnostic-event]')?.getAttribute('data-event-seq')).toBe('1');
});

it('exposes safe original evidence per record without changing notices or the original events', async () => {
  const warning = event(1, 'native/unsupported', { native_type: 'warning', message: 'Native configuration needs attention.', content: [{ type: 'thinking', thinking: 'PRIVATE_THINKING', signature: 'PRIVATE_SIGNATURE' }] }, 'run-a', 'PRIVATE_TEXT');
  const reasoning = event(2, 'reasoning', { type: 'reasoning', text: 'PRIVATE_REASONING', encrypted_content: 'PRIVATE_ENCRYPTED' }, 'run-a', 'PRIVATE_EVENT');
  const unknown = event(3, 'native/unsupported', { payload_omitted: true, custom: 'Unknown evidence is retained' });
  const events = [warning, reasoning, unknown];
  const original = JSON.stringify(events);
  const view = render(<AgentTranscriptDiagnostics events={events} />);
  fireEvent.click(screen.getByText('agentReply.diagnostics'));
  await waitFor(() => expect(view.container.querySelectorAll('[data-diagnostic-event]')).toHaveLength(3));
  expect(view.container.querySelector('pre')).toBeNull();
  for (const row of view.container.querySelectorAll('[data-diagnostic-event]')) fireEvent.click(row.querySelector('summary')!);
  expect(await screen.findByText(/Native configuration needs attention/, { selector: 'pre' })).toBeTruthy();
  expect(await screen.findByText(/Unknown evidence is retained/, { selector: 'pre' })).toBeTruthy();
  await waitFor(() => expect(view.container.querySelectorAll('pre')).toHaveLength(3));
  expect(view.container.textContent).not.toMatch(/PRIVATE_/);
  expect(view.container.querySelector('[data-operation-state]')).toBeNull();
  expect(screen.queryByRole('alert')).toBeNull();
  expect(JSON.stringify(events)).toBe(original);
});

it('opens the exact run and page for a focus request, reveals evidence and supports repeated navigation', async () => {
  const events = [
    ...Array.from({ length: 60 }, (_, index) => event(index + 1, 'agentMessage', { text: `Run A ${index}` }, 'run-a')),
    ...Array.from({ length: 60 }, (_, index) => event(index + 1, 'native/unsupported', { native_type: 'hook/completed', custom: `Run B ${index}` }, 'run-b')),
  ];
  const focusEvent = { runId: 'run-b', seq: 57, nonce: 1 };
  const view = render(<AgentTranscriptDiagnostics events={events} />);
  fireEvent.click(screen.getByText('agentReply.diagnostics'));
  fireEvent.click(await screen.findByRole('button', { name: /^agentMessage/ }));
  view.rerender(<AgentTranscriptDiagnostics events={events} focusEvent={focusEvent} />);
  await waitFor(() => expect(view.container.querySelector('pre[data-event-run="run-b"][data-event-seq="57"]')).not.toBeNull());
  expect((screen.getByTestId('agent-transcript-diagnostics') as HTMLDetailsElement).open).toBe(true);
  expect(screen.getByText('agentReply.diagnosticPage 3 3')).toBeTruthy();
  expect(view.container.querySelectorAll('pre')).toHaveLength(1);
  const target = view.container.querySelector('[data-focused-diagnostic]') as HTMLDetailsElement;
  expect(target.open).toBe(true);
  expect(target.getAttribute('data-event-run')).toBe('run-b');
  expect(document.activeElement).toBe(target.querySelector('summary'));
  expect((screen.getByRole('textbox', { name: 'agentReply.diagnosticSequence' }) as HTMLInputElement).value).toBe('57');
  expect(screen.getByRole('combobox', { name: 'agentReply.diagnosticRun' }).textContent).toContain('run-b');

  fireEvent.click(screen.getByRole('button', { name: /^agentMessage/ }));
  const updatedEvents = [...events, event(61, 'agentMessage', null, 'run-a')];
  view.rerender(<AgentTranscriptDiagnostics events={updatedEvents} focusEvent={{ ...focusEvent }} />);
  expect(screen.getByRole('button', { name: /^agentMessage/ }).getAttribute('aria-pressed')).toBe('true');
  expect(view.container.querySelector('[data-focused-diagnostic]')).toBeNull();
  fireEvent.click(screen.getByText('agentReply.diagnostics'));
  await waitFor(() => expect((screen.getByTestId('agent-transcript-diagnostics') as HTMLDetailsElement).open).toBe(false));
  view.rerender(<AgentTranscriptDiagnostics events={updatedEvents} focusEvent={{ ...focusEvent, nonce: 2 }} />);
  await waitFor(() => expect(view.container.querySelector('pre[data-event-run="run-b"][data-event-seq="57"]')).not.toBeNull());
  expect(document.activeElement).toBe(view.container.querySelector('[data-focused-diagnostic] > summary'));
});

it('refuses ambiguous sequence matches until a run is selected and can locate outside the active type', async () => {
  const events = [
    ...Array.from({ length: 60 }, (_, index) => event(index + 1, 'agentMessage', { text: 'Run A evidence' }, 'run-a')),
    ...Array.from({ length: 60 }, (_, index) => event(index + 1, 'native/unsupported', { native_type: 'hook/completed', custom: 'Run B evidence' }, 'run-b')),
  ];
  const view = render(<AgentTranscriptDiagnostics events={events} />);
  fireEvent.click(screen.getByText('agentReply.diagnostics'));
  fireEvent.click(await screen.findByRole('button', { name: /^agentMessage/ }));
  const sequence = screen.getByRole('textbox', { name: 'agentReply.diagnosticSequence' });
  fireEvent.change(sequence, { target: { value: '55' } });
  fireEvent.click(screen.getByRole('button', { name: 'agentReply.diagnosticLocate' }));
  expect(screen.getByText('agentReply.diagnosticAmbiguousSequence 55')).toBeTruthy();
  expect(sequence.getAttribute('aria-invalid')).toBe('true');
  expect(view.container.querySelector('[data-focused-diagnostic]')).toBeNull();
  expect(view.container.querySelector('pre')).toBeNull();
  const run = screen.getByRole('combobox', { name: 'agentReply.diagnosticRun' });
  run.focus();
  fireEvent.keyDown(run, { key: 'Enter' });
  fireEvent.click(within(await screen.findByRole('listbox')).getByRole('option', { name: 'run-b' }));
  await waitFor(() => expect(screen.queryByRole('listbox')).toBeNull());
  await waitFor(() => expect(document.activeElement).toBe(run));
  fireEvent.click(screen.getByRole('button', { name: 'agentReply.diagnosticLocate' }));
  await waitFor(() => expect(view.container.querySelector('pre[data-event-run="run-b"][data-event-seq="55"]')).not.toBeNull());
  expect(screen.getByRole('button', { name: /^agentReply.diagnosticAllEvents/ }).getAttribute('aria-pressed')).toBe('true');
  expect(screen.getByText('agentReply.diagnosticPage 3 3')).toBeTruthy();
  expect(document.activeElement).toBe(view.container.querySelector('[data-focused-diagnostic] > summary'));
  fireEvent.change(sequence, { target: { value: '999' } });
  fireEvent.click(screen.getByRole('button', { name: 'agentReply.diagnosticLocate' }));
  expect(screen.getByText('agentReply.diagnosticNotFound 999')).toBeTruthy();
  expect(view.container.querySelector('[data-focused-diagnostic]')).toBeNull();
});

it('finds a unique sequence across pages and validates empty or non-integral input', async () => {
  const events = Array.from({ length: 75 }, (_, index) => event(index, 'message', { text: `Evidence ${index}` }, null));
  const view = render(<AgentTranscriptDiagnostics events={events} />);
  fireEvent.click(screen.getByText('agentReply.diagnostics'));
  const sequence = await screen.findByRole('textbox', { name: 'agentReply.diagnosticSequence' });
  for (const value of ['', '1.5', '-1', 'not-a-number', '9007199254740992']) {
    fireEvent.change(sequence, { target: { value } });
    fireEvent.click(screen.getByRole('button', { name: 'agentReply.diagnosticLocate' }));
    expect(screen.getByText('agentReply.diagnosticSequenceRequired')).toBeTruthy();
    expect(view.container.querySelector('[data-focused-diagnostic]')).toBeNull();
  }
  fireEvent.change(sequence, { target: { value: '74' } });
  fireEvent.submit(sequence.closest('form')!);
  await waitFor(() => expect(view.container.querySelector('pre[data-event-run=""][data-event-seq="74"]')).not.toBeNull());
  expect(screen.getByText('agentReply.diagnosticPage 2 2')).toBeTruthy();
  expect(view.container.querySelectorAll('[data-diagnostic-event]')).toHaveLength(25);
  fireEvent.change(sequence, { target: { value: '0' } });
  fireEvent.click(screen.getByRole('button', { name: 'agentReply.diagnosticLocate' }));
  await waitFor(() => expect(view.container.querySelector('pre[data-event-seq="0"]')).not.toBeNull());
  expect(screen.getByText('agentReply.diagnosticPage 1 2')).toBeTruthy();
});

it('reports exact-run not-found requests and resolves them when the requested event arrives', async () => {
  const focusEvent = { runId: 'run-b', seq: 7, nonce: 1 };
  const first = event(7, 'agentMessage', { text: 'Unrelated run' }, 'run-a');
  const view = render(<AgentTranscriptDiagnostics events={[first]} focusEvent={focusEvent} />);
  expect(await screen.findByText('agentReply.diagnosticNotFound 7')).toBeTruthy();
  expect(view.container.querySelector('[data-focused-diagnostic]')).toBeNull();
  expect(view.container.querySelector('pre')).toBeNull();
  const target = event(7, 'native/unsupported', { native_type: 'future/event', custom: 'Requested run evidence' }, 'run-b');
  view.rerender(<AgentTranscriptDiagnostics events={[first, target]} focusEvent={focusEvent} />);
  expect(await screen.findByText(/Requested run evidence/, { selector: 'pre' })).toBeTruthy();
  expect(view.container.querySelector('[data-focused-diagnostic]')?.getAttribute('data-event-run')).toBe('run-b');
  expect(screen.queryByText('agentReply.diagnosticNotFound 7')).toBeNull();
  view.rerender(<AgentTranscriptDiagnostics events={[]} focusEvent={focusEvent} />);
  expect(await screen.findByText('agentReply.diagnosticNotFound 7')).toBeTruthy();
  expect(screen.getByText('agentReply.diagnosticRange 0 0 0 0')).toBeTruthy();
  expect(view.container.querySelector('pre')).toBeNull();
});

it('preserves individual expansion and counts on append, clamps a shortened page and reports empty filters', async () => {
  const first = event(1, 'native/unsupported', { native_type: 'future/event', custom: 'First retained record' });
  const view = render(<AgentTranscriptDiagnostics events={[first]} />);
  fireEvent.click(screen.getByText('agentReply.diagnostics'));
  await waitFor(() => expect(view.container.querySelector('[data-diagnostic-event]')).not.toBeNull());
  fireEvent.click(view.container.querySelector('[data-diagnostic-event] > summary')!);
  expect(await screen.findByText(/First retained record/, { selector: 'pre' })).toBeTruthy();
  const second = event(2, 'message', { text: 'Second retained record' });
  view.rerender(<AgentTranscriptDiagnostics events={[first, second]} />);
  expect(screen.getByText('agentReply.diagnosticCount 2 2')).toBeTruthy();
  expect(view.container.querySelectorAll('pre')).toHaveLength(1);
  expect((view.container.querySelector('[data-diagnostic-event]') as HTMLDetailsElement).open).toBe(true);
  fireEvent.click(screen.getByRole('button', { name: /^native\/unsupported\s*\/\s*future\/event/ }));
  view.rerender(<AgentTranscriptDiagnostics events={[second]} />);
  expect(screen.getByText('agentReply.diagnosticEmpty')).toBeTruthy();
  expect(screen.getByText('agentReply.diagnosticRange 0 0 0 1')).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: /^agentReply.diagnosticAllEvents/ }));
  const longer = Array.from({ length: 75 }, (_, index) => event(index + 1, 'message'));
  view.rerender(<AgentTranscriptDiagnostics events={longer} />);
  fireEvent.click(screen.getByRole('button', { name: 'agentReply.diagnosticNext' }));
  expect(screen.getByText('agentReply.diagnosticPage 2 2')).toBeTruthy();
  view.rerender(<AgentTranscriptDiagnostics events={[second]} />);
  expect(screen.getByText('agentReply.diagnosticPage 1 1')).toBeTruthy();
  expect(view.container.querySelector('[data-diagnostic-event]')?.getAttribute('data-event-seq')).toBe('2');
  view.rerender(<AgentTranscriptDiagnostics events={[]} />);
  expect(view.container.childElementCount).toBe(0);
});

it('does not replay a consumed focus request when the conversation becomes visible again', async () => {
  const events = [event(1, 'message'), event(2, 'message')];
  const focusEvent = { runId: 'run-a', seq: 1, nonce: 1 };
  const view = render(<AgentTranscriptDiagnostics events={events} focusEvent={focusEvent} />);
  await waitFor(() => expect(view.container.querySelector('[data-focused-diagnostic]')).not.toBeNull());
  fireEvent.click(screen.getByText('agentReply.diagnostics'));
  await waitFor(() => expect(screen.queryByRole('textbox')).toBeNull());
  view.rerender(<AgentTranscriptDiagnostics events={events} />);
  view.rerender(<AgentTranscriptDiagnostics events={events} focusEvent={focusEvent} />);
  expect((screen.getByTestId('agent-transcript-diagnostics') as HTMLDetailsElement).open).toBe(false);
  view.rerender(<AgentTranscriptDiagnostics events={events} focusEvent={{ ...focusEvent, nonce: 2 }} />);
  await waitFor(() => expect(view.container.querySelector('[data-focused-diagnostic]')).not.toBeNull());
});

it('keeps keyboard focus on pagination after leaving a located record and returning to its page', async () => {
  const events = Array.from({ length: 60 }, (_, index) => event(index + 1, 'message'));
  const view = render(<AgentTranscriptDiagnostics events={events} focusEvent={{ runId: 'run-a', seq: 1, nonce: 1 }} />);
  await waitFor(() => expect(document.activeElement).toBe(view.container.querySelector('[data-focused-diagnostic] > summary')));
  const next = screen.getByRole('button', { name: 'agentReply.diagnosticNext' });
  next.focus(); fireEvent.click(next);
  const previous = screen.getByRole('button', { name: 'agentReply.diagnosticPrevious' });
  previous.focus(); fireEvent.click(previous);
  expect(document.activeElement).toBe(previous);
  expect(view.container.querySelector('[data-focused-diagnostic]')).toBeNull();
});
