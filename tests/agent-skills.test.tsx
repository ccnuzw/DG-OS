import { useState } from 'react';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { Modal } from '@dgos/ui';
import { afterEach, expect, it, vi } from 'vitest';
import { AgentConversation } from '../src/pages/workbench/AgentConversation';
import { AgentSkillPicker } from '../src/components/development/AgentSkillPicker';
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
function select(value = session) {
  useDevelopment.setState({ selection: { project_id: 'p', task_id: null, session_id: value.id } });
  useAgentRunner.setState({ selected: value.id, session: value, snapshots: { [value.id]: value } });
}
function draft(id = 'first', projectId: string | null = 'p') { useDevelopmentDrafts.setState(state => ({ entries: { ...state.entries, [agentSkillsKey(projectId, session)]: { kind: 'conversation', baseline: [], value: [{ id, skill }] } } })); }
it('shows only current enabled unique native skills and isolates selection by project, session and generation', async () => {
  select();
  invoke.mockResolvedValue({ session_id: session.id, generation: session.generation, engine: 'codex', domain: 'skills', incomplete: false, skills: [{ ...skill, enabled: true, classification: 'skill' }, { name: 'disabled', path: '/disabled', enabled: false, classification: 'skill' }, ...Array.from({ length: 2 }, () => ({ name: 'duplicate', path: '/duplicate', enabled: true, classification: 'skill' }))] });
  render(<AgentSkillPicker projectId="p" session={session} disabled={false} active />);
  fireEvent.click(screen.getByRole('button', { name: 'agentSkills.title' }));
  fireEvent.click(await screen.findByRole('checkbox', { name: /review/ }));
  expect(screen.queryByRole('checkbox', { name: /disabled/ })).toBeNull();
  expect(screen.queryByRole('checkbox', { name: /duplicate/ })).toBeNull();
  expect(captureAgentSkills('p', session).map(item => item.skill)).toEqual([skill]);
  expect(captureAgentSkills('other-project', session)).toEqual([]);
  expect(captureAgentSkills(null, session)).toEqual([]);
  expect(captureAgentSkills('p', { ...session, generation: 'replacement' })).toEqual([]);
  expect(captureAgentSkills('p', { ...session, id: 'other' })).toEqual([]);
  expect(invoke).toHaveBeenCalledWith('observe_native_session', { input: { session_id: session.id, generation: session.generation, domain: 'skills' } });
});
it.each(['codex', 'claude'] as const)('never invents %s skill lists before native discovery', engine => {
  render(<AgentSkillPicker projectId="p" engine={engine} disabled={false} active />);
  fireEvent.click(screen.getByRole('button', { name: engine === 'claude' ? 'agentSkills.commands.title' : 'agentSkills.title' }));
  expect(screen.queryByRole('checkbox')).toBeNull();
  expect(screen.queryByRole('group')).toBeNull();
  expect(invoke).not.toHaveBeenCalled();
});
it('sends text plus explicit native skill and references, clearing only the acknowledged selection identity', async () => {
  select(); draft();
  const receipt = Promise.withResolvers<null>();
  invoke.mockImplementation(command => command === 'send_agent_message' ? receipt.promise : Promise.resolve(session));
  const reference = { project_id: 'p', task_id: null, path: 'a.ts', kind: 'file' as const, expected_hash: 'version' };
  addConversationReference('p', session.id, reference);
  render(<MemoryRouter><AgentConversation session={session} drafts={{ [session.id]: 'Review this' }} setDraft={vi.fn()} busy={false} error={false} /></MemoryRouter>);
  fireEvent.click(screen.getByRole('button', { name: 'agent.send' }));
  await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === 'send_agent_message')).toBe(true));
  expect(invoke.mock.calls.find(([command]) => command === 'send_agent_message')![1].input).toMatchObject({ text: 'Review this', skills: [skill], references: [reference] });
  act(() => draft('later-reselected'));
  await act(async () => { receipt.resolve(null); });
  expect(captureAgentSkills('p', session).map(item => item.id)).toEqual(['later-reselected']);
});
it.each(['agent_skill_unavailable', 'unknown'])('preserves selected skill after %s and never replays an unknown send', async failure => {
  select(); draft();
  invoke.mockImplementation(command => command === 'send_agent_message' ? Promise.reject(failure) : Promise.resolve(session));
  render(<MemoryRouter><AgentConversation session={session} drafts={{ [session.id]: 'Review this' }} setDraft={vi.fn()} busy={false} error={false} /></MemoryRouter>);
  fireEvent.click(screen.getByRole('button', { name: 'agent.send' }));
  await waitFor(() => expect(useAgentRunner.getState().error).toBe(failure));
  expect(captureAgentSkills('p', session).map(item => item.skill)).toEqual([skill]);
  if (failure === 'unknown') {
    await screen.findByRole('button', { name: 'directConversation.recheck' });
    fireEvent.click(screen.getByRole('button', { name: 'agent.send' }));
    expect(invoke.mock.calls.filter(([command]) => command === 'send_agent_message')).toHaveLength(1);
  }
});
it('returns to a clean draft after selecting and cancelling the final native skill', async () => {
  select();
  invoke.mockResolvedValue({ session_id: session.id, generation: session.generation, engine: 'codex', domain: 'skills', incomplete: false, skills: [{ ...skill, enabled: true, classification: 'skill' }] });
  render(<AgentSkillPicker projectId="p" session={session} disabled={false} active />);
  fireEvent.click(screen.getByRole('button', { name: 'agentSkills.title' }));
  const checkbox = await screen.findByRole('checkbox', { name: /review/ });
  const confirm = vi.spyOn(Modal, 'confirm').mockImplementation(options => { options.onCancel?.(); return { id: 'confirm', close: () => {} }; });
  try {
    fireEvent.click(checkbox);
    expect(await guardWorkbenchDrafts('p')).toBe(false);
    expect(captureAgentSkills('p', session).map(item => item.skill)).toEqual([skill]);
    confirm.mockClear();
    fireEvent.click(checkbox);
    expect(captureAgentSkills('p', session)).toEqual([]);
    expect(await guardWorkbenchDrafts('p')).toBe(true);
    expect(await guardWorkbenchDrafts()).toBe(true);
    expect(confirm).not.toHaveBeenCalled();
  } finally { confirm.mockRestore(); }
});
it('protects skill drafts before project removal and clears only the removed project after confirmation', async () => {
  select(); draft(); draft('other-project', 'other'); draft('unscoped', null);
  const updateProject = vi.spyOn(useDevelopment.getState(), 'updateProject').mockResolvedValue({ ok: true, value: null });
  const confirm = vi.spyOn(Modal, 'confirm')
    .mockImplementationOnce(options => { options.onConfirm?.(); return { id: 'confirm', close: () => {} }; })
    .mockImplementation(options => { options.onCancel?.(); return { id: 'confirm', close: () => {} }; });
  try {
    expect(await removeDevelopmentProject('p')).toEqual({ ok: false, code: 'cancelled' });
    expect(updateProject).not.toHaveBeenCalled();
    expect(captureAgentSkills('p', session).map(item => item.id)).toEqual(['first']);
    confirm.mockImplementation(options => { options.onConfirm?.(); return { id: 'confirm', close: () => {} }; });
    expect(await removeDevelopmentProject('p')).toEqual({ ok: true, value: null });
    expect(updateProject).toHaveBeenCalledWith('p', 'unlink', null);
    expect(captureAgentSkills('p', session)).toEqual([]);
    expect(captureAgentSkills('other', session).map(item => item.id)).toEqual(['other-project']);
    expect(captureAgentSkills(null, session).map(item => item.id)).toEqual(['unscoped']);
  } finally { updateProject.mockRestore(); confirm.mockRestore(); }
});
it('acknowledges only submitted skill identities within their project scope', () => {
  select(); draft(); draft('other-project', 'other');
  const submitted = captureAgentSkills('p', session);
  const added = { id: 'concurrent', skill: { name: 'implement', path: '/native/implement/SKILL.md' } };
  useDevelopmentDrafts.setState(state => ({ entries: { ...state.entries, [agentSkillsKey('p', session)]: { kind: 'conversation', baseline: [], value: [...submitted, added] } } }));
  acknowledgeAgentSkills('p', session, submitted);
  expect(captureAgentSkills('p', session)).toEqual([added]);
  expect(captureAgentSkills('other', session).map(item => item.id)).toEqual(['other-project']);
  acknowledgeAgentSkills('p', session, [added]);
  expect(captureAgentSkills('p', session)).toEqual([]);
});

