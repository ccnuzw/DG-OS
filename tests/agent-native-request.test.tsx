import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { openUrl } from '@tauri-apps/plugin-opener';
import { Request } from '../src/components/development/AgentNativeRequest';
import { NativeElicitation, elicitationDefaults, elicitationValid } from '../src/components/development/NativeElicitation';
import type { AgentEvent, Json, Pending } from '../src/stores/agentRunnerStore';

vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl: vi.fn(async () => {}) }));
vi.mock('../src/i18n', () => ({ useLocale: () => ({ t: (key: string) => key }), t: (key: string) => key, formatDateTime: String }));
afterEach(() => { cleanup(); vi.clearAllMocks(); });

const claudeQuestions: Pending = { id: 'questions', kind: 'claude/toolPermission', title: 'AskUserQuestion', data: { input: { questions: [
  { question: 'Which branch?', header: 'Branch', options: [{ label: 'main' }, { label: 'release' }] },
  { question: 'Which checks?', header: 'Checks', multiSelect: true, options: [{ label: 'types' }, { label: 'unit' }] },
] } } };

it('keeps Claude approval disabled until every question has a nonblank answer and links each error to its field', async () => {
  const submit = vi.fn(async () => true);
  render(<Request engine="claude" pending={claudeQuestions} submit={submit} />);
  const answer = screen.getByRole('button', { name: 'agent.answer' }) as HTMLButtonElement;
  expect(answer.disabled).toBe(true);
  const branch = screen.getByRole('group', { name: 'Branch' });
  expect(branch.getAttribute('aria-invalid')).toBe('true');
  const input = within(branch).getByRole('textbox');
  const described = input.getAttribute('aria-describedby')?.split(' ') ?? [];
  expect(described.length).toBeGreaterThan(0);
  expect(described.every(id => document.getElementById(id))).toBe(true);
  fireEvent.click(answer);
  expect(submit).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole('button', { name: 'release' }));
  expect(answer.disabled).toBe(true);
  expect(branch.hasAttribute('aria-invalid')).toBe(false);
  fireEvent.change(within(screen.getByRole('group', { name: 'Checks' })).getByRole('textbox'), { target: { value: '  ' } });
  expect(answer.disabled).toBe(true);
  fireEvent.change(within(screen.getByRole('group', { name: 'Checks' })).getByRole('textbox'), { target: { value: '' } });
  fireEvent.click(screen.getByRole('button', { name: 'types' }));
  fireEvent.click(screen.getByRole('button', { name: 'unit' }));
  expect(answer.disabled).toBe(false);
  fireEvent.click(answer);
  await waitFor(() => expect(submit).toHaveBeenCalledExactlyOnceWith({ behavior: 'allow', answers: { 'Which branch?': 'release', 'Which checks?': 'types, unit' } }));
});

it('preserves answers after a rejected submission and never retries until the user acts again', async () => {
  const response = Promise.withResolvers<boolean>();
  const submit = vi.fn().mockReturnValueOnce(response.promise).mockResolvedValueOnce(true);
  render(<Request engine="claude" pending={claudeQuestions} submit={submit} />);
  fireEvent.click(screen.getByRole('button', { name: 'main' }));
  fireEvent.click(screen.getByRole('button', { name: 'types' }));
  const answer = screen.getByRole('button', { name: 'agent.answer' });
  fireEvent.click(answer);
  fireEvent.click(answer);
  expect(submit).toHaveBeenCalledTimes(1);
  expect(screen.getByTestId('agent-native-request').getAttribute('aria-busy')).toBe('true');
  await act(async () => response.reject(new Error('transport failed')));
  expect((await screen.findByRole('alert')).textContent).toBe('agentRequest.sendFailed');
  expect((within(screen.getByRole('group', { name: 'Branch' })).getByRole('textbox') as HTMLInputElement).value).toBe('main');
  expect((within(screen.getByRole('group', { name: 'Checks' })).getByRole('textbox') as HTMLInputElement).value).toBe('types');
  expect(submit).toHaveBeenCalledTimes(1);
  fireEvent.click(answer);
  await waitFor(() => expect(submit).toHaveBeenCalledTimes(2));
  expect(submit.mock.calls[1]).toEqual(submit.mock.calls[0]);
});

