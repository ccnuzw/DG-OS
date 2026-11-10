import { useEffect, useRef, useState } from 'react';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { Modal } from '@dgos/ui';
import { afterEach, expect, it, vi } from 'vitest';
import { AgentConversation } from '../src/pages/workbench/AgentConversation';
import { AgentSlashPicker, agentSlashQuery, type AgentSlashObservation } from '../src/components/development/AgentSlashPicker';
import { commandReason } from '../src/components/development/ClaudeCommandPicker';
import { codexSkillCandidates } from '../src/components/development/AgentSkillPicker';
import { useAgentRunner, type AgentSession } from '../src/stores/agentRunnerStore';
import { useDevelopment } from '../src/stores/developmentStore';
import { guardWorkbenchDrafts, useDevelopmentDrafts } from '../src/stores/developmentDrafts';
import { acknowledgeAgentSkills, agentSkillsKey, captureAgentSkills } from '../src/stores/agentSkills';
import { addConversationReference } from '../src/stores/developmentReferences';
import { removeDevelopmentProject } from '../src/components/development/removeProject';
import type { NativeObservedSkill } from '../src/types/nativeExtensions';
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('../src/i18n', () => ({ useLocale: () => ({ locale: 'en-US', t: (key: string) => key }), t: (key: string) => key }));
afterEach(() => { cleanup(); useDevelopment.getState().reset(); useAgentRunner.getState().reset(); useDevelopmentDrafts.getState().clear(); invoke.mockReset(); });
const session: AgentSession = { id: 'skill-session', generation: 'skill-generation', engine: 'codex', native_id: 'native', workspace: '/project', status: 'idle', events: [], pending: [], error: null };
const skill = { name: 'review', path: '/native/review/SKILL.md' };
const claude: AgentSession = { ...session, id: 'claude-commands', generation: 'claude-generation', engine: 'claude' };
const nativeCommand = (name: string, builtin: boolean | null = null, aliases: string[] = []): NativeObservedSkill => ({ name, builtin, aliases, classification: builtin === true ? 'builtin_command' : 'unknown', enabled: null, path: null, scope: null, plugin_id: null });
const codexSkill: NativeObservedSkill = { ...nativeCommand('review'), ...skill, enabled: true, classification: 'skill' };
const observation = (value = claude, skills: NativeObservedSkill[] = [nativeCommand('review')]) => ({ session_id: value.id, generation: value.generation, engine: value.engine, domain: 'skills', incomplete: false, skills });
function select(value = session) {
  useDevelopment.setState({ selection: { project_id: 'p', task_id: null, session_id: value.id } });
  useAgentRunner.setState({ selected: value.id, session: value, snapshots: { [value.id]: value } });
}
function draft(id = 'first', projectId: string | null = 'p') { useDevelopmentDrafts.setState(state => ({ entries: { ...state.entries, [agentSkillsKey(projectId, session)]: { kind: 'conversation', baseline: [], value: [{ id, skill }] } } })); }
function Picker({ runtime = claude, initial = '/', active = true, disabled = false }: { runtime?: AgentSession; initial?: string; active?: boolean; disabled?: boolean }) {
  const input = useRef<HTMLTextAreaElement>(null);
  const [value, setValue] = useState(initial);
  useEffect(() => {
    // Model a user invoking / at the end of the draft, not an unfocused
    // restored textarea whose native selection starts before the slash.
    const node = input.current!;
    node.focus(); node.setSelectionRange(initial.length, initial.length); fireEvent.select(node);
  }, []);
  return <><textarea ref={input} aria-label="Draft" value={value} onChange={event => setValue(event.target.value)} /><AgentSlashPicker session={runtime} projectId="p" value={value} onChange={(next, expected) => setValue(current => current === expected ? next : current)} inputRef={input} active={active} disabled={disabled} /></>;
}
it.each(['/usr/local/bin', 'src/foo', '```\n/review', 'text /review', '/review arguments', '// comment'])('does not treat ordinary path/code %s as slash invocation', value => {
  expect(agentSlashQuery(value, value.length)).toBeNull();
});
it('recognizes only a plain leading slash token and supports arguments after the caret', () => {
  expect(agentSlashQuery('/rev', 4)).toBe('rev');
  expect(agentSlashQuery('/rev arguments', 4)).toBe('rev');
});
it('supports namespaced native commands without capturing ordinary slash-delimited paths', async () => {
  expect(agentSlashQuery('/apps/web:dep', 13)).toBe('apps/web:dep');
  expect(agentSlashQuery('/apps/web', 9)).toBeNull();
  select(claude);
  invoke.mockResolvedValue(observation(claude, [nativeCommand('apps/web:deploy', false)]));
  render(<Picker initial="/apps/web:dep" />);
  fireEvent.click(await screen.findByRole('option', { name: /apps\/web:deploy/ }));
  await waitFor(() => expect(screen.getByRole('textbox')).toHaveProperty('value', '/apps/web:deploy '));
  expect(invoke.mock.calls.every(([name]) => name === 'observe_native_session')).toBe(true);
});
it('accepts only enabled uniquely identified Codex skills and preserves native command precedence', () => {
  expect(codexSkillCandidates([codexSkill, { ...codexSkill, name: 'disabled', enabled: false }, { ...codexSkill, name: 'duplicate' }, { ...codexSkill, name: 'duplicate' }, { ...codexSkill, name: 'no-path', path: null }])).toEqual([codexSkill]);
  const actual = nativeCommand('apps/web:deploy', false, ['deploy', 'status']);
  const builtin = nativeCommand('status', true, ['deploy']);
  expect(commandReason([actual, builtin], actual, 'apps/web:deploy')).toBeNull();
  expect(commandReason([actual, builtin], actual, 'deploy')).toBe('agentSkills.commands.ambiguous');
  expect(commandReason([actual, builtin], actual, 'status')).toBe('agentSkills.commands.builtin');
  expect(commandReason([{ ...actual, enabled: false }], { ...actual, enabled: false }, actual.name)).not.toBeNull();
});
it('searches real native commands, shows source, supports keyboard selection and never dispatches a turn', async () => {
  select(claude);
  invoke.mockResolvedValue(observation(claude, [{ ...nativeCommand('review'), source: '/repo/.claude/skills/review/SKILL.md' }, nativeCommand('deploy')]));
  render(<Picker initial="/rev" />);
  const input = screen.getByRole('textbox', { name: 'Draft' }); input.focus();
  expect(await screen.findByRole('option', { name: /review/ })).toBeTruthy();
  expect(screen.queryByRole('option', { name: /deploy/ })).toBeNull();
  expect(screen.getByText('/repo/.claude/skills/review/SKILL.md')).toBeTruthy();
  fireEvent.keyDown(input, { key: 'Enter' });
  await waitFor(() => expect(input).toHaveProperty('value', '/review '));
  expect(invoke.mock.calls.filter(([name]) => name === 'observe_native_session')).toHaveLength(2);
  expect(invoke.mock.calls.every(([name]) => name === 'observe_native_session')).toBe(true);
});
it('navigates candidates with arrows and Tab, and Escape preserves the plain text', async () => {
  select(claude); invoke.mockResolvedValue(observation(claude, [nativeCommand('review'), nativeCommand('deploy')]));
  render(<Picker />); const input = screen.getByRole('textbox', { name: 'Draft' }); input.focus();
  await screen.findByRole('option', { name: /review/ });
  fireEvent.keyDown(input, { key: 'ArrowDown' }); fireEvent.keyDown(input, { key: 'Tab' });
  await waitFor(() => expect(input).toHaveProperty('value', '/deploy '));
  fireEvent.input(input, { target: { value: '/' } });
  await screen.findByRole('option', { name: /review/ });
  fireEvent.keyDown(input, { key: 'Escape' });
  expect(screen.queryByRole('listbox')).toBeNull(); expect(input).toHaveProperty('value', '/');
});
it('does not intercept composition or ordinary paths and never sends on IME Enter', async () => {
  select(claude); invoke.mockResolvedValue(observation()); render(<Picker />);
  const input = screen.getByRole('textbox', { name: 'Draft' }); input.focus();
  await screen.findByRole('option');
  fireEvent.compositionStart(input);
  expect(fireEvent.keyDown(input, { key: 'Enter', isComposing: true, keyCode: 229 })).toBe(true);
  expect(input).toHaveProperty('value', '/');
  fireEvent.input(input, { target: { value: '/usr/local' } }); fireEvent.compositionEnd(input);
  expect(screen.queryByRole('listbox')).toBeNull();
  expect(fireEvent.keyDown(input, { key: 'Enter' })).toBe(true);
});
it('selects a structured Codex skill and removing the final chip releases the draft guard', async () => {
  select(); invoke.mockResolvedValue(observation(session, [codexSkill])); render(<Picker runtime={session} />);
  const input = screen.getByRole('textbox', { name: 'Draft' }); input.focus();
  fireEvent.click(await screen.findByRole('option', { name: /review/ }));
  await waitFor(() => expect(captureAgentSkills('p', session).map(row => row.skill)).toEqual([skill]));
  expect(invoke.mock.calls.filter(([name]) => name === 'observe_native_session')).toHaveLength(2);
  expect(input).toHaveProperty('value', '');
  const confirm = vi.spyOn(Modal, 'confirm').mockImplementation(options => { options.onCancel?.(); return { id: 'confirm', close: () => {} }; });
  try {
    expect(await guardWorkbenchDrafts('p')).toBe(false);
    fireEvent.click(screen.getByRole('button', { name: /review.*agentSkills.remove/ }));
    expect(captureAgentSkills('p', session)).toEqual([]);
    confirm.mockClear(); expect(await guardWorkbenchDrafts('p')).toBe(true); expect(confirm).not.toHaveBeenCalled();
  } finally { confirm.mockRestore(); }
});
it.each(['incomplete', 'wrong-generation', 'wrong-engine', 'wrong-session', 'wrong-domain', 'rejected'])('does not expose unverified %s native capabilities', async failure => {
  select(claude); const value = observation();
  if (failure === 'incomplete') value.incomplete = true;
  if (failure === 'wrong-generation') value.generation = 'old';
  if (failure === 'wrong-engine') value.engine = 'codex';
  if (failure === 'wrong-session') value.session_id = 'other';
  if (failure === 'wrong-domain') value.domain = 'mcp';
  if (failure === 'rejected') invoke.mockRejectedValue('unsupported'); else invoke.mockResolvedValue(value);
  render(<Picker />); await screen.findByRole('alert'); expect(screen.queryByRole('option')).toBeNull();
});
it.each(['removed', 'alias-reassigned', 'builtin-added', 'duplicate-added', 'disabled'])('rechecks a native command before applying a %s stale choice', async change => {
  select(claude); const original = nativeCommand('review', false, ['inspect']);
  const changed = change === 'removed' ? [] : change === 'alias-reassigned' ? [original, nativeCommand('other', false, ['inspect'])] : change === 'builtin-added' ? [original, nativeCommand('inspect', true)] : change === 'disabled' ? [{ ...original, enabled: false }] : [original, { ...original }];
  invoke.mockResolvedValueOnce(observation(claude, [original])).mockResolvedValue(observation(claude, changed));
  render(<Picker initial="/ins" />);
  fireEvent.click(await screen.findByRole('option', { name: /inspect/ }));
  await screen.findByRole('alert'); expect(screen.getByRole('textbox')).toHaveProperty('value', '/ins');
  expect(invoke.mock.calls.filter(([name]) => name === 'observe_native_session')).toHaveLength(2);
});
it.each(['account', 'session', 'inactive', 'unmounted', 'dismissed'])('discards a pending command selection after its %s changes', async change => {
  select(claude); const pending = Promise.withResolvers<AgentSlashObservation>();
  invoke.mockResolvedValueOnce(observation()).mockReturnValueOnce(pending.promise);
  const view = render(<Picker />); fireEvent.click(await screen.findByRole('option'));
  expect(invoke.mock.calls.filter(([name]) => name === 'observe_native_session')).toHaveLength(2);
  const input = screen.getByRole('textbox');
  if (change === 'account') act(() => useDevelopment.getState().reset());
  if (change === 'session') act(() => useAgentRunner.setState({ selected: 'other' }));
  if (change === 'inactive') view.rerender(<Picker active={false} />);
  if (change === 'unmounted') view.unmount();
  if (change === 'dismissed') fireEvent.keyDown(input, { key: 'Escape' });
  await act(async () => { pending.resolve(observation()); });
  if (change !== 'unmounted') expect(input).toHaveProperty('value', '/');
});
it('never overwrites edits made while a command choice is being rechecked', async () => {
  select(claude); const pending = Promise.withResolvers<AgentSlashObservation>();
  invoke.mockResolvedValueOnce(observation()).mockReturnValueOnce(pending.promise);
  render(<Picker />); fireEvent.click(await screen.findByRole('option'));
  expect(invoke.mock.calls.filter(([name]) => name === 'observe_native_session')).toHaveLength(2);
  const input = screen.getByRole('textbox'); fireEvent.input(input, { target: { value: 'Latest draft' } });
  await act(async () => { pending.resolve(observation()); });
  expect(input).toHaveProperty('value', 'Latest draft');
});
it('puts a native command in the real conversation draft, dispatching only on an explicit Send', async () => {
  select(claude); invoke.mockImplementation(command => command === 'observe_native_session' ? Promise.resolve(observation()) : command === 'send_agent_message' ? Promise.resolve(null) : Promise.resolve(claude));
  function Conversation() { const [drafts, setDrafts] = useState({ [claude.id]: '/rev inspect' }); return <AgentConversation session={claude} drafts={drafts} setDraft={(id, value, expected) => setDrafts(previous => expected === undefined || previous[id] === expected ? { ...previous, [id]: value } : previous)} busy={false} error={false} />; }
  render(<MemoryRouter><Conversation /></MemoryRouter>); const input = screen.getByRole('textbox', { name: 'agent.prompt' }) as HTMLTextAreaElement;
  input.focus(); input.setSelectionRange(4, 4); fireEvent.select(input);
  fireEvent.click(await screen.findByRole('option', { name: /review/ }));
  await waitFor(() => expect(input).toHaveProperty('value', '/review inspect'));
  expect(invoke.mock.calls.some(([name]) => name === 'send_agent_message' || name === 'start_agent_session')).toBe(false);
  fireEvent.click(screen.getByRole('button', { name: 'agent.send' }));
  await waitFor(() => expect(invoke.mock.calls.filter(([name]) => name === 'send_agent_message')).toHaveLength(1));
  expect(invoke.mock.calls.find(([name]) => name === 'send_agent_message')![1].input).toMatchObject({ text: '/review inspect' });
});
it('sends text plus explicit native skill and references, clearing only the acknowledged selection identity', async () => {
  select(); draft(); const receipt = Promise.withResolvers<null>();
  invoke.mockImplementation(command => command === 'send_agent_message' ? receipt.promise : Promise.resolve(session));
  const reference = { project_id: 'p', task_id: null, path: 'a.ts', kind: 'file' as const, expected_hash: 'version' }; addConversationReference('p', session.id, reference);
  render(<MemoryRouter><AgentConversation session={session} drafts={{ [session.id]: 'Review this' }} setDraft={vi.fn()} busy={false} error={false} /></MemoryRouter>);
  fireEvent.click(screen.getByRole('button', { name: 'agent.send' }));
  await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === 'send_agent_message')).toBe(true));
  expect(invoke.mock.calls.find(([command]) => command === 'send_agent_message')![1].input).toMatchObject({ text: 'Review this', skills: [skill], references: [reference] });
  act(() => draft('later-reselected')); await act(async () => { receipt.resolve(null); });
  expect(captureAgentSkills('p', session).map(item => item.id)).toEqual(['later-reselected']);
});
it.each(['agent_skill_unavailable', 'unknown'])('preserves selected skill after %s and never replays an unknown send', async failure => {
  select(); draft(); invoke.mockImplementation(command => command === 'send_agent_message' ? Promise.reject(failure) : Promise.resolve(session));
  render(<MemoryRouter><AgentConversation session={session} drafts={{ [session.id]: 'Review this' }} setDraft={vi.fn()} busy={false} error={false} /></MemoryRouter>);
  fireEvent.click(screen.getByRole('button', { name: 'agent.send' })); await waitFor(() => expect(useAgentRunner.getState().error).toBe(failure));
  expect(captureAgentSkills('p', session).map(item => item.skill)).toEqual([skill]);
  if (failure === 'unknown') { await screen.findByRole('button', { name: 'directConversation.recheck' }); fireEvent.click(screen.getByRole('button', { name: 'agent.send' })); expect(invoke.mock.calls.filter(([command]) => command === 'send_agent_message')).toHaveLength(1); }
});
it('protects skill drafts before project removal and clears only the removed project after confirmation', async () => {
  select(); draft(); draft('other-project', 'other'); draft('unscoped', null);
  const updateProject = vi.spyOn(useDevelopment.getState(), 'updateProject').mockResolvedValue({ ok: true, value: null });
  const confirm = vi.spyOn(Modal, 'confirm').mockImplementationOnce(options => { options.onConfirm?.(); return { id: 'confirm', close: () => {} }; }).mockImplementation(options => { options.onCancel?.(); return { id: 'confirm', close: () => {} }; });
  try {
    expect(await removeDevelopmentProject('p')).toEqual({ ok: false, code: 'cancelled' }); expect(updateProject).not.toHaveBeenCalled();
    expect(captureAgentSkills('p', session).map(item => item.id)).toEqual(['first']);
    confirm.mockImplementation(options => { options.onConfirm?.(); return { id: 'confirm', close: () => {} }; });
    expect(await removeDevelopmentProject('p')).toEqual({ ok: true, value: null }); expect(updateProject).toHaveBeenCalledWith('p', 'unlink', null);
    expect(captureAgentSkills('p', session)).toEqual([]); expect(captureAgentSkills('other', session).map(item => item.id)).toEqual(['other-project']); expect(captureAgentSkills(null, session).map(item => item.id)).toEqual(['unscoped']);
  } finally { updateProject.mockRestore(); confirm.mockRestore(); }
});
it('acknowledges only submitted skill identities within their project scope', () => {
  select(); draft(); draft('other-project', 'other'); const submitted = captureAgentSkills('p', session);
  const added = { id: 'concurrent', skill: { name: 'implement', path: '/native/implement/SKILL.md' } };
  useDevelopmentDrafts.setState(state => ({ entries: { ...state.entries, [agentSkillsKey('p', session)]: { kind: 'conversation', baseline: [], value: [...submitted, added] } } }));
  acknowledgeAgentSkills('p', session, submitted); expect(captureAgentSkills('p', session)).toEqual([added]); expect(captureAgentSkills('other', session).map(item => item.id)).toEqual(['other-project']);
  acknowledgeAgentSkills('p', session, [added]); expect(captureAgentSkills('p', session)).toEqual([]);
});