const claude: AgentSession = { ...session, id: 'claude-commands', generation: 'claude-generation', engine: 'claude' };
type CommandObservation = { session_id: string; generation: string; engine: string; domain: string; incomplete: boolean; skills: NativeObservedSkill[] };
const nativeCommand = (name: string, builtin: boolean | null = null, aliases: string[] = []): NativeObservedSkill => ({ name, builtin, aliases, classification: builtin === true ? 'builtin_command' : 'unknown', enabled: null, path: null, scope: null, plugin_id: null });
const commandObservation = (skills = [nativeCommand('review')]): CommandObservation => ({ session_id: claude.id, generation: claude.generation, engine: 'claude', domain: 'skills', incomplete: false, skills });

it('adds only a unique non-builtin original Claude command name to the draft callback without sending or selecting a Codex skill', async () => {
  select(claude);
  const onCommandSelect = vi.fn();
  invoke.mockResolvedValue(commandObservation([nativeCommand('plugin:review'), nativeCommand('explicit-user-command', false), nativeCommand('help', true), nativeCommand('help'), nativeCommand('duplicate'), nativeCommand('duplicate', false), nativeCommand('status', true)]));
  render(<AgentSkillPicker projectId="p" session={claude} disabled={false} active onCommandSelect={onCommandSelect} />);
  fireEvent.click(screen.getByRole('button', { name: 'agentSkills.commands.title' }));
  const command = await screen.findByRole('button', { name: '/plugin:review' });
  expect(command).toHaveProperty('disabled', false);
  expect(screen.getByRole('button', { name: '/explicit-user-command' })).toHaveProperty('disabled', false);
  for (const button of screen.getAllByRole('button', { name: /^\/(help|duplicate|status)$/ })) expect(button).toHaveProperty('disabled', true);
  expect(screen.queryByRole('checkbox')).toBeNull();
  expect(onCommandSelect).not.toHaveBeenCalled();
  fireEvent.click(command);
  await waitFor(() => expect(onCommandSelect).toHaveBeenCalledExactlyOnceWith('/plugin:review'));
  expect(screen.queryByRole('button', { name: '/plugin:review' })).toBeNull();
  expect(captureAgentSkills('p', claude)).toEqual([]);
  expect(invoke.mock.calls).toEqual(Array.from({ length: 2 }, () => ['observe_native_session', { input: { session_id: claude.id, generation: claude.generation, domain: 'skills' } }]));
});

