import { useEffect, useRef, useState, type ReactNode } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Link } from 'react-router-dom';
import { Button, Card, Checkbox, Input, Loading, Select, Stack, StatusIcon, cn } from '@dgos/ui';
import { usePageError } from '../pageError';
import type { Provider, Model } from '../stores/providerStore';
import type { Skill } from '../stores/skillsStore';
import type { McpServer, McpTool } from '../stores/mcpStore';

interface CommandResponse<T> { success: boolean; data?: T; error?: { code?: number; message: string } }
export interface TaskRecord {
  task: { id: string; status: string; provider_id: string; model_id: string; created_at: number; error_code?: string; total_tokens?: number };
  skills: { skill_id: string; version_id: string; content_hash: string }[];
  tools?: { server_id: string; tool_name: string; schema_hash: string }[];
}
interface TaskOutcome { record: TaskRecord; content?: string; audit_warning?: boolean }
interface AvailableTool extends McpTool { server_name: string }
interface ToolProposal {
  id: string; server_id: string; server_name: string; tool_name: string; schema_hash: string;
  arguments: unknown; expires_at: number;
}
interface ToolTaskUpdate extends TaskOutcome { phase: string; proposal?: ToolProposal }
const toolKey = (tool: McpTool) => JSON.stringify([tool.server_id, tool.tool_name, tool.schema_hash]);
const message = (error: unknown) => error instanceof Error ? error.message : String(error);
const taskFailureMessage = (status: string, code?: string) => {
  const detail = `Task ${status}: ${code || 'unknown_error'}`;
  if (code === 'mcp_unknown_result') return `${detail}. The external tool result is unknown; do not retry automatically. Check the MCP audit before repeating the task.`;
  if (code === 'mcp_audit_warning') return `${detail}. The tool may have executed, but its audit is incomplete; do not retry automatically.`;
  return `${detail}. Nothing was retried without your selected skill.`;
};
async function command<T>(name: string, args: Record<string, unknown>) {
  const result = await invoke<CommandResponse<T>>(name, args);
  if (!result.success || !result.data) {
    // The message stays as it was; the numeric code (when the host sent one) rides along so handleError can map it.
    const failure: Error & { code?: number } = new Error(result.error?.message || 'Unable to load configuration');
    if (typeof result.error?.code === 'number') failure.code = result.error.code;
    throw failure;
  }
  return result.data;
}

// Radix Select items cannot carry an empty value, so "nothing chosen" travels as this sentinel.
const NONE = '__none__';
const PARA = 'm-0 text-sm text-muted-foreground';
const LINK = 'text-primary underline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-focus-ring';
const FOCUS = 'focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-focus-ring';
const PRE = cn('m-0 rounded-md bg-secondary p-4 text-sm whitespace-pre-wrap wrap-anywhere', FOCUS);

function Notice({ tone, children }: { tone: 'error' | 'warning'; children: ReactNode }) {
  return (
    <div role="alert" className={cn('flex items-start gap-2 rounded-md border-l-4 p-3 text-sm',
      tone === 'error' ? 'border-error bg-error-bg text-error-text' : 'border-warning bg-warning-bg text-warning-text')}>
      <StatusIcon name={tone === 'error' ? 'error' : 'warning'} className="mt-0.5" />
      <p className="m-0 min-w-0 wrap-anywhere">{children}</p>
    </div>
  );
}

