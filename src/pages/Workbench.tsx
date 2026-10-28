import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Link } from 'react-router-dom';
import type { Provider, Model } from '../stores/providerStore';
import type { Skill } from '../stores/skillsStore';
import type { McpServer, McpTool } from '../stores/mcpStore';
import './Workbench.css';

interface CommandResponse<T> { success: boolean; data?: T; error?: { message: string } }
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
  if (!result.success || !result.data) throw new Error(result.error?.message || 'Unable to load configuration');
  return result.data;
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
    }).catch(() => { if (active) { setTools([]); setToolsError('MCP tools unavailable. Text-only tasks still work; open MCP management to connect a server.'); } })
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
    }).catch(e => { if (active) setError(`Unable to load workbench: ${message(e)}`); })
      .finally(() => { if (active) setLoading(false); });
    invoke<TaskRecord[]>('list_workbench_tasks', {})
      .then(records => { if (active) { setHistory(records); setHistoryError(null); } })
      .catch(() => { if (active) setHistoryError('Task history is unavailable. Refresh to try again.'); });
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
      if (active) setModels(allModels.filter(m => m.model_type === 'chat'));
    }).catch(() => { if (active) setError('Unable to load models. Check your provider configuration.'); })
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
      if (mounted.current) setError(`Cancellation not confirmed: ${message(e)}. Check task audit; external effects may already have occurred.`);
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
      if (mounted.current && current === generation.current) setError(`Task not completed: ${message(e)}. No fallback request was sent. Check task history before submitting again.`);
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
      if (mounted.current && current === generation.current) setError(`Task action not completed: ${message(e)}. Do not repeat approval; cancel and check task audit.`);
    } finally { submitting.current = false; if (mounted.current) setRunning(false); }
  }

  return <main className="workbench">
    <header className="workbench-header">
      <div><h1>AI Workbench</h1><p>One task, with the skill you choose. Versions stay fixed for each run.</p></div>
      <button type="button" disabled={locked || loading} onClick={() => setReload(n => n + 1)}>Refresh configuration</button>
    </header>
    <div className="workbench-layout">
      <section className="workbench-composer" aria-labelledby="task-heading">
        <h2 id="task-heading">New task</h2>
        <p>Configure a <Link to="/providers">Provider</Link> and add content from <Link to="/skills">Skills</Link> first.</p>
        {loading && <p role="status">Loading local configuration...</p>}
        {!loading && !providers.length && <p>No ready provider. Add, enable, or migrate a provider in Providers.</p>}
        <form onSubmit={e => { e.preventDefault(); void run(); }}>
          <fieldset disabled={locked || loading}>
            <div className="workbench-model-row">
              <label>Provider<select value={providerId} onChange={e => { setProviderId(e.target.value); setError(null); }}>
                <option value="">Choose a provider</option>{providers.map(p => <option key={p.id} value={p.id}>{p.name}</option>)}
              </select></label>
              <label>Model<select value={modelId} disabled={!providerId || modelsLoading} onChange={e => setModelId(e.target.value)}>
                <option value="">{modelsLoading ? 'Loading models...' : 'Choose a cached model'}</option>{models.map(m => <option key={m.id} value={m.model_id}>{m.model_name}</option>)}
              </select></label>
            </div>
            {!!providerId && !modelsLoading && !models.length && <p>Refresh or select models in Providers before running a task.</p>}
            <label>Skill for this task<select value={skillId} onChange={e => { setSkillId(e.target.value); setWithoutSkill(false); }}>
              <option value="">No skill selected</option>{skills.map(s => <option key={s.id} value={s.skill_id}>{s.name} — v{s.version}</option>)}
            </select></label>
            <p>Only enabled content skills are listed. Compatibility and integrity are rechecked before sending; a rejected skill is never silently skipped.</p>
            {!skillId && <label className="workbench-confirm"><input type="checkbox" checked={withoutSkill} onChange={e => setWithoutSkill(e.target.checked)} />I explicitly want to run without a skill</label>}
            <section className="workbench-tools" aria-labelledby="tools-heading">
              <h3 id="tools-heading">MCP tools for this task <span>{selectedTools.length} / 8</span></h3>
              <p>Optional. Only selected tools are sent to the Provider. Every model-proposed call requires your separate approval.</p>
              {toolsLoading && <p role="status">Loading connected tools...</p>}
              {toolsError && <p>{toolsError}</p>}
              {!toolsLoading && !toolsError && !tools.length && <p>No connected tools. Connect an enabled server in <Link to="/mcp">MCP management</Link>, then refresh.</p>}
              {tools.map(tool => <label className="workbench-tool-choice" key={toolKey(tool)}>
                <input type="checkbox" checked={selectedTools.includes(toolKey(tool))}
                  disabled={!selectedTools.includes(toolKey(tool)) && selectedTools.length >= 8}
                  onChange={e => setSelectedTools(old => e.target.checked ? [...old, toolKey(tool)] : old.filter(key => key !== toolKey(tool)))} />
                <span>{tool.server_name} / {tool.tool_name}<small>{tool.description || 'No description supplied'}</small></span>
              </label>)}
            </section>
            <label>Task input<textarea rows={9} value={prompt} onChange={e => setPrompt(e.target.value)} placeholder="Describe your task and paste the material to work with." aria-describedby="input-budget" /></label>
            <p id="input-budget">{bytes.toLocaleString()} / 16,384 UTF-8 bytes. Text, selected skill{selectedTools.length ? ', selected tool descriptions and approved results' : ''} are sent to the chosen Provider. {selectedTools.length ? 'Unselected tools remain unavailable.' : 'No tools are selected.'}</p>
            <button className="workbench-run" type="submit" disabled={!ready || locked}>{running ? 'Running task...' : skillId ? 'Run with selected skill' : 'Run without a skill'}</button>
          </fieldset>
        </form>
        {running && <p role="status">{taskActive ? 'Waiting for the model or approved tool. No tool is automatically retried.' : 'Waiting for the model. You can cancel this task; leaving the page also requests cancellation.'}</p>}
        {(running || taskActive) && taskRef.current && <button type="button" disabled={cancelPending} onClick={() => void cancelTask()}>{cancelPending ? 'Cancelling task...' : taskActive ? 'Cancel tool task' : 'Cancel task'}</button>}
        {error && <p className="workbench-error" role="alert">{error}</p>}
      </section>
      <section className="workbench-result" aria-labelledby="result-heading" aria-busy={running}>
        <h2 id="result-heading">Result</h2>
        {!outcome && <p>No result yet. Responses appear here as plain text, never executable content.</p>}
        {outcome && <>
          <p>Task: {outcome.record.task.id} · {outcome.record.task.status}</p>
          {outcome.record.skills.map(s => <p key={s.version_id} className="workbench-reference">Bound version: {s.version_id}<br />SHA-256: {s.content_hash}</p>)}
          {!outcome.record.skills.length && <p>Explicitly run without a skill.</p>}
          {outcome.record.tools?.map(tool => <p key={JSON.stringify([tool.server_id, tool.tool_name])} className="workbench-reference">Selected tool: {tool.tool_name}<br />Server: {tool.server_id}<br />Schema: {tool.schema_hash}</p>)}
          {outcome.content && <pre tabIndex={0}>{outcome.content}</pre>}
          {outcome.audit_warning && <p role="alert">The provider response completed, but usage audit persistence failed. Do not retry automatically; verify the result before repeating the task.</p>}
          {toolTask?.audit_warning && <p role="alert">A provider response completed, but usage audit persistence failed. The task was not retried automatically.</p>}
        </>}
        {taskActive && toolTask?.proposal && <section className="workbench-proposal" aria-labelledby="proposal-heading" aria-busy={running}>
          <h3 id="proposal-heading" ref={proposalHeading} tabIndex={-1}>Review proposed tool call</h3>
          <p>The model requests <strong>{toolTask.proposal.server_name} / {toolTask.proposal.tool_name}</strong>. Review the exact arguments. Tools may modify data or send it externally; descriptions are not guarantees.</p>
          <pre tabIndex={0} aria-label="Exact tool arguments">{JSON.stringify(toolTask.proposal.arguments, null, 2)}</pre>
          <p>{Math.max(0, Math.ceil((toolTask.proposal.expires_at * 1000 - now) / 1000))} seconds remaining. Approval applies once to this exact call, not future calls.</p>
          <div className="workbench-proposal-actions">
            <button type="button" disabled={running || cancelPending || now >= toolTask.proposal.expires_at * 1000} onClick={() => void decide('approve')}>Approve this call once</button>
            <button type="button" disabled={running || cancelPending || now >= toolTask.proposal.expires_at * 1000} onClick={() => void decide('deny')}>Reject and stop</button>
          </div>
        </section>}
      </section>
    </div>
    <section className="workbench-history" aria-labelledby="history-heading">
      <h2 id="history-heading">Recent task audit</h2>
      <p>Only versions, tool schema references, status and usage are saved. Full inputs and outputs are not stored and disappear when you leave this page. Tool tasks are cancelled on leaving this page; cancellation cannot undo external effects. No automatic replay after a restart.</p>
      {historyError && <p role="alert">{historyError}</p>}
      {!history.length && <p>No saved tasks.</p>}
      <ul>{history.map(row => <li key={row.task.id}>
        <strong>{row.task.status}</strong> · {row.task.model_id} · {new Date(row.task.created_at * 1000).toLocaleString()}
        <p>{row.skills.map(s => s.version_id).join(', ') || 'No skill'}{row.task.error_code ? ` · ${row.task.error_code}` : ''}{row.task.total_tokens != null ? ` · ${row.task.total_tokens} tokens` : ''}</p>
        {!!row.tools?.length && <p>{row.tools.map(tool => tool.tool_name).join(', ')} · {row.tools.length} selected tool references</p>}
        <small>{row.task.id}</small>
      </li>)}</ul>
    </section>
  </main>;
}