it.each([{ questions: [] }, { questions: [{ question: 'Repeated?' }, { question: 'Repeated?' }] }, { questions: [{ header: 'Missing question' }] }])('declines malformed Claude question identities without inventing an answer map: $questions', async ({ questions }) => {
  const submit = vi.fn(async () => true);
  render(<Request engine="claude" pending={{ ...claudeQuestions, data: { input: { questions } } }} submit={submit} />);
  expect((screen.getByRole('button', { name: 'agent.answer' }) as HTMLButtonElement).disabled).toBe(true);
  fireEvent.click(screen.getByRole('button', { name: 'agent.deny' }));
  await waitFor(() => expect(submit).toHaveBeenCalledExactlyOnceWith({ behavior: 'deny' }));
});

it.each([false, true])('preserves the Codex contract for empty or partial answers (partial=%s)', async partial => {
  const submit = vi.fn(async () => true);
  render(<Request engine="codex" pending={{ id: 'codex-question', kind: 'item/tool/requestUserInput', title: 'Question', data: { questions: [{ id: 'first', question: 'First?' }, { id: 'second', question: 'Second?' }] } }} submit={submit} />);
  if (partial) fireEvent.change(within(screen.getByRole('group', { name: 'First?' })).getByRole('textbox'), { target: { value: 'one' } });
  fireEvent.click(screen.getByRole('button', { name: 'agent.answer' }));
  await waitFor(() => expect(submit).toHaveBeenCalledExactlyOnceWith({ answers: partial ? { first: { answers: ['one'] } } : {} }));
});

it('clears a selected optional enum and sends the field as omitted', async () => {
  const submit = vi.fn(async () => true);
  render(<Request engine="codex" pending={{ id: 'enum', kind: 'mcpServer/elicitation/request', title: 'Form', data: { requestedSchema: { type: 'object', properties: { choice: { type: 'string', title: 'Choice', enum: ['alpha,beta', 'gamma'] } } } } }} submit={submit} />);
  fireEvent.click(screen.getByRole('radio', { name: 'alpha,beta' }));
  fireEvent.click(screen.getByRole('radio', { name: 'agent.unanswered' }));
  fireEvent.click(screen.getByRole('button', { name: 'agent.allow' }));
  await waitFor(() => expect(submit).toHaveBeenCalledExactlyOnceWith({ action: 'accept', content: {} }));
});

it('shows constraints and number errors beside the field with live accessible descriptions', () => {
  const schema = { type: 'object', required: ['count'], properties: { count: { type: 'integer', title: 'Count', description: 'Number of copies', minimum: 1, maximum: 3 } } };
  const onChange = vi.fn();
  const view = render(<NativeElicitation schema={schema} values={{ count: 4 }} disabled={false} onChange={onChange} />);
  const input = screen.getByRole('textbox', { name: /Count/ });
  expect(input.getAttribute('aria-invalid')).toBe('true');
  const ids = input.getAttribute('aria-describedby')?.split(' ') ?? [];
  expect(ids.length).toBe(2);
  expect(ids.every(id => document.getElementById(id))).toBe(true);
  expect(document.body.textContent).toContain('agentRequest.constraint.minimum');
  expect(document.body.textContent).toContain('agentRequest.validation.maximum');
  fireEvent.change(input, { target: { value: '1e-' } });
  view.rerender(<NativeElicitation schema={schema} values={{ count: '1e-' }} disabled={false} onChange={onChange} />);
  expect((input as HTMLInputElement).value).toBe('1e-');
  expect(onChange).toHaveBeenLastCalledWith({ count: '1e-' });
  view.rerender(<NativeElicitation schema={schema} values={{ count: 2 }} disabled={false} onChange={onChange} />);
  expect(screen.getByRole('textbox', { name: /Count/ }).hasAttribute('aria-invalid')).toBe(false);
  expect((screen.getByRole('textbox', { name: /Count/ }) as HTMLInputElement).value).toBe('2');
});