it.each(['rejected', 'incomplete', 'wrong-generation', 'wrong-engine', 'wrong-session', 'wrong-domain'])('allows retry after a %s Claude command observation without exposing unverified commands', async failure => {
  select(claude);
  const onCommandSelect = vi.fn();
  const value = commandObservation();
  if (failure === 'incomplete') value.incomplete = true;
  if (failure === 'wrong-generation') value.generation = 'old-generation';
  if (failure === 'wrong-engine') value.engine = 'codex';
  if (failure === 'wrong-session') value.session_id = 'other-session';
  if (failure === 'wrong-domain') value.domain = 'mcp';
  if (failure === 'rejected') invoke.mockRejectedValueOnce(new Error('unavailable'));
  else invoke.mockResolvedValueOnce(value);
  invoke.mockResolvedValue(commandObservation());
  render(<AgentSkillPicker projectId="p" session={claude} disabled={false} active onCommandSelect={onCommandSelect} />);
  fireEvent.click(screen.getByRole('button', { name: 'agentSkills.commands.title' }));
  await screen.findByRole('alert');
  expect(screen.queryByRole('button', { name: '/review' })).toBeNull();
  expect(onCommandSelect).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole('button', { name: 'agentSkills.commands.refresh' }));
  fireEvent.click(await screen.findByRole('button', { name: '/review' }));
  await waitFor(() => expect(onCommandSelect).toHaveBeenCalledExactlyOnceWith('/review'));
  expect(invoke.mock.calls.every(([name]) => name === 'observe_native_session')).toBe(true);
});

