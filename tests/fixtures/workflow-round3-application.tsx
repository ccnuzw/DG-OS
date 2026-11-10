// Browser fixture for real production components with an explicit deterministic IPC double.
// Vite: /tests/fixtures/terminal-accessibility.html?workflow=round3&scenario=all
// No native host, model, filesystem or process execution is claimed by this fixture.
import React, { useState } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { flushSync } from 'react-dom';
import { MemoryRouter, Route, Routes, useNavigate } from 'react-router-dom';
import { mockIPC, clearMocks } from '@tauri-apps/api/mocks';
import { ModalHost } from '@dgos/ui';
import { setLocaleForTesting, t } from '../../src/i18n';
import { ProjectConversation } from '../../src/pages/workbench/ProjectConversation';
import { ProjectCapabilities } from '../../src/components/development/ProjectCapabilities';
import { DevelopmentRuntime } from '../../src/components/development/DevelopmentRuntime';
import { TaskReview } from '../../src/components/development/TaskReview';
import { AgentConversation } from '../../src/pages/workbench/AgentConversation';
import { useDevelopment, type DevelopmentTask, type Project } from '../../src/stores/developmentStore';
import { useDevelopmentDrafts } from '../../src/stores/developmentDrafts';
import { useDevelopmentRuntime, type DevelopmentProcess } from '../../src/stores/developmentRuntime';
import { useDevelopmentTakeover } from '../../src/stores/developmentTakeover';
import { useAgentRunner, type AgentSession } from '../../src/stores/agentRunnerStore';
import type { CliProviderCatalog, CliProviderRevision } from '../../src/stores/developmentCapabilities';
import type { WorkspaceView } from '../../src/stores/developmentDelivery';
import '../../src/index.css';
import '../../src/components/development/development.css';

const project: Project = { id: 'fixture-project', owner: 'fixture-owner', name: 'Round3 fault fixture', path: '/fixture/round3', directory_identity: 'fixture-directory', archived: false, created_at: 1, last_opened_at: null };
const task: DevelopmentTask = { id: 'fixture-task', owner: project.owner, project_id: project.id, goal: 'Round3 workflow evidence', requirements: 'Preserve draft and never replay unknown requests', status: 'review', created_at: 1 };
function delay(ms: number) {
  const pending = Promise.withResolvers<void>();
  window.setTimeout(pending.resolve, ms);
  return pending.promise;
}
function frame() {
  const pending = Promise.withResolvers<void>();
  requestAnimationFrame(() => pending.resolve());
  return pending.promise;
}
async function settle() { await delay(0); await frame(); await frame(); }
async function until(predicate: () => unknown, description: string) {
  const deadline = performance.now() + 4000;
  while (!predicate()) { if (performance.now() > deadline) throw new Error(`Timed out: ${description}`); await delay(20); }
  await settle();
}
function required<T extends Element = HTMLElement>(selector: string, scope: ParentNode = document): T {
  const node = scope.querySelector<T>(selector); if (!node) throw new Error(`Missing ${selector}`); return node;
}
function button(key: string, scope: ParentNode = document): HTMLButtonElement {
  const node = [...scope.querySelectorAll<HTMLButtonElement>('button')].find(item => item.textContent?.trim() === t(key));
  if (!node) throw new Error(`Missing button ${key}`); return node;
}
function field(label: string, scope: ParentNode = document): HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement {
  const node = [...scope.querySelectorAll('label')].find(item => [...item.childNodes].filter(child => !(child instanceof HTMLTextAreaElement || child instanceof HTMLInputElement || child instanceof HTMLSelectElement)).map(child => child.textContent ?? '').join('').trim() === t(label));
  if (!node) throw new Error(`Missing label ${label}`);
  return node.htmlFor ? required(`#${CSS.escape(node.htmlFor)}`, scope) : required('input,textarea,select', node);
}
function change(node: HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement, value: string) {
  const prototype = node instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : node instanceof HTMLSelectElement ? HTMLSelectElement.prototype : HTMLInputElement.prototype;
  Object.getOwnPropertyDescriptor(prototype, 'value')!.set!.call(node, value);
  node.dispatchEvent(new Event(node instanceof HTMLSelectElement ? 'change' : 'input', { bubbles: true }));
}
const testid = <T extends Element = HTMLElement>(id: string) => required<T>(`[data-testid="${id}"]`);
interface Call { command: string; args: unknown; outcome: 'pending' | 'resolved' | 'rejected'; error?: string }
interface Assertion { name: string; passed: boolean; actual: unknown; expected: unknown }
interface ScenarioResult { scenario: string; passed: boolean; assertions: Assertion[]; calls: Call[]; counts: Record<string, number>; identities: unknown; error?: string }
const scenarios = ['capability-roundtrip', 'stale-capabilities', 'cleanup-failure', 'stale-review-unknown', 'native-approval'] as const;
type Scenario = typeof scenarios[number];