it('refuses unknown required constraints and root rules but allows unsupported optional fields to be omitted', () => {
  const properties = { optional: { type: 'string', pattern: '^x$' }, count: { type: 'integer', minimum: 1 } };
  const schema = { type: 'object', properties, required: ['count'] };
  expect(elicitationValid(schema, { count: 1 })).toBe(true);
  expect(elicitationValid(schema, { count: 1, optional: 'x' })).toBe(false);
  expect(elicitationValid({ ...schema, required: ['count', 'optional'] }, { count: 1 })).toBe(false);
  expect(elicitationValid({ ...schema, allOf: [] }, { count: 1 })).toBe(false);
  expect(elicitationValid(schema, { count: 1, extra: true })).toBe(false);
  expect(elicitationValid({ type: 'object', properties: {}, required: ['unknown'] }, {})).toBe(false);
  render(<Request engine="codex" pending={{ id: 'unsupported', kind: 'mcpServer/elicitation/request', title: 'Form', data: { mode: 'form', requestedSchema: { ...schema, required: ['optional'] } } }} submit={vi.fn()} />);
  expect((screen.getByRole('button', { name: 'agent.allow' }) as HTMLButtonElement).disabled).toBe(true);
  expect(screen.getByRole('alert').textContent).toBe('agentRequest.fieldUnsupportedRequired');
});

it('only initializes valid supported defaults and preserves null omission rules', () => {
  const schema = { type: 'object', required: null, properties: { enabled: { type: 'boolean', default: false }, count: { type: 'integer', default: 4, minimum: null, maximum: 3 }, future: { type: 'string', pattern: '.', default: 'x' }, optional: { type: 'string', default: null } } };
  expect(elicitationDefaults(schema)).toEqual({ enabled: false });
  expect(elicitationValid(schema, { enabled: false })).toBe(true);
  expect(elicitationValid({ type: 'object', properties: { field: { type: 'string', enum: ['a', 'a'] } }, required: ['field'] }, { field: 'a' })).toBe(false);
  expect(elicitationValid({ type: 'object', properties: { field: { type: 'array', items: { type: 'string', enum: ['a'] }, minItems: -1 } }, required: ['field'] }, { field: [] })).toBe(false);
  expect(elicitationValid({ type: 'object', properties: { field: { type: 'string', minLength: 9007199254740992 } }, required: ['field'] }, { field: '' })).toBe(false);
});

it.each([
  ['email', 'name@example.com', 'name@invalid'],
  ['uri', 'https://example.com/review', 'relative/path'],
  ['date', '2024-02-29', '2023-02-29'],
  ['date-time', '2024-02-29t23:59:01.123+08:00', '2024-02-29T23:59:60Z'],
])('matches native %s format validation without coercion', (format, valid, invalid) => {
  const schema = { type: 'object', properties: { field: { type: 'string', format } }, required: ['field'] };
  expect(elicitationValid(schema, { field: valid })).toBe(true);
  expect(elicitationValid(schema, { field: invalid })).toBe(false);
});

it('counts Unicode characters and bounds payload size using the host estimate', () => {
  const schema = { type: 'object', properties: { field: { type: 'string', minLength: 1, maxLength: 1 } }, required: ['field'] };
  expect(elicitationValid(schema, { field: '🧭' })).toBe(true);
  expect(elicitationValid(schema, { field: 'ab' })).toBe(false);
  expect(elicitationValid({ type: 'object', properties: { field: { type: 'string' } } }, { field: 'x'.repeat(12000) })).toBe(false);
});

it('opens URL elicitation through explicit confirmation and does not submit acceptance after opening', async () => {
  const submit = vi.fn(async () => true);
  render(<Request engine="codex" pending={{ id: 'url', kind: 'mcpServer/elicitation/request', title: 'Review', data: { mode: 'url', url: 'https://example.invalid/review' } }} submit={submit} />);
  fireEvent.click(screen.getByRole('button', { name: 'agentRequest.openExternal' }));
  expect(openUrl).not.toHaveBeenCalled();
  expect(submit).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole('button', { name: 'chat.open' }));
  await waitFor(() => expect(openUrl).toHaveBeenCalledExactlyOnceWith('https://example.invalid/review'));
  expect(submit).not.toHaveBeenCalled();
  fireEvent.click(await screen.findByRole('button', { name: 'agent.allow' }));
  await waitFor(() => expect(submit).toHaveBeenCalledExactlyOnceWith({ action: 'accept', content: null }));
});