it.each(['session', 'generation', 'account', 'inactive', 'disabled', 'unmounted'])('discards late Claude commands after its %s owner changes', async change => {
  select(claude);
  const pending = Promise.withResolvers<CommandObservation>();
  const onCommandSelect = vi.fn();
  invoke.mockReturnValue(pending.promise);
  const picker = (value = claude, active = true, disabled = false) => <AgentSkillPicker projectId="p" session={value} disabled={disabled} active={active} onCommandSelect={onCommandSelect} />;
  const view = render(picker());
  fireEvent.click(screen.getByRole('button', { name: 'agentSkills.commands.title' }));
  expect(invoke).toHaveBeenCalledTimes(1);
  if (change === 'session' || change === 'generation') {
    const replacement = change === 'session' ? { ...claude, id: 'other-session' } : { ...claude, generation: 'replacement' };
    act(() => select(replacement));
    view.rerender(picker(replacement));
  } else if (change === 'account') act(() => useDevelopment.getState().reset());
  else if (change === 'inactive') view.rerender(picker(claude, false));
  else if (change === 'disabled') view.rerender(picker(claude, true, true));
  else view.unmount();
  await act(async () => { pending.resolve(commandObservation()); });
  expect(screen.queryByRole('button', { name: '/review' })).toBeNull();
  expect(onCommandSelect).not.toHaveBeenCalled();
  expect(invoke).toHaveBeenCalledTimes(1);
});

it('rechecks live session ownership before allowing a previously discovered Claude command to change a draft', async () => {
  select(claude);
  const onCommandSelect = vi.fn();
  invoke.mockResolvedValue(commandObservation());
  render(<AgentSkillPicker projectId="p" session={claude} disabled={false} active onCommandSelect={onCommandSelect} />);
  fireEvent.click(screen.getByRole('button', { name: 'agentSkills.commands.title' }));
  const command = await screen.findByRole('button', { name: '/review' });
  act(() => useAgentRunner.setState({ selected: 'other', session: { ...claude, id: 'other' } }));
  fireEvent.click(command);
  expect(onCommandSelect).not.toHaveBeenCalled();
});

it('restores trigger focus on keyboard dismissal and leaves focus with the draft after explicit selection', async () => {
  select(claude);
  invoke.mockResolvedValue(commandObservation());
  render(<><textarea aria-label="Draft" /><AgentSkillPicker projectId="p" session={claude} disabled={false} active onCommandSelect={() => screen.getByRole('textbox', { name: 'Draft' }).focus()} /></>);
  const trigger = screen.getByRole('button', { name: 'agentSkills.commands.title' });
  trigger.focus(); fireEvent.click(trigger);
  const command = await screen.findByRole('button', { name: '/review' });
  command.focus();
  fireEvent.keyDown(command, { key: 'Escape' });
  await waitFor(() => expect(document.activeElement).toBe(trigger));
  expect(screen.queryByRole('button', { name: '/review' })).toBeNull();
  fireEvent.click(trigger);
  fireEvent.click(await screen.findByRole('button', { name: '/review' }));
  await waitFor(() => expect(document.activeElement).toBe(screen.getByRole('textbox', { name: 'Draft' })));
  expect(invoke.mock.calls.every(([name]) => name === 'observe_native_session')).toBe(true);
});

it('puts a Claude command into the real conversation draft and sends it only after the user explicitly sends', async () => {
  select(claude);
  invoke.mockImplementation(command => command === 'observe_native_session' ? Promise.resolve(commandObservation()) : command === 'send_agent_message' ? Promise.resolve(null) : Promise.resolve(claude));
  function Conversation() {
    const [drafts, setDrafts] = useState({ [claude.id]: 'Inspect current changes' });
    return <AgentConversation session={claude} drafts={drafts} setDraft={(id, value) => setDrafts(previous => ({ ...previous, [id]: value }))} busy={false} error={false} />;
  }
  render(<MemoryRouter><Conversation /></MemoryRouter>);
  const prompt = screen.getByRole('textbox', { name: 'agent.prompt' });
  fireEvent.click(screen.getByRole('button', { name: 'agentSkills.commands.title' }));
  fireEvent.click(await screen.findByRole('button', { name: '/review' }));
  await waitFor(() => expect(prompt).toHaveProperty('value', '/review Inspect current changes'));
  await waitFor(() => expect(document.activeElement).toBe(prompt));
  expect(invoke.mock.calls.some(([name]) => name === 'send_agent_message' || name === 'start_agent_session')).toBe(false);
  fireEvent.click(screen.getByRole('button', { name: 'agent.send' }));
  await waitFor(() => expect(invoke.mock.calls.filter(([name]) => name === 'send_agent_message')).toHaveLength(1));
  expect(invoke.mock.calls.find(([name]) => name === 'send_agent_message')![1].input).toMatchObject({ text: '/review Inspect current changes' });
  expect(invoke.mock.calls.find(([name]) => name === 'send_agent_message')![1].input.skills ?? []).toEqual([]);
});