class FixtureHost {
  calls: Call[] = [];
  tasks = [structuredClone(task)];
  capability: CliProviderRevision = { revision: 4, permission_epoch: 2, configuration: { engine: 'codex', provider: { provider_id: 'fixture-provider', model_id: 'fixture-model', provider_revision: 1 } }, created_at: 1 };
  catalog: CliProviderCatalog = { project_id: project.id, engine: 'codex', providers: [{ id: 'fixture-provider', name: 'Fixture provider', config_revision: 1, enabled: true, availability: { available: true, reason: null }, models: [{ id: 'fixture-model', name: 'Fixture model', model_type: 'text' }] }] };
  processes: DevelopmentProcess[] = [];
  workspaces: WorkspaceView = { workspaces: [{ id: 'fixture-workspace', owner: project.owner, project_id: project.id, path: project.path, directory_identity: project.directory_identity, kind: 'original', common_dir: null, common_identity: null, created_operation: null, base_commit: null }], selected_workspace_id: 'fixture-workspace', operation: null, operations: [] };
  fingerprint = 'files-current';
  session: AgentSession = { id: 'fixture-session', generation: 'fixture-generation', engine: 'codex', native_id: 'fixture-native', workspace: project.path, status: 'waiting', events: [], pending: [], error: null, capabilities: null };
  cleanupFails = false;
  workspaceBusy = false;
  constructor() {
    mockIPC(async (command, args) => {
      const call: Call = { command, args: structuredClone(args), outcome: 'pending' }; this.calls.push(call);
      try { const value = await this.dispatch(command, args as Record<string, unknown>); call.outcome = 'resolved'; return structuredClone(value); }
      catch (error) { call.outcome = 'rejected'; call.error = String(error); throw error; }
    });
  }
  count(command: string) { return this.calls.filter(call => call.command === command).length; }
  async dispatch(command: string, args: Record<string, unknown> = {}): Promise<unknown> {
    switch (command) {
      case 'read_development_cli_provider': return this.capability;
      case 'read_development_cli_provider_catalog': return { ...this.catalog, engine: args.engine };
      case 'save_development_cli_provider': {
        const input = args.input as { expected_revision: number; configuration: CliProviderRevision['configuration'] };
        if (input.expected_revision !== this.capability.revision) throw 'capability_conflict';
        this.capability = { ...this.capability, revision: this.capability.revision + 1, configuration: input.configuration }; return this.capability;
      }
      case 'read_development_workspace': return { projects: [project], tasks: this.tasks, sessions: [], operations: [], selection: useDevelopment.getState().selection };
      case 'save_development_task': {
        const input = args.input as Partial<DevelopmentTask>;
        const saved = { ...task, ...input, id: input.id ?? 'fixture-created-task' };
        this.tasks = [...this.tasks.filter(item => item.id !== saved.id), saved]; return saved;
      }
      case 'select_development_context': return null;
      case 'read_development_commands': return { commands: [], discovered: [{ id: 'fixture-command', name: 'Fixture check', program: 'node', args: ['--test', 'fixture.js'], cwd: '', env_refs: {} }] };
      case 'list_development_processes': return this.processes;
      case 'read_development_process': { const process = this.processes.find(item => item.id === args.id); if (!process) throw 'process_not_found'; return process; }
      case 'start_development_process': if (this.workspaceBusy) throw 'workspace_busy'; throw new Error('Unexpected process dispatch: fixture never executes commands');
      case 'read_development_takeover': return { project_id: project.id, task_id: task.id, directory: project.path, sessions: [{ id: this.session.id, task_id: task.id, engine: 'codex', status: 'waiting' }], restricted: false, host_busy: false, can_close: true, token: 'fixture-cleanup-token' };
      case 'close_development_takeover': if (this.cleanupFails) throw 'cleanup_pending'; throw new Error('Unexpected successful cleanup');
      case 'read_development_review': return { task_id: task.id, captured_at: 2, baseline_late: false, truncated: false, fingerprint: this.fingerprint, changes: [{ path: 'fixture.js', kind: 'modified', pre_existing: false, origin: 'uncertain', previewable: true }], runs_stale: false };
      case 'list_development_runs': return [];
      case 'read_development_project_git': return { kind: 'directory', path: project.path, branch: null, head: null, merge_heads: [], files: [], conflicts: [], snapshot_token: 'fixture-snapshot-token' };
      case 'read_development_workspaces': return this.workspaces;
      case 'get_development_session_configuration': return { format: 'provider_bound', read_only: false, snapshot: { binding_id: 'fixture-binding', engine: 'codex', ...this.capability.configuration.provider, revision: this.capability.revision, permission_epoch: this.capability.permission_epoch } };
      case 'respond_agent_request': this.session = { ...this.session, status: 'idle', pending: [] }; return null;
      case 'get_agent_session': return this.session;
      case 'open_agent_session': throw 'unknown';
      default: throw new Error(`Unexpected fixture IPC: ${command}`);
    }
  }
}
function seed(newTask = false) {
  useAgentRunner.getState().reset(); useDevelopment.getState().reset(); useDevelopmentRuntime.getState().reset(); useDevelopmentDrafts.getState().clear();
  useDevelopment.setState({ projects: [project], tasks: [task], sessions: [], operations: [], loaded: true, selection: { project_id: project.id, task_id: newTask ? null : task.id, session_id: null } });
}
const verificationProcess: DevelopmentProcess = { id: 'fixture-verification', project_id: project.id, task_id: task.id, kind: 'command', source: 'host_command', status: 'exited', output: 'Fixture verification snapshot', truncated: false, exit_code: 0, reason: null, started_at: 3, finished_at: 4, program: 'node', args: ['--test', 'fixture.js'], cwd: '', fingerprint: 'files-current', start_fingerprint: 'files-current', verification_status: 'observed_stable', verification_complete: true, mutation_observed: false, preview_url: null };
function Conversation() {
  const session = useAgentRunner(state => state.session);
  return session ? <AgentConversation session={session} drafts={{}} setDraft={() => {}} busy={false} error={false} /> : null;
}

