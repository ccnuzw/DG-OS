import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Link } from 'react-router-dom';
import type { Provider, Model } from '../stores/providerStore';
import type { Skill } from '../stores/skillsStore';
import './Workbench.css';

interface CommandResponse<T> { success: boolean; data?: T; error?: { message: string } }
export interface TaskRecord {
  task: { id: string; status: string; provider_id: string; model_id: string; created_at: number; error_code?: string; total_tokens?: number };
  skills: { skill_id: string; version_id: string; content_hash: string }[];
}
interface TaskOutcome { record: TaskRecord; content?: string }
const userId = 'default-user'; // Desktop-only identity, not remote authentication.
const message = (error: unknown) => error instanceof Error ? error.message : String(error);
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
  const submitting = useRef(false);

  useEffect(() => {
    let active = true;
    setLoading(true);
    setError(null);
    Promise.all([
      command<Provider[]>('list_providers', { userId }),
      invoke<Skill[]>('list_installed_skills', { userId }),
    ]).then(([allProviders, allSkills]) => {
      if (!active) return;
      setProviders(allProviders.filter(p => p.enabled));
      setSkills(allSkills.filter(s => s.enabled && s.call_type === 'content'));
    }).catch(e => { if (active) setError(`Unable to load workbench: ${message(e)}`); })
      .finally(() => { if (active) setLoading(false); });
    invoke<TaskRecord[]>('list_workbench_tasks', { userId })
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

  async function run() {
    if (!ready || submitting.current) return;
    submitting.current = true;
    setRunning(true);
    setError(null);
    setOutcome(null);
    try {
      const result = await invoke<TaskOutcome>('run_workbench_task', {
        userId,
        input: { provider_id: providerId, model_id: modelId, prompt, skill_id: skillId || null, allow_without_skill: !skillId && withoutSkill },
      });
      setOutcome(result);
      setHistory(old => [result.record, ...old.filter(row => row.task.id !== result.record.task.id)].slice(0, 50));
      if (result.record.task.status !== 'succeeded') setError(`Task ${result.record.task.status}: ${result.record.task.error_code || 'unknown_error'}. Nothing was retried without your selected skill.`);
    } catch (e) {
      setError(`Task not completed: ${message(e)}. No fallback request was sent. Check task history before submitting again.`);
    } finally {
      submitting.current = false;
      setRunning(false);
    }
  }

  return <main className="workbench">
    <header className="workbench-header">
      <div><h1>AI Workbench</h1><p>One task, with the skill you choose. Versions stay fixed for each run.</p></div>
      <button type="button" disabled={running || loading} onClick={() => setReload(n => n + 1)}>Refresh configuration</button>
    </header>
    <div className="workbench-layout">
      <section className="workbench-composer" aria-labelledby="task-heading">
        <h2 id="task-heading">New task</h2>
        <p>Configure a <Link to="/providers">Provider</Link> and add content from <Link to="/skills">Skills</Link> first.</p>
        {loading && <p role="status">Loading local configuration...</p>}
        {!loading && !providers.length && <p>No enabled provider. Add or enable one in Providers.</p>}
        <form onSubmit={e => { e.preventDefault(); void run(); }}>
          <fieldset disabled={running || loading}>
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
            <label>Task input<textarea rows={9} value={prompt} onChange={e => setPrompt(e.target.value)} placeholder="Describe your task and paste the material to work with." aria-describedby="input-budget" /></label>
            <p id="input-budget">{bytes.toLocaleString()} / 16,384 UTF-8 bytes. Text and selected skill are sent to the chosen Provider. No tools are available.</p>
            <button className="workbench-run" type="submit" disabled={!ready || running}>{running ? 'Running task...' : skillId ? 'Run with selected skill' : 'Run without a skill'}</button>
          </fieldset>
        </form>
        {running && <p role="status">Waiting for the model. Leaving this page does not cancel the request.</p>}
        {error && <p className="workbench-error" role="alert">{error}</p>}
      </section>
      <section className="workbench-result" aria-labelledby="result-heading" aria-busy={running}>
        <h2 id="result-heading">Result</h2>
        {!outcome && <p>No result yet. Responses appear here as plain text, never executable content.</p>}
        {outcome && <>
          <p>Task: {outcome.record.task.id} · {outcome.record.task.status}</p>
          {outcome.record.skills.map(s => <p key={s.version_id} className="workbench-reference">Bound version: {s.version_id}<br />SHA-256: {s.content_hash}</p>)}
          {!outcome.record.skills.length && <p>Explicitly run without a skill.</p>}
          {outcome.content && <pre tabIndex={0}>{outcome.content}</pre>}
        </>}
      </section>
    </div>
    <section className="workbench-history" aria-labelledby="history-heading">
      <h2 id="history-heading">Recent task audit</h2>
      <p>Only versions, status and usage are saved. Full inputs and outputs are not stored and disappear when you leave this page. No automatic replay after a restart.</p>
      {historyError && <p role="alert">{historyError}</p>}
      {!history.length && <p>No saved tasks.</p>}
      <ul>{history.map(row => <li key={row.task.id}>
        <strong>{row.task.status}</strong> · {row.task.model_id} · {new Date(row.task.created_at * 1000).toLocaleString()}
        <p>{row.skills.map(s => s.version_id).join(', ') || 'No skill'}{row.task.error_code ? ` · ${row.task.error_code}` : ''}{row.task.total_tokens != null ? ` · ${row.task.total_tokens} tokens` : ''}</p>
        <small>{row.task.id}</small>
      </li>)}</ul>
    </section>
  </main>;
}