it('shows Claude commands read-only when no draft callback is connected', async () => {
  select(claude);
  invoke.mockResolvedValue(commandObservation());
  render(<AgentSkillPicker projectId="p" session={claude} disabled={false} active />);
  fireEvent.click(screen.getByRole('button', { name: 'agentSkills.commands.title' }));
  expect(await screen.findByRole('button', { name: '/review' })).toHaveProperty('disabled', true);
  expect(invoke.mock.calls.every(([name]) => name === 'observe_native_session')).toBe(true);
});

it('shows nested Claude names and aliases, honoring actual-name precedence over builtin aliases', async () => {
  select(claude);
  const onCommandSelect = vi.fn();
  invoke.mockResolvedValue(commandObservation([
    nativeCommand('apps/web:deploy', false, ['deploy']),
    nativeCommand('native-status', true, ['deploy']),
    nativeCommand('deploy', false),
    nativeCommand('first', false, ['shared']),
    nativeCommand('second', false, ['shared']),
    nativeCommand('help', true),
    nativeCommand('third', false, ['help']),
  ]));
  render(<AgentSkillPicker projectId="p" session={claude} disabled={false} active onCommandSelect={onCommandSelect} />);
  fireEvent.click(screen.getByRole('button', { name: 'agentSkills.commands.title' }));
  expect(await screen.findByRole('button', { name: '/apps/web:deploy' })).toHaveProperty('disabled', false);
  const deploy = screen.getAllByRole('button', { name: '/deploy' });
  expect(deploy.map(button => (button as HTMLButtonElement).disabled)).toEqual([true, true, false]);
  for (const button of screen.getAllByRole('button', { name: /^\/(shared|help)$/ })) expect(button).toHaveProperty('disabled', true);
  expect(screen.getAllByText(/agentSkills.commands.aliasOf/).length).toBeGreaterThan(0);
  fireEvent.click(deploy[2]);
  await waitFor(() => expect(onCommandSelect).toHaveBeenCalledExactlyOnceWith('/deploy'));
  expect(invoke.mock.calls.every(([name]) => name === 'observe_native_session')).toBe(true);
});

it.each(['apps/web:deploy', 'deploy'])('puts the unambiguous native token /%s in the draft without implicit sending', async name => {
  select(claude);
  const onCommandSelect = vi.fn();
  invoke.mockResolvedValue(commandObservation([nativeCommand('apps/web:deploy', false, ['deploy'])]));
  render(<AgentSkillPicker projectId="p" session={claude} disabled={false} active onCommandSelect={onCommandSelect} />);
  fireEvent.click(screen.getByRole('button', { name: 'agentSkills.commands.title' }));
  fireEvent.click(await screen.findByRole('button', { name: `/${name}` }));
  await waitFor(() => expect(onCommandSelect).toHaveBeenCalledExactlyOnceWith(`/${name}`));
  expect(invoke.mock.calls.map(([name]) => name)).toEqual(['observe_native_session', 'observe_native_session']);
});