export async function runWorkflowRound3Probe() {
  if ('__TAURI_INTERNALS__' in window) throw new Error('Open this deterministic fixture in a browser, never in the native application');
  const requested = new URLSearchParams(location.search).get('scenario') ?? 'all';
  if (requested !== 'all' && !scenarios.includes(requested as Scenario)) throw new Error(`Unknown round3 scenario: ${requested}`);
  const chosen = requested === 'all' ? [...scenarios] : [requested as Scenario];
  setLocaleForTesting('en-US');
  document.documentElement.dataset.theme = 'light'; document.documentElement.dataset.density = '100';
  document.body.classList.add('bg-surface-canvas', 'text-ink-primary', 'flex-col');
  document.getElementById('result')?.classList.add('whitespace-pre-wrap', 'break-all');
  document.getElementById('workflow-round3-fixture')?.remove();
  const container = document.createElement('main'); container.id = 'workflow-round3-fixture'; container.className = 'dev-review-stack p-4';
  const title = document.createElement('h2'); title.textContent = 'Round3 production components · deterministic IPC fault coverage';
  const scope = document.createElement('p'); scope.textContent = 'Synthetic host responses. Command/request counts are observed at the IPC boundary; no real native process, model request or filesystem write runs.';
  const stage = document.createElement('div'); stage.id = 'workflow-round3-stage'; stage.className = 'dev-review-stack';
  container.append(title, scope, stage); document.body.append(container);
  let root: Root = createRoot(stage);
  const results: ScenarioResult[] = [];
  const render = async (element: React.ReactNode) => { flushSync(() => root.render(<MemoryRouter>{element}<ModalHost /></MemoryRouter>)); await settle(); };
  const remount = async (element: React.ReactNode) => { flushSync(() => root.unmount()); root = createRoot(stage); await render(element); };
  const outcomes = document.getElementById('result')!;
  const report = () => {
    const result = { scope: 'production React components; deterministic IPC double; no native execution', passed: results.length === chosen.length && results.every(item => item.passed), scenarios: results };
    Object.assign(window, { workflowRound3Result: result }); outcomes.textContent = JSON.stringify(result, null, 2); return result;
  };
  try {
    for (const scenario of chosen) {
      flushSync(() => root.render(null)); await settle(); seed(scenario === 'capability-roundtrip');
      const host = new FixtureHost(); const assertions: Assertion[] = [];
      const check = (name: string, actual: unknown, expected: unknown) => { assertions.push({ name, actual, expected, passed: JSON.stringify(actual) === JSON.stringify(expected) }); };
      let error: string | undefined;
      try {
        if (scenario === 'capability-roundtrip') {
          function Roundtrip() {
            const [tab, setTab] = useState<'launch' | 'capabilities'>('launch'); const navigate = useNavigate();
            return <Routes><Route path="/" element={<><nav><button type="button" data-fixture="launch" onClick={() => setTab('launch')}>Return to conversation draft</button><button type="button" data-fixture="capabilities" onClick={() => setTab('capabilities')}>Project provider</button></nav>{tab === 'launch' ? <ProjectConversation project={project} taskId={null} onStart={async () => { throw new Error('Roundtrip must not start a session'); }} onInspect={async () => {}} setSessionDraft={() => {}} /> : <ProjectCapabilities projectId={project.id} />}</>} /><Route path="/settings/providers" element={<section><h3>Fixture provider catalog management boundary</h3><p>The production Providers link arrived here. Remove the pinned provider to model management invalidation.</p><button type="button" data-fixture="remove-provider" onClick={() => { host.catalog.providers = []; }}>Remove fixture provider from catalog</button><button type="button" data-fixture="return" onClick={() => navigate('/')}>Return to project provider</button></section>} /></Routes>;
          }
          await render(<Roundtrip />); await until(() => host.count('read_development_cli_provider') > 0, 'initial provider revision');
          change(field('agent.prompt'), 'Retained round3 prompt\nRetained requirements through management'); await settle();
          required<HTMLButtonElement>('[data-fixture="capabilities"]').click(); await until(() => stage.querySelector('fieldset'), 'provider catalog');
          change(field('capabilities.source', stage), 'fixture-provider'); await settle();
          const providersLink = required<HTMLAnchorElement>('a[href="/settings/providers"]', stage); providersLink.click(); await settle();
          required<HTMLButtonElement>('[data-fixture="remove-provider"]').click(); required<HTMLButtonElement>('[data-fixture="return"]').click(); await until(() => stage.querySelector('fieldset'), 'return from management');
          check('removed provider blocks save', button('capabilities.save', stage).disabled, true);
          check('stale catalog warning remains visible', [...stage.querySelectorAll('[role="alert"]')].some(item => item.textContent === t('capabilities.staleCatalog')), true);
          required<HTMLButtonElement>('[data-fixture="launch"]').click(); await settle();
          check('complete first prompt survives provider roundtrip', field('agent.prompt').value, 'Retained round3 prompt\nRetained requirements through management');
          check('management causes no provider save', host.count('save_development_cli_provider'), 0); check('management creates no task', host.count('save_development_task'), 0);
          check('management never opens a session', host.count('open_agent_session'), 0);
        } else if (scenario === 'stale-capabilities') {
          const start: React.ComponentProps<typeof ProjectConversation>['onStart'] = async (input, _projectId, _taskId, dispatch) => {
            await useAgentRunner.getState().open(input);
            const runner = useAgentRunner.getState();
            if (runner.error || !runner.session) return { ok: false, code: runner.error ?? 'unknown' };
            return dispatch(runner.session);
          };
          const conversation = <ProjectConversation project={project} taskId={task.id} onStart={start} onInspect={async () => {}} setSessionDraft={() => {}} />;
          await render(conversation);
          await until(() => !testid<HTMLInputElement>('direct-conversation-trust').disabled && testid('composer-model-picker-trigger').textContent?.includes('Fixture model'), 'resolved directory and provider readiness');
          change(field('agent.prompt'), 'Never replay this first prompt'); await settle();
          const initialReads = host.count('read_development_cli_provider');
          host.capability = { ...host.capability, revision: 5 };
          testid<HTMLInputElement>('direct-conversation-trust').click();
          await until(() => !testid<HTMLButtonElement>('direct-conversation-send').disabled, 'explicit directory trust');
          testid<HTMLButtonElement>('direct-conversation-send').click();
          await until(() => [...stage.querySelectorAll('[role="alert"]')].some(item => item.textContent === t('dev.error.capability_conflict')), 'stale revision rejection');
          check('stale provider binding is revalidated', host.count('read_development_cli_provider') > initialReads, true);
          check('stale provider binding does not dispatch a session', host.count('open_agent_session'), 0);
          check('stale rejection retains complete prompt', field('agent.prompt').value, 'Never replay this first prompt');
          await remount(conversation);
          await until(() => !testid<HTMLInputElement>('direct-conversation-trust').disabled && testid('composer-model-picker-trigger').textContent?.includes('Fixture model'), 'remounted directory and provider');
          testid<HTMLInputElement>('direct-conversation-trust').click();
          await until(() => !testid<HTMLButtonElement>('direct-conversation-send').disabled, 'remounted directory trust');
          testid<HTMLButtonElement>('direct-conversation-send').click();
          await until(() => host.count('open_agent_session') === 1 && testid<HTMLButtonElement>('direct-conversation-send').disabled && [...stage.querySelectorAll('[role="alert"]')].some(item => item.textContent === t('directConversation.open_unknown')), 'unknown session outcome');
          const originalRequest = host.calls.find(call => call.command === 'open_agent_session');
          testid<HTMLButtonElement>('direct-conversation-send').click(); await settle();
          await remount(conversation);
          await until(() => !testid<HTMLInputElement>('direct-conversation-trust').disabled, 'unknown open remount readiness');
          testid<HTMLInputElement>('direct-conversation-trust').click(); await settle();
          check('unknown session remains blocked across remount', testid<HTMLButtonElement>('direct-conversation-send').disabled, true);
          testid<HTMLButtonElement>('direct-conversation-send').click(); await settle();
          check('unknown session never replays', host.count('open_agent_session'), 1);
          check('unknown session retains original request payload', host.calls.find(call => call.command === 'open_agent_session')?.args, originalRequest?.args);
          check('unknown open retains first prompt', field('agent.prompt').value, 'Never replay this first prompt');
        } else if (scenario === 'cleanup-failure') {
          host.workspaceBusy = true; host.cleanupFails = true;
          await render(<DevelopmentRuntime projectId={project.id} taskId={task.id} />);
          await until(() => stage.querySelector('select option[value="fixture-command"]'), 'command discovery');
          const discovery = required<HTMLSelectElement>('select:has(option[value="fixture-command"])', stage); change(discovery, 'fixture-command'); await settle();
          testid<HTMLInputElement>('dev-process-trust').click(); await settle(); testid<HTMLButtonElement>('dev-process-start').click();
          await until(() => stage.querySelector('[data-testid="dev-manual-takeover"] button') && host.count('read_development_takeover') > 0, 'takeover after workspace busy');
          button('takeover.close', testid('dev-manual-takeover')).click(); await until(() => document.querySelector('[role="dialog"]'), 'explicit cleanup confirmation');
          button('takeover.close', required('[role="dialog"]')).click(); await until(() => host.count('close_development_takeover') === 1 && !useDevelopmentTakeover.getState().entries[JSON.stringify([project.id, task.id])]?.busy, 'cleanup failure');
          check('cleanup failure never releases directory', useDevelopmentTakeover.getState().entries[JSON.stringify([project.id, task.id])]?.released, false);
          check('cleanup failure remains visible', Boolean(testid('dev-manual-takeover').querySelector('[role="alert"]')), true);
          testid<HTMLButtonElement>('dev-open-terminal').click(); await settle(); testid<HTMLButtonElement>('dev-process-start').click(); await settle();
          check('manual attempts remain rejected by busy host', host.calls.filter(call => call.command === 'start_development_process').map(call => call.error), ['workspace_busy', 'workspace_busy', 'workspace_busy']);
          check('failed cleanup never creates a process', host.processes.length, 0);
          check('failed cleanup keeps command draft', testid('dev-runtime-draft-summary').textContent?.includes('["node","--test","fixture.js"]'), true);
          check('cleanup is never retried implicitly', host.count('close_development_takeover'), 1);
        } else if (scenario === 'stale-review-unknown') {
          host.processes = [structuredClone(verificationProcess)];
          const review = <TaskReview projectId={project.id} taskId={task.id} onOpenDiff={() => {}} onOpenRuntime={() => {}} onOpenDelivery={() => {}} />;
          await render(review); await until(() => stage.querySelector('[data-verification="observed_stable"]'), 'current verification');
          testid<HTMLInputElement>('dev-task-review-confirm').click(); await settle(); host.fingerprint = 'files-changed'; testid<HTMLButtonElement>('dev-task-review-complete').click();
          await until(() => stage.querySelector('[data-verification="stale"]'), 'fresh snapshot invalidates verification');
          check('changed files revoke completion confirmation', testid<HTMLInputElement>('dev-task-review-confirm').checked, false);
          check('stale verification never saves completion', host.count('save_development_task'), 0);
          host.workspaces.operations = [{ id: 'fixture-unknown-push', kind: 'git_push', status: 'unknown', project_id: project.id, task_id: task.id, workspace_id: null, request_fingerprint: 'fixture-original-request', details: {} }];
          useDevelopmentRuntime.setState({ operations: [{ id: 'fixture-unknown-runtime', epoch: useDevelopment.getState().epoch, projectId: project.id, taskId: task.id, action: 'start', targetId: 'fixture-original-process', phase: 'unknown' }] });
          await remount(review); await until(() => stage.querySelector('[data-verification="stale"]'), 'unknown operation snapshot');
          testid<HTMLInputElement>('dev-task-review-accept').click(); await settle(); change(testid<HTMLTextAreaElement>('dev-task-review-explanation'), 'Explicit acceptance cannot override unresolved operations'); await settle(); testid<HTMLInputElement>('dev-task-review-confirm').click(); await settle();
          check('unknown runtime and delivery block accepted stale completion', testid<HTMLButtonElement>('dev-task-review-complete').disabled, true); testid<HTMLButtonElement>('dev-task-review-complete').click();
          check('unknown operations never replay process or Git commands', host.calls.filter(call => ['start_development_process', 'execute_development_git', 'save_development_task'].includes(call.command)).length, 0);
          check('unknown delivery request identity retained', host.workspaces.operations[0].id, 'fixture-unknown-push');
        } else {
          host.session.pending = [{ id: 'fixture-native-approval', kind: 'item/commandExecution/requestApproval', title: 'Native command approval', data: { command: 'node --test fixture.js', availableDecisions: ['accept', 'decline'] } }];
          useAgentRunner.setState({ selected: host.session.id, session: structuredClone(host.session) });
          await render(<Conversation />); await until(() => stage.querySelector(`section[aria-label="${t('agent.approval')}"]`), 'native approval');
          const native = required(`section[aria-label="${t('agent.approval')}"]`, stage); const nativeAllow = button('agent.allow', native); nativeAllow.click(); nativeAllow.click();
          await until(() => host.count('respond_agent_request') === 1 && !useAgentRunner.getState().session?.pending.length, 'native approval acknowledgement');
          check('native approval dispatched exactly once', host.count('respond_agent_request'), 1);
          const nativeArgs = host.calls.find(call => call.command === 'respond_agent_request')?.args;
          const nativeIdentity = nativeArgs && typeof nativeArgs === 'object' && 'input' in nativeArgs && nativeArgs.input && typeof nativeArgs.input === 'object' && 'request_id' in nativeArgs.input ? nativeArgs.input.request_id : null;
          check('native request identity preserved', nativeIdentity, 'fixture-native-approval');
          check('removed host gateway is never called', host.calls.filter(call => ['get_development_session_capabilities', 'list_development_capability_confirmations', 'respond_development_capability_confirmation'].includes(call.command)).length, 0);
        }
      } catch (failure) { error = failure instanceof Error ? failure.stack ?? failure.message : String(failure); }
      check('fixture rejects no unmodeled IPC silently', host.calls.filter(call => call.error?.startsWith('Error: Unexpected')).map(call => ({ command: call.command, error: call.error })), []);
      check('fault scenarios never send model prompts', host.count('send_agent_message'), 0);
      const counts = Object.fromEntries([...new Set(host.calls.map(call => call.command))].map(command => [command, host.count(command)]));
      results.push({ scenario, passed: !error && assertions.length > 0 && assertions.every(item => item.passed), assertions, calls: structuredClone(host.calls), counts, identities: { project: project.id, tasks: host.tasks.map(item => item.id), session: host.session.id, processes: host.processes.map(item => item.id), capabilityRevision: host.capability.revision, reviewFingerprint: host.fingerprint, deliveryOperations: host.workspaces.operations.map(item => ({ id: item.id, status: item.status })), runtimeOperations: useDevelopmentRuntime.getState().operations.map(item => ({ id: item.id, targetId: item.targetId, phase: item.phase })) }, ...(error ? { error } : {}) });
      report();
    }
    return report();
  } finally {
    // Freeze the observed production DOM before disposing timers. The capture is
    // screenshot evidence only; assertions and command counts remain in the report.
    const snapshot = stage.cloneNode(true);
    useAgentRunner.getState().detach();
    flushSync(() => root.unmount());
    if (new URLSearchParams(location.search).has('hold')) {
      stage.append(...snapshot.childNodes);
      stage.inert = true;
    } else {
      const summary = document.createElement('p'); summary.textContent = 'Scenario measurements complete. Reload with one scenario and &hold=1 to inspect its static final production surface.';
      stage.append(summary);
    }
    clearMocks();
    Reflect.deleteProperty(window, '__TAURI_INTERNALS__');
    Reflect.deleteProperty(window, '__TAURI_EVENT_PLUGIN_INTERNALS__');
  }
}