it.each(['http://example.invalid/', 'javascript:alert(1)', 'https://user:pass@example.invalid/', 'https://example.invalid:444/'])('does not offer an external opener for unsafe address %s', url => {
  render(<Request engine="codex" pending={{ id: 'url', kind: 'mcpServer/elicitation/request', title: 'Review', data: { mode: 'url', url } }} submit={vi.fn()} />);
  expect(screen.queryByRole('button', { name: 'agentRequest.openExternal' })).toBeNull();
  expect(openUrl).not.toHaveBeenCalled();
});

it.each(['openai/form', 'openaiForm', 'verification', null])('never accepts unsupported elicitation mode %s', mode => {
  render(<Request engine="codex" pending={{ id: 'mode', kind: 'mcpServer/elicitation/request', title: 'Form', data: { mode, requestedSchema: { type: 'object', properties: {} } } }} submit={vi.fn()} />);
  expect((screen.getByRole('button', { name: 'agent.allow' }) as HTMLButtonElement).disabled).toBe(true);
});

it('renders only the matching active-run file proposal and ignores an older run with a reused native item ID', () => {
  const pending: Pending = { id: 'file', kind: 'item/fileChange/requestApproval', title: 'Write', data: { itemId: 'native-file', threadId: 'thread', turnId: 'turn', grantRoot: '/project' } };
  const event = (seq: number, run: string, data: Json): AgentEvent => ({ seq, kind: 'item/started', text: null, run, data });
  const events = [event(1, 'old', { type: 'fileChange', id: 'native-file', changes: [{ path: '/project/old', diff: '-PRIVATE_OLD\n+old' }] }), event(2, 'active', { type: 'fileChange', id: 'native-file', changes: [{ path: '/project/new', kind: { type: 'update' }, diff: '@@ -1 +1 @@\n-before\n+after' }] })];
  const view = render(<Request engine="codex" pending={pending} events={events} activeRun="active" submit={vi.fn()} />);
  expect(document.body.textContent).toContain('/project/new');
  expect(screen.getAllByRole('region', { name: 'execution.diff' }).length).toBeGreaterThan(0);
  expect(document.body.textContent).not.toContain('PRIVATE_OLD');
  view.rerender(<Request engine="codex" pending={pending} events={events} activeRun="missing" submit={vi.fn()} />);
  expect(screen.queryByRole('region', { name: 'execution.diff' })).toBeNull();
  expect(document.body.textContent).toContain('agentRequest.diffUnavailable');
});

it('shows readable permission scope while echoing the exact native grant', async () => {
  const submit = vi.fn(async () => true);
  const permissions = { network: { enabled: true }, fileSystem: { read: ['/project/input'], entries: [{ path: { type: 'special', value: { kind: 'project_roots', subpath: 'src' } }, access: 'write' }], globScanMaxDepth: 3 } };
  render(<Request engine="codex" pending={{ id: 'permission', kind: 'item/permissions/requestApproval', title: 'Permissions', data: { permissions, cwd: '/project' } }} submit={submit} />);
  expect(document.body.textContent).toContain('agentRequest.networkOn');
  expect(document.body.textContent).toContain('/project/input');
  expect(document.body.textContent).toContain('agentRequest.path.project_roots');
  fireEvent.click(screen.getByRole('button', { name: 'agent.allowTurn' }));
  await waitFor(() => expect(submit).toHaveBeenCalledExactlyOnceWith({ permissions, scope: 'turn' }));
});