it.each(['removed', 'alias-reassigned', 'actual-name-added', 'builtin-added', 'duplicate-added'])('refreshes rather than using a stale Claude command after %s', async change => {
  select(claude);
  const onCommandSelect = vi.fn();
  const original = nativeCommand('apps/web:deploy', false, ['deploy']);
  const changed = change === 'removed' ? []
    : change === 'alias-reassigned' ? [nativeCommand('other', false, ['deploy'])]
    : change === 'actual-name-added' ? [original, nativeCommand('deploy', false)]
    : change === 'builtin-added' ? [original, nativeCommand('deploy', true)]
    : [original, nativeCommand('other', false, ['deploy'])];
  invoke.mockResolvedValueOnce(commandObservation([original])).mockResolvedValue(commandObservation(changed));
  render(<AgentSkillPicker projectId="p" session={claude} disabled={false} active onCommandSelect={onCommandSelect} />);
  fireEvent.click(screen.getByRole('button', { name: 'agentSkills.commands.title' }));
  fireEvent.click(await screen.findByRole('button', { name: '/deploy' }));
  expect(await screen.findByRole('alert')).toHaveProperty('textContent', 'agentSkills.commands.changed');
  expect(onCommandSelect).not.toHaveBeenCalled();
  expect(invoke.mock.calls.every(([name]) => name === 'observe_native_session')).toBe(true);
  if (change === 'actual-name-added') {
    const actual = screen.getAllByRole('button', { name: '/deploy' }).find(button => !(button as HTMLButtonElement).disabled)!;
    fireEvent.click(actual);
    await waitFor(() => expect(onCommandSelect).toHaveBeenCalledExactlyOnceWith('/deploy'));
  }
});

it.each(['dismissed', 'session-changed', 'unmounted'])('does not apply a pending command choice after it is %s', async change => {
  select(claude);
  const onCommandSelect = vi.fn();
  const pending = Promise.withResolvers<CommandObservation>();
  invoke.mockResolvedValueOnce(commandObservation()).mockReturnValueOnce(pending.promise);
  const view = render(<AgentSkillPicker projectId="p" session={claude} disabled={false} active onCommandSelect={onCommandSelect} />);
  fireEvent.click(screen.getByRole('button', { name: 'agentSkills.commands.title' }));
  fireEvent.click(await screen.findByRole('button', { name: '/review' }));
  await screen.findByRole('status');
  if (change === 'dismissed') fireEvent.keyDown(screen.getByRole('status'), { key: 'Escape' });
  else if (change === 'session-changed') act(() => useAgentRunner.setState({ selected: 'other', session: { ...claude, id: 'other' } }));
  else view.unmount();
  await act(async () => { pending.resolve(commandObservation()); });
  expect(onCommandSelect).not.toHaveBeenCalled();
});

it('shows Claude aliases read-only when no draft callback is connected', async () => {
  select(claude);
  invoke.mockResolvedValue(commandObservation([nativeCommand('review', false, ['inspect'])]));
  render(<AgentSkillPicker projectId="p" session={claude} disabled={false} active />);
  fireEvent.click(screen.getByRole('button', { name: 'agentSkills.commands.title' }));
  expect(await screen.findByRole('button', { name: '/inspect' })).toHaveProperty('disabled', true);
  expect(screen.getByRole('button', { name: '/review' })).toHaveProperty('disabled', true);
  expect(invoke).toHaveBeenCalledTimes(1);
});

it('keeps edits made while a Claude command choice is being rechecked', async () => {
  select(claude);
  const pending = Promise.withResolvers<CommandObservation>();
  invoke.mockImplementation(command => command === 'observe_native_session' ? Promise.resolve(commandObservation()) : Promise.resolve(claude));
  function Conversation() {
    const [drafts, setDrafts] = useState({ [claude.id]: 'Initial draft' });
    return <AgentConversation session={claude} drafts={drafts} setDraft={(id, value) => setDrafts(previous => ({ ...previous, [id]: value }))} busy={false} error={false} />;
  }
  render(<MemoryRouter><Conversation /></MemoryRouter>);
  const prompt = screen.getByRole('textbox', { name: 'agent.prompt' });
  fireEvent.click(screen.getByRole('button', { name: 'agentSkills.commands.title' }));
  const command = await screen.findByRole('button', { name: '/review' });
  invoke.mockImplementation(name => name === 'observe_native_session' ? pending.promise : Promise.resolve(claude));
  fireEvent.click(command);
  fireEvent.change(prompt, { target: { value: 'Latest draft' } });
  await act(async () => { pending.resolve(commandObservation()); });
  await waitFor(() => expect(prompt).toHaveProperty('value', '/review Latest draft'));
  await waitFor(() => expect(document.activeElement).toBe(prompt));
  expect(invoke.mock.calls.some(([name]) => name === 'send_agent_message' || name === 'start_agent_session')).toBe(false);
});