export default function Workbench() {
  const [providers, setProviders] = useState<Provider[]>([]);
  const [models, setModels] = useState<Model[]>([]);
  const [skills, setSkills] = useState<Skill[]>([]);
  const [providerId, setProviderId] = useState('');
  const [modelId, setModelId] = useState('');
  const [skillId, setSkillId] = useState('');
  const [withoutSkill, setWithoutSkill] = useState(false);
  const [prompt, setPrompt] = useState('');
  const [loading, setLoading] = useState(true);
  const [modelsLoading, setModelsLoading] = useState(false);
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [historyError, setHistoryError] = useState<string | null>(null);
  const [history, setHistory] = useState<TaskRecord[]>([]);
  const [outcome, setOutcome] = useState<TaskOutcome | null>(null);
  const [reload, setReload] = useState(0);
  const [tools, setTools] = useState<AvailableTool[]>([]);
  const [selectedTools, setSelectedTools] = useState<string[]>([]);
  const [toolsError, setToolsError] = useState<string | null>(null);
  const [toolsLoading, setToolsLoading] = useState(true);
  const [toolTask, setToolTask] = useState<ToolTaskUpdate | null>(null);
  const [now, setNow] = useState(Date.now());
  const [cancelPending, setCancelPending] = useState(false);
  const submitting = useRef(false);
  const cancelling = useRef(false);
  const taskRef = useRef<string | null>(null);
  const taskKind = useRef<'text' | 'tool' | null>(null);
  const mounted = useRef(true);
  const generation = useRef(0);
  const proposalHeading = useRef<HTMLHeadingElement>(null);
  const reportError = usePageError('Workbench');
  const report = useRef(reportError);
  useEffect(() => { report.current = reportError; }, [reportError]);
  const taskActive = toolTask?.record.task.status === 'running';
  const locked = running || taskActive || cancelPending;

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      if (taskRef.current) {
        void invoke(taskKind.current === 'text' ? 'cancel_workbench_task' : 'cancel_workbench_tool_task', { taskId: taskRef.current }).catch(() => {});
      }
    };
  }, []);

  useEffect(() => {
    if (!toolTask?.proposal) return;
    setNow(Date.now());
    proposalHeading.current?.focus();
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [toolTask?.proposal?.id]);

  useEffect(() => {
    let active = true;
    setToolsLoading(true);
    setSelectedTools([]);
    setToolsError(null);
    invoke<McpServer[]>('list_mcp_servers').then(async servers => {
      const groups = await Promise.all(servers.filter(s => s.enabled && s.status === 'ready').map(async server => {
        const listed = await invoke<McpTool[]>('list_mcp_tools', { serverId: server.id });
        return listed.map(tool => ({ ...tool, server_name: server.name }));
      }));
      if (active) setTools(groups.flat());
    }).catch(e => { if (active) { setTools([]); setToolsError('MCP tools unavailable. Text-only tasks still work; open MCP management to connect a server.'); report.current(e); } })
      .finally(() => { if (active) setToolsLoading(false); });
    return () => { active = false; };
  }, [reload]);

  useEffect(() => {
    let active = true;
    setLoading(true);
    setError(null);
    Promise.all([
      command<Provider[]>('list_providers', {}),
      invoke<Skill[]>('list_installed_skills', {}),
    ]).then(([allProviders, allSkills]) => {
      if (!active) return;
      setProviders(allProviders.filter(p => p.enabled && p.credential_status === 'stored'));
      setSkills(allSkills.filter(s => s.enabled && s.call_type === 'content'));
    }).catch(e => { if (active) { setError(`Unable to load workbench: ${message(e)}`); report.current(e); } })
      .finally(() => { if (active) setLoading(false); });
    invoke<TaskRecord[]>('list_workbench_tasks', {})
      .then(records => { if (active) { setHistory(records); setHistoryError(null); } })
      .catch(e => { if (active) { setHistoryError('Task history is unavailable. Refresh to try again.'); report.current(e); } });
    return () => { active = false; };
  }, [reload]);

  useEffect(() => {
    let active = true;
    setModels([]);
    setModelId('');
    setModelsLoading(false);
    if (!providerId) return;
    setModelsLoading(true);
    command<Model[]>('list_models', { providerId }).then(allModels => {
      if (active) setModels(allModels.filter(m => m.model_type === 'text-generation' || m.model_type === 'chat'));
    }).catch(e => { if (active) { setError('Unable to load models. Check your provider configuration.'); report.current(e); } })
      .finally(() => { if (active) setModelsLoading(false); });
    return () => { active = false; };
  }, [providerId, reload]);

  const bytes = new TextEncoder().encode(prompt).length;
  const ready = !loading && !modelsLoading && providers.some(p => p.id === providerId)
    && models.some(m => m.model_id === modelId && m.provider_id === providerId) && prompt.trim() && bytes <= 16384
    && (skills.some(s => s.skill_id === skillId) || (!skillId && withoutSkill));

  function rememberToolUpdate(result: ToolTaskUpdate) {
    if (!mounted.current) return;
    setToolTask(result);
    setOutcome(result);
    taskRef.current = result.record.task.status === 'running' ? result.record.task.id : null;
    setHistory(old => [result.record, ...old.filter(row => row.task.id !== result.record.task.id)].slice(0, 50));
    if (result.record.task.status === 'failed') setError(`${taskFailureMessage('failed', result.record.task.error_code)} Cancellation does not undo external effects.`);
  }

  async function cancelTask() {
    if (!taskRef.current || cancelling.current) return;
    const taskId = taskRef.current;
    const kind = taskKind.current;
    cancelling.current = true;
    setCancelPending(true);
    ++generation.current;
    setError(null);
    try {
      if (kind === 'tool') {
        const result = await invoke<ToolTaskUpdate>('cancel_workbench_tool_task', { taskId });
        rememberToolUpdate(result);
      } else {
        const cancelled = await invoke<boolean>('cancel_workbench_task', { taskId });
        if (!cancelled && mounted.current) setError('Task is no longer running. Check task history for its final state.');
      }
    } catch (e) {
      if (mounted.current) { setError(`Cancellation not confirmed: ${message(e)}. Check task audit; external effects may already have occurred.`); report.current(e); }
    } finally {
      cancelling.current = false;
      if (mounted.current) setCancelPending(false);
    }
  }

  async function run() {
    if (!ready || locked || submitting.current) return;
    submitting.current = true;
    setRunning(true);
    setError(null);
    setOutcome(null);
    setToolTask(null);
    const current = ++generation.current;
    const clientTaskId = crypto.randomUUID();
    try {
      taskRef.current = clientTaskId;
      taskKind.current = selectedTools.length ? 'tool' : 'text';
      if (selectedTools.length) {
        const chosen = tools.filter(tool => selectedTools.includes(toolKey(tool)));
        if (chosen.length !== selectedTools.length) throw new Error('Selected tools changed; refresh and select again');
        const started = await invoke<ToolTaskUpdate>('start_workbench_tool_task', { input: {
          task: { provider_id: providerId, model_id: modelId, prompt, skill_id: skillId || null, allow_without_skill: !skillId && withoutSkill, client_task_id: clientTaskId },
          tools: chosen.map(tool => ({ server_id: tool.server_id, tool_name: tool.tool_name, schema_hash: tool.schema_hash })),
        } });
        if (!mounted.current) {
          void invoke('cancel_workbench_tool_task', { taskId: started.record.task.id }).catch(() => {});
          return;
        }
        rememberToolUpdate(started);
        const result = await invoke<ToolTaskUpdate>('advance_workbench_tool_task', { taskId: started.record.task.id, action: { type: 'run_model' } });
        if (current === generation.current) rememberToolUpdate(result);
        return;
      }
      const result = await invoke<TaskOutcome>('run_workbench_task', {
        input: { provider_id: providerId, model_id: modelId, prompt, skill_id: skillId || null, allow_without_skill: !skillId && withoutSkill, client_task_id: clientTaskId },
      });
      setOutcome(result);
      if (result.record.task.status !== 'running' && taskRef.current === clientTaskId) {
        taskRef.current = null;
        taskKind.current = null;
      }
      setHistory(old => [result.record, ...old.filter(row => row.task.id !== result.record.task.id)].slice(0, 50));
      if (result.record.task.status !== 'succeeded') setError(taskFailureMessage(result.record.task.status, result.record.task.error_code));
    } catch (e) {
      if (mounted.current && current === generation.current) { setError(`Task not completed: ${message(e)}. No fallback request was sent. Check task history before submitting again.`); report.current(e); }
    } finally {
      if (taskKind.current === 'text' && taskRef.current === clientTaskId) {
        taskRef.current = null;
        taskKind.current = null;
      }
      submitting.current = false;
      if (mounted.current) setRunning(false);
    }
  }

  async function decide(type: 'approve' | 'deny') {
    if (!toolTask?.proposal || submitting.current || cancelling.current || now >= toolTask.proposal.expires_at * 1000) return;
    submitting.current = true;
    setRunning(true);
    setError(null);
    const current = ++generation.current;
    try {
      const result = await invoke<ToolTaskUpdate>('advance_workbench_tool_task', {
        taskId: toolTask.record.task.id, action: { type, proposal_id: toolTask.proposal.id },
      });
      if (current === generation.current) rememberToolUpdate(result);
    } catch (e) {
      if (mounted.current && current === generation.current) { setError(`Task action not completed: ${message(e)}. Do not repeat approval; cancel and check task audit.`); report.current(e); }
    } finally { submitting.current = false; if (mounted.current) setRunning(false); }
  }

  const proposalExpired = !!toolTask?.proposal && now >= toolTask.proposal.expires_at * 1000;
  const providerOptions = [{ value: NONE, label: 'Choose a provider' }, ...providers.map(p => ({ value: p.id, label: p.name }))];
  const modelPlaceholder = modelsLoading ? 'Loading models...' : 'Choose a cached model';
  const modelOptions = [{ value: NONE, label: modelPlaceholder }, ...models.map(m => ({ value: m.model_id, label: m.model_name }))];
  const skillOptions = [{ value: NONE, label: 'No skill selected' }, ...skills.map(s => ({ value: s.skill_id, label: `${s.name} — v${s.version}` }))];
  const fromSelect = (value: string) => value === NONE ? '' : value;

  return <main className="h-full min-w-0 overflow-y-auto bg-background p-8 text-foreground">
    <header className="mb-6 flex flex-wrap items-start justify-between gap-6">
      <div className="min-w-0">
        <h1 className="m-0 mb-2 text-3xl font-semibold text-foreground">AI Workbench</h1>
        <p className={PARA}>One task, with the skill you choose. Versions stay fixed for each run.</p>
      </div>
      <Button variant="secondary" disabled={locked || loading} onClick={() => setReload(n => n + 1)}>Refresh configuration</Button>
    </header>
    <div className="flex flex-wrap items-start gap-8">
      <section className="min-w-0 grow basis-64" aria-labelledby="task-heading">
        <Card>
          <Card.Header><h2 id="task-heading" className="m-0 text-lg font-semibold">New task</h2></Card.Header>
          <Card.Body>
            <Stack gap={4}>
              <p className={PARA}>Configure a <Link to="/providers" className={LINK}>Provider</Link> and add content from <Link to="/skills" className={LINK}>Skills</Link> first.</p>
              {loading && <Loading label="Loading local configuration..." />}
              {!loading && !providers.length && <p className={PARA}>No ready provider. Add, enable, or migrate a provider in Providers.</p>}
              <form onSubmit={e => { e.preventDefault(); void run(); }}>
                <fieldset disabled={locked || loading} className="m-0 min-w-0 border-0 p-0">
                  <Stack gap={4}>
                    <div className="flex flex-wrap gap-4">
                      <div className="min-w-0 grow basis-40">
                        <Select label="Provider" options={providerOptions} value={providerId} placeholder="Choose a provider" className="overflow-hidden"
                          onChange={value => { setProviderId(fromSelect(value)); setError(null); }} />
                      </div>
                      <div className="min-w-0 grow basis-40">
                        <Select label="Model" options={modelOptions} value={modelId} placeholder={modelPlaceholder} className="overflow-hidden"
                          disabled={!providerId || modelsLoading} onChange={value => setModelId(fromSelect(value))} />
                      </div>
                    </div>
                    {!!providerId && !modelsLoading && !models.length && <p className={PARA}>Refresh or select models in Providers before running a task.</p>}
                    <Select label="Skill for this task" options={skillOptions} value={skillId} placeholder="No skill selected" className="overflow-hidden"
                      onChange={value => { setSkillId(fromSelect(value)); setWithoutSkill(false); }} />
                    <p className={PARA}>Only enabled content skills are listed. Compatibility and integrity are rechecked before sending; a rejected skill is never silently skipped.</p>
                    {!skillId && <Checkbox label="I explicitly want to run without a skill" checked={withoutSkill} onChange={setWithoutSkill} />}
                    <section aria-labelledby="tools-heading">
                      <Card className="bg-background shadow-none">
                        <Card.Body>
                          <Stack gap={3}>
                            <h3 id="tools-heading" className="m-0 text-base font-semibold">MCP tools for this task <span className="font-normal text-muted-foreground">{selectedTools.length} / 8</span></h3>
                            <p className={PARA}>Optional. Only selected tools are sent to the Provider. Every model-proposed call requires your separate approval.</p>
                            {toolsLoading && <Loading label="Loading connected tools..." />}
                            {toolsError && <p className={PARA}>{toolsError}</p>}
                            {!toolsLoading && !toolsError && !tools.length && <p className={PARA}>No connected tools. Connect an enabled server in <Link to="/mcp" className={LINK}>MCP management</Link>, then refresh.</p>}
                            {tools.map(tool => {
                              const key = toolKey(tool);
                              const checked = selectedTools.includes(key);
                              return <Checkbox key={key} className="max-w-full items-start wrap-anywhere" checked={checked}
                                disabled={!checked && selectedTools.length >= 8}
                                onChange={on => setSelectedTools(old => on ? [...old, key] : old.filter(k => k !== key))}
                                label={<span>{tool.server_name} / {tool.tool_name}<small className="mt-1 block text-xs text-muted-foreground">{tool.description || 'No description supplied'}</small></span>} />;
                            })}
                          </Stack>
                        </Card.Body>
                      </Card>
                    </section>
                    <Input multiline label="Task input" rows={9} value={prompt} onChange={e => setPrompt(e.target.value)} className="resize-y leading-relaxed"
                      placeholder="Describe your task and paste the material to work with." aria-describedby="input-budget" />
                    <p id="input-budget" className={PARA}>{bytes.toLocaleString()} / 16,384 UTF-8 bytes. Text, selected skill{selectedTools.length ? ', selected tool descriptions and approved results' : ''} are sent to the chosen Provider. {selectedTools.length ? 'Unselected tools remain unavailable.' : 'No tools are selected.'}</p>
                    <div><Button type="submit" loading={running} disabled={!ready || locked}>{running ? 'Running task...' : skillId ? 'Run with selected skill' : 'Run without a skill'}</Button></div>
                  </Stack>
                </fieldset>
              </form>
              {running && <Loading label={taskActive ? 'Waiting for the model or approved tool. No tool is automatically retried.' : 'Waiting for the model. You can cancel this task; leaving the page also requests cancellation.'} />}
              {(running || taskActive) && taskRef.current && <div><Button variant="secondary" disabled={cancelPending} loading={cancelPending} onClick={() => void cancelTask()}>{cancelPending ? 'Cancelling task...' : taskActive ? 'Cancel tool task' : 'Cancel task'}</Button></div>}
              {error && <Notice tone="error">{error}</Notice>}
            </Stack>
          </Card.Body>
        </Card>
      </section>
      <section className="min-w-0 grow basis-64" aria-labelledby="result-heading" aria-busy={running}>
        <Card>
          <Card.Header><h2 id="result-heading" className="m-0 text-lg font-semibold">Result</h2></Card.Header>
          <Card.Body>
            <Stack gap={3}>
              {!outcome && <p className={PARA}>No result yet. Responses appear here as plain text, never executable content.</p>}
              {outcome && <>
                <p className="m-0 text-sm wrap-anywhere">Task: {outcome.record.task.id} · {outcome.record.task.status}</p>
                {outcome.record.skills.map(s => <p key={s.version_id} className="m-0 font-mono text-xs text-muted-foreground wrap-anywhere">Bound version: {s.version_id}<br />SHA-256: {s.content_hash}</p>)}
                {!outcome.record.skills.length && <p className={PARA}>Explicitly run without a skill.</p>}
                {outcome.record.tools?.map(tool => <p key={JSON.stringify([tool.server_id, tool.tool_name])} className="m-0 font-mono text-xs text-muted-foreground wrap-anywhere">Selected tool: {tool.tool_name}<br />Server: {tool.server_id}<br />Schema: {tool.schema_hash}</p>)}
                {outcome.content && <pre tabIndex={0} className={cn(PRE, 'font-sans leading-relaxed')}>{outcome.content}</pre>}
                {outcome.audit_warning && <Notice tone="warning">The provider response completed, but usage audit persistence failed. Do not retry automatically; verify the result before repeating the task.</Notice>}
                {toolTask?.audit_warning && <Notice tone="warning">A provider response completed, but usage audit persistence failed. The task was not retried automatically.</Notice>}
              </>}
            </Stack>
          </Card.Body>
        </Card>
        {taskActive && toolTask?.proposal && <section className="mt-6" aria-labelledby="proposal-heading" aria-busy={running}>
          <Card className="border-2 border-primary">
            <Card.Body>
              <Stack gap={3}>
                <h3 id="proposal-heading" ref={proposalHeading} tabIndex={-1} className={cn('m-0 text-base font-semibold', FOCUS)}>Review proposed tool call</h3>
                <p className="m-0 text-sm wrap-anywhere">The model requests <strong>{toolTask.proposal.server_name} / {toolTask.proposal.tool_name}</strong>. Review the exact arguments. Tools may modify data or send it externally; descriptions are not guarantees.</p>
                <pre tabIndex={0} aria-label="Exact tool arguments" className={cn(PRE, 'font-mono')}>{JSON.stringify(toolTask.proposal.arguments, null, 2)}</pre>
                <p className={PARA}>{Math.max(0, Math.ceil((toolTask.proposal.expires_at * 1000 - now) / 1000))} seconds remaining. Approval applies once to this exact call, not future calls.</p>
                <div className="flex flex-wrap gap-3">
                  <Button disabled={running || cancelPending || proposalExpired} onClick={() => void decide('approve')}>Approve this call once</Button>
                  <Button variant="secondary" disabled={running || cancelPending || proposalExpired} onClick={() => void decide('deny')}>Reject and stop</Button>
                </div>
              </Stack>
            </Card.Body>
          </Card>
        </section>}
      </section>
    </div>
    <section className="mt-8" aria-labelledby="history-heading">
      <Card>
        <Card.Header><h2 id="history-heading" className="m-0 text-lg font-semibold">Recent task audit</h2></Card.Header>
        <Card.Body>
          <Stack gap={3}>
            <p className={PARA}>Only versions, tool schema references, status and usage are saved. Full inputs and outputs are not stored and disappear when you leave this page. Tool tasks are cancelled on leaving this page; cancellation cannot undo external effects. No automatic replay after a restart.</p>
            {historyError && <Notice tone="error">{historyError}</Notice>}
            {!history.length && <p className={PARA}>No saved tasks.</p>}
            {!!history.length && <ul className="m-0 list-none p-0">{history.map(row => <li key={row.task.id} className="border-t border-border py-4 wrap-anywhere first:border-t-0 first:pt-0">
              <strong>{row.task.status}</strong> · {row.task.model_id} · {new Date(row.task.created_at * 1000).toLocaleString()}
              <p className={cn(PARA, 'mt-1')}>{row.skills.map(s => s.version_id).join(', ') || 'No skill'}{row.task.error_code ? ` · ${row.task.error_code}` : ''}{row.task.total_tokens != null ? ` · ${row.task.total_tokens} tokens` : ''}</p>
              {!!row.tools?.length && <p className={cn(PARA, 'mt-1')}>{row.tools.map(tool => tool.tool_name).join(', ')} · {row.tools.length} selected tool references</p>}
              <small className="mt-1 block text-xs text-muted-foreground">{row.task.id}</small>
            </li>)}</ul>}
          </Stack>
        </Card.Body>
      </Card>
    </section>
  </main>;
}