it('keeps selected Skills across a native generation change without moving them to another project', () => {
  select(); draft();
  expect(captureAgentSkills('p', { ...session, generation: 'restored-generation' }).map(row => row.skill)).toEqual([skill]);
  expect(captureAgentSkills('other', session)).toEqual([]);
});

it('does not invent a trailing caret for restored text or capture a selected code token', async () => {
  select(claude); invoke.mockResolvedValue(observation());
  function RestoredDraft() {
    const input = useRef<HTMLTextAreaElement>(null);
    return <><textarea ref={input} aria-label="Restored draft" defaultValue="/rev" /><AgentSlashPicker session={claude} projectId="p" value="/rev" onChange={vi.fn()} inputRef={input} active /></>;
  }
  render(<RestoredDraft />);
  const input = screen.getByRole<HTMLTextAreaElement>('textbox', { name: 'Restored draft' });
  expect(input.selectionStart).toBe(0);
  expect(screen.queryByRole('listbox')).toBeNull(); expect(invoke).not.toHaveBeenCalled();
  input.focus(); input.setSelectionRange(4, 4); fireEvent.select(input);
  await screen.findByRole('option');
  input.setSelectionRange(0, 4); fireEvent.select(input);
  expect(screen.queryByRole('listbox')).toBeNull();
  expect(fireEvent.keyDown(input, { key: 'Enter' })).toBe(true);
  expect(invoke.mock.calls.filter(([name]) => name === 'observe_native_session')).toHaveLength(1);
});