it('puts a command, workspace and managed network destination before its exact native approval', async () => {
  const submit = vi.fn(async () => true);
  render(<Request engine="codex" pending={{ id: 'command', kind: 'item/commandExecution/requestApproval', title: 'Command', data: { command: 'curl https://example.invalid/', cwd: '/project', networkApprovalContext: { protocol: 'https', host: 'example.invalid' }, availableDecisions: ['accept', 'decline'] } }} submit={submit} />);
  expect(document.body.textContent).toContain('curl https://example.invalid/');
  expect(document.body.textContent).toContain('/project');
  expect(screen.getByText('https://example.invalid')).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: 'agent.allow' }));
  await waitFor(() => expect(submit).toHaveBeenCalledExactlyOnceWith({ decision: 'accept' }));
});

it('retains a form answer after the host returns false and sends the same content on an explicit retry', async () => {
  const submit = vi.fn().mockResolvedValueOnce(false).mockResolvedValueOnce(true);
  render(<Request engine="codex" pending={{ id: 'form-retry', kind: 'mcpServer/elicitation/request', title: 'Form', data: { mode: 'form', requestedSchema: { type: 'object', properties: { name: { type: 'string', title: 'Name', minLength: 1 } }, required: ['name'] } } }} submit={submit} />);
  fireEvent.change(screen.getByRole('textbox', { name: /Name/ }), { target: { value: 'retained' } });
  fireEvent.click(screen.getByRole('button', { name: 'agent.allow' }));
  expect((await screen.findByRole('alert')).textContent).toBe('agentRequest.sendFailed');
  expect((screen.getByRole('textbox', { name: /Name/ }) as HTMLInputElement).value).toBe('retained');
  expect(submit).toHaveBeenCalledTimes(1);
  fireEvent.click(screen.getByRole('button', { name: 'agent.allow' }));
  await waitFor(() => expect(submit).toHaveBeenCalledTimes(2));
  expect(submit).toHaveBeenLastCalledWith({ action: 'accept', content: { name: 'retained' } });
});

it('allows explicit empty arrays but can omit a previously answered optional array', async () => {
  const submit = vi.fn(async () => true);
  render(<Request engine="codex" pending={{ id: 'empty-list', kind: 'mcpServer/elicitation/request', title: 'Form', data: { requestedSchema: { type: 'object', properties: { tags: { type: 'array', title: 'Tags', items: { anyOf: [{ const: 'a', title: 'Alpha' }] } } } } } }} submit={submit} />);
  fireEvent.click(screen.getByRole('button', { name: 'agentRequest.useEmptyList' }));
  fireEvent.click(screen.getByRole('checkbox', { name: 'Alpha' }));
  fireEvent.click(screen.getByRole('button', { name: 'agentRequest.clearOptional' }));
  fireEvent.click(screen.getByRole('button', { name: 'agent.allow' }));
  await waitFor(() => expect(submit).toHaveBeenCalledExactlyOnceWith({ action: 'accept', content: {} }));
});

it('does not show a native file proposal carrying a different turn even if its item and run match', () => {
  const events: AgentEvent[] = [{ seq: 1, kind: 'fileChange', run: 'run', text: null, data: { id: 'item', type: 'fileChange', turnId: 'other', changes: [{ path: '/other', diff: '-wrong\n+turn' }] } }];
  render(<Request engine="codex" pending={{ id: 'file', kind: 'item/fileChange/requestApproval', title: 'File', data: { itemId: 'item', turnId: 'expected' } }} events={events} activeRun="run" submit={vi.fn()} />);
  expect(document.body.textContent).not.toContain('/other');
  expect(screen.queryByRole('region', { name: 'execution.diff' })).toBeNull();
});

it.each([
  { questions: [{ question: 'Missing id?' }] },
  { questions: [{ id: 'same', question: 'First?' }, { id: 'same', question: 'Second?' }] },
  {},
] as Json[])('never invents or merges Codex question identifiers: %j', async data => {
  const submit = vi.fn(async () => true);
  render(<Request engine="codex" pending={{ id: 'invalid-codex', kind: 'item/tool/requestUserInput', title: 'Question', data }} submit={submit} />);
  expect(screen.getByRole('alert').textContent).toBe('agentRequest.invalidQuestions');
  expect(screen.getByRole('button', { name: 'agent.answer' })).toHaveProperty('disabled', true);
  expect(screen.queryAllByRole('textbox').every(input => (input as HTMLInputElement).disabled || input.closest('fieldset')?.disabled)).toBe(true);
  fireEvent.click(screen.getByRole('button', { name: 'agent.deny' }));
  await waitFor(() => expect(submit).toHaveBeenCalledExactlyOnceWith({ answers: {} }));
});

it('retains an empty native Codex identifier instead of replacing it with a display index', async () => {
  const submit = vi.fn(async () => true);
  render(<Request engine="codex" pending={{ id: 'empty-id', kind: 'item/tool/requestUserInput', title: 'Question', data: { questions: [{ id: '', question: 'Native empty id?' }] } }} submit={submit} />);
  fireEvent.change(screen.getByRole('textbox'), { target: { value: 'answer' } });
  fireEvent.click(screen.getByRole('button', { name: 'agent.answer' }));
  await waitFor(() => expect(submit).toHaveBeenCalledExactlyOnceWith({ answers: { '': { answers: ['answer'] } } }));
});

it.each(['claude', 'codex'] as const)('isolates %s answers and in-flight failures when a session generation changes', async engine => {
  const response = Promise.withResolvers<boolean>();
  const submit = vi.fn().mockReturnValueOnce(response.promise).mockResolvedValueOnce(true);
  const pending: Pending = engine === 'claude' ? claudeQuestions : { id: 'questions', kind: 'item/tool/requestUserInput', title: 'Question', data: { questions: [{ id: 'branch', question: 'Branch?', options: [{ label: 'main' }] }] } };
  const view = render(<Request engine={engine} sessionId="session" generation="old" pending={pending} submit={submit} />);
  fireEvent.click(screen.getByRole('button', { name: 'main' }));
  if (engine === 'claude') fireEvent.click(screen.getByRole('button', { name: 'types' }));
  const answer = screen.getByRole('button', { name: 'agent.answer' });
  act(() => {
    answer.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    answer.dispatchEvent(new MouseEvent('click', { bubbles: true }));
  });
  expect(submit).toHaveBeenCalledTimes(1);
  view.rerender(<Request engine={engine} sessionId="session" generation="new" pending={pending} submit={submit} />);
  expect(screen.getByTestId('agent-native-request').getAttribute('aria-busy')).toBe('false');
  expect(screen.getByRole('button', { name: 'main' }).getAttribute('aria-pressed')).toBe('false');
  if (engine === 'claude') expect(screen.getByRole('button', { name: 'agent.answer' })).toHaveProperty('disabled', true);
  await act(async () => response.resolve(false));
  expect(screen.queryByText('agentRequest.sendFailed')).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: 'main' }));
  if (engine === 'claude') fireEvent.click(screen.getByRole('button', { name: 'types' }));
  fireEvent.click(screen.getByRole('button', { name: 'agent.answer' }));
  await waitFor(() => expect(submit).toHaveBeenCalledTimes(2));
});

it('shows the native form purpose and preserves numeric drafts only for the corresponding controlled value', () => {
  const schema = { type: 'object', title: 'Deployment settings', description: 'Choose the requested deployment settings.', properties: { count: { type: 'number', title: 'Count' } } };
  const onChange = vi.fn();
  const view = render(<NativeElicitation schema={schema} values={{ count: 1 }} disabled={false} onChange={onChange} />);
  expect(screen.getByRole('group', { name: 'Deployment settings' })).toBeTruthy();
  expect(screen.getByText('Choose the requested deployment settings.')).toBeTruthy();
  fireEvent.change(screen.getByRole('textbox', { name: 'Count' }), { target: { value: '1e2' } });
  expect(onChange).toHaveBeenLastCalledWith({ count: 100 });
  view.rerender(<NativeElicitation schema={schema} values={{ count: 100 }} disabled={false} onChange={onChange} />);
  expect(screen.getByRole('textbox', { name: 'Count' })).toHaveProperty('value', '1e2');
  view.rerender(<NativeElicitation schema={schema} values={{ count: 2 }} disabled={false} onChange={onChange} />);
  expect(screen.getByRole('textbox', { name: 'Count' })).toHaveProperty('value', '2');
});

it('preserves a Codex question answer after failure and only resends on an explicit retry', async () => {
  const submit = vi.fn().mockResolvedValueOnce(false).mockResolvedValueOnce(true);
  render(<Request engine="codex" pending={{ id: 'question-retry', kind: 'item/tool/requestUserInput', title: 'Question', data: { questions: [{ id: 'choice', question: 'Which scope?', options: [{ label: 'Core' }, { label: 'UI' }] }] } }} submit={submit} />);
  fireEvent.click(screen.getByRole('button', { name: 'Core' }));
  fireEvent.click(screen.getByRole('button', { name: 'agent.answer' }));
  expect((await screen.findByRole('alert')).textContent).toBe('agentRequest.sendFailed');
  expect(screen.getByRole('button', { name: 'Core' }).getAttribute('aria-pressed')).toBe('true');
  expect(submit).toHaveBeenCalledExactlyOnceWith({ answers: { choice: { answers: ['Core'] } } });
  fireEvent.click(screen.getByRole('button', { name: 'agent.answer' }));
  await waitFor(() => expect(submit).toHaveBeenCalledTimes(2));
  expect(submit.mock.calls[1]).toEqual(submit.mock.calls[0]);
});

it('keeps a broad policy grant secondary, requires its confirmation and resets it for a new generation', async () => {
  const submit = vi.fn(async () => true);
  const decision = { acceptWithExecpolicyAmendment: { execpolicyAmendment: ['echo', 'reviewed'] } };
  const pending: Pending = { id: 'policy', kind: 'item/commandExecution/requestApproval', title: 'Command', data: { command: 'echo reviewed', availableDecisions: ['accept', 'acceptForSession', 'decline', decision] } };
  const view = render(<Request engine="codex" sessionId="session" generation="first" pending={pending} submit={submit} />);
  expect(screen.getByRole('button', { name: 'agent.allow', exact: true }).classList.contains('bg-accent-default')).toBe(true);
  expect(screen.getByRole('button', { name: 'agent.allowSession' }).classList.contains('bg-accent-default')).toBe(false);
  const apply = screen.getByRole('button', { name: 'agent.applyPolicy' });
  expect(apply.classList.contains('bg-accent-default')).toBe(false);
  expect(apply).toHaveProperty('disabled', true);
  fireEvent.click(screen.getByRole('checkbox', { name: 'agent.confirmPolicy' }));
  expect(apply).toHaveProperty('disabled', false);
  view.rerender(<Request engine="codex" sessionId="session" generation="second" pending={pending} submit={submit} />);
  expect(screen.getByRole('checkbox', { name: 'agent.confirmPolicy' })).toHaveProperty('checked', false);
  expect(screen.getByRole('button', { name: 'agent.applyPolicy' })).toHaveProperty('disabled', true);
  fireEvent.click(screen.getByRole('checkbox', { name: 'agent.confirmPolicy' }));
  fireEvent.click(screen.getByRole('button', { name: 'agent.applyPolicy' }));
  await waitFor(() => expect(submit).toHaveBeenCalledExactlyOnceWith({ decision }));
});

it.each(['session', 'decline'] as const)('echoes the exact native permission %s choice without broadening the primary action', async action => {
  const submit = vi.fn(async () => true);
  const permissions = { network: { enabled: true }, fileSystem: { write: ['/task/output'] } };
  render(<Request engine="codex" pending={{ id: 'permission-scope', kind: 'item/permissions/requestApproval', title: 'Permissions', data: { permissions } }} submit={submit} />);
  expect(screen.getByRole('button', { name: 'agent.allowTurn' }).classList.contains('bg-accent-default')).toBe(true);
  expect(screen.getByRole('button', { name: 'agent.allowSession' }).classList.contains('bg-accent-default')).toBe(false);
  fireEvent.click(screen.getByRole('button', { name: action === 'session' ? 'agent.allowSession' : 'agent.deny' }));
  await waitFor(() => expect(submit).toHaveBeenCalledExactlyOnceWith({ permissions: action === 'session' ? permissions : {}, scope: action === 'session' ? 'session' : 'turn' }));
});
