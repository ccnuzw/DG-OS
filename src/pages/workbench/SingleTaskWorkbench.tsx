import { useLocale as usePageLocale } from '../../i18n';
import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Link } from 'react-router-dom';
import { Button, Card, Checkbox, Input, Loading, Select, Stack, StatusIcon, cn } from '@dgos/ui';
import { FOCUS_RING } from '@dgos/ui/lib/focus';
import { usePageError } from '../../pageError';
import { formatDateTime, formatNumber, useLocale } from '../../i18n';
import type { Provider, Model } from '../../stores/providerStore';
import type { Skill } from '../../stores/skillsStore';
import type { McpServer, McpTool } from '../../stores/mcpStore';
import { useTaskRunner, type TaskRecord } from '../../stores/taskRunnerStore';
import { useSurfaceActive, useSurfaceCloseGuard, useSurfaceStatus } from '../../workspace/SurfaceContext';
import { confirmDiscard, prepareTaskClose } from '../../workspace/closeCoordinator';
import { Modal } from '@dgos/ui';
import Markdown from '../../components/chat/Markdown';
import './workbench.css';
import { GenerationControls, OrderedSkills, type GenerationParameters } from '../../components/chat/GenerationControls';

interface CommandResponse<T> { success: boolean; data?: T; error?: { code?: number; message: string } }
interface AvailableTool extends McpTool { server_name: string }
const toolKey = (tool: McpTool) => JSON.stringify([tool.server_id, tool.tool_name, tool.schema_hash]);
const message = (error: unknown) => error instanceof Error ? error.message : String(error);
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
const PARA = 'm-0 text-sm text-ink-secondary';
const LINK = cn('text-accent-default underline', FOCUS_RING);
const PRE = cn('m-0 rounded-md bg-surface-inset p-4 text-sm whitespace-pre-wrap wrap-anywhere', FOCUS_RING);

function Notice({ tone, children }: { tone: 'error' | 'warning'; children: ReactNode }) {
  usePageLocale();
  return (
    <div role="alert" className={cn('flex items-start gap-2 rounded-md border-l-4 p-3 text-sm',
      tone === 'error' ? 'border-status-error bg-status-error-surface text-status-error-text' : 'border-status-warning bg-status-warning-surface text-status-warning-text')}>
      <StatusIcon name={tone === 'error' ? 'error' : 'warning'} className="mt-0.5" />
      <p className="m-0 min-w-0 wrap-anywhere">{children}</p>
    </div>
  );
}

export default function SingleTaskWorkbench() {
  usePageLocale();
  const { t } = useLocale();
  const [providers, setProviders] = useState<Provider[]>([]);
  const [models, setModels] = useState<Model[]>([]);
  const [skills, setSkills] = useState<Skill[]>([]);
  const [providerId, setProviderId] = useState('');
  const [modelId, setModelId] = useState('');
  const [skillIds, setSkillIds] = useState<string[]>([]);
  const [generation, setGeneration] = useState<GenerationParameters>({});
  const [withoutSkill, setWithoutSkill] = useState(false);
  const [prompt, setPrompt] = useState('');
  const [loading, setLoading] = useState(true);
  const [modelsLoading, setModelsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [historyError, setHistoryError] = useState<string | null>(null);
  const [history, setHistory] = useState<TaskRecord[]>([]);
  const [reload, setReload] = useState(0);
  const [tools, setTools] = useState<AvailableTool[]>([]);
  const [selectedTools, setSelectedTools] = useState<string[]>([]);
  const [toolsError, setToolsError] = useState<string | null>(null);
  const [toolsLoading, setToolsLoading] = useState(true);
  const [now, setNow] = useState(Date.now());
  const [metadataWake, setMetadataWake] = useState(0);
  const [modelsWake, setModelsWake] = useState(0);
  const mounted = useRef(false);
  const metadataFlight = useRef<{ reload: number; refresh: boolean } | null>(null);
  const modelsFlight = useRef<{ reload: number; refresh: boolean } | null>(null);
  const currentProvider = useRef(providerId);
  currentProvider.current = providerId;
  const proposalHeading = useRef<HTMLHeadingElement>(null);
  const lastProposalId = useRef<string | undefined>(undefined);
  const reportError = usePageError('Workbench');
  const report = useRef(reportError);
  useEffect(() => { report.current = reportError; }, [reportError]);
  const runner = useTaskRunner();
  const { running, cancelPending, error: runnerError, outcome, toolTask } = runner;
  const surfaceActive = useSurfaceActive();
  const taskRef = { current: runner.taskId };
  useSurfaceStatus({ dirty: Boolean(prompt), running: Boolean(runner.taskId) });
  useSurfaceCloseGuard(async () => {
    const current = useTaskRunner.getState();
    if (current.taskId) {
      const accepted = await new Promise<boolean>(resolve => Modal.confirm({ title: t('workbench.running'), content: t('shell.closeRunningConfirm'), confirmText: t('shell.stopAndClose'), cancelText: t('common.cancel'), confirmVariant: 'danger', onConfirm: () => resolve(true), onCancel: () => resolve(false) }));
      if (!accepted) return false;
      return await prepareTaskClose();
    }
    return !prompt.trim() || await confirmDiscard();
  });
  const taskActive = toolTask?.record.task.status === 'running';
  const locked = running || !!runner.taskId || taskActive || cancelPending;
  const currentActivity = useRef(surfaceActive);
  currentActivity.current = surfaceActive;
  const accountEpoch = runner.epoch;
  useLayoutEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);
  useLayoutEffect(() => {
    currentProvider.current = '';
    setProviders([]); setSkills([]); setTools([]); setHistory([]);
    setProviderId(''); setModelId(''); setSkillIds([]); setSelectedTools([]);
    setGeneration({}); setWithoutSkill(false); setPrompt('');
    setLoading(true); setToolsLoading(true);
    setError(null); setToolsError(null); setHistoryError(null);
  }, [accountEpoch]);
  useLayoutEffect(() => {
    setModels([]);
    setModelsLoading(false);
  }, [providerId, accountEpoch]);


  const proposalId = toolTask?.proposal?.id;
  const proposalExpiresAt = toolTask?.proposal?.expires_at;
  useEffect(() => {
    const isNewProposal = proposalId !== lastProposalId.current;
    lastProposalId.current = proposalId;
    if (isNewProposal && proposalId && taskActive && surfaceActive) proposalHeading.current?.focus();
  }, [proposalId, taskActive, surfaceActive]);

  useLayoutEffect(() => {
    if (!surfaceActive || !taskActive || !proposalId || proposalExpiresAt === undefined) return;
    const expiresAt = proposalExpiresAt * 1000;
    let timer: number | undefined;
    function updateCountdown() {
      const currentTime = Date.now();
      setNow(currentTime);
      if (currentTime < expiresAt) timer = window.setTimeout(updateCountdown, Math.min(1000, expiresAt - currentTime));
    }
    updateCountdown();
    return () => window.clearTimeout(timer);
  }, [proposalId, proposalExpiresAt, surfaceActive, taskActive]);

  useEffect(() => {
    if (!surfaceActive) {
      if (metadataFlight.current) metadataFlight.current.refresh = true;
      return;
    }
    if (metadataFlight.current) {
      if (metadataFlight.current.reload !== reload) metadataFlight.current.refresh = true;
      return;
    }
    const flight = { reload, refresh: false };
    metadataFlight.current = flight;
    const current = () => mounted.current && useTaskRunner.getState().epoch === accountEpoch;
    setLoading(true);
    setToolsLoading(true);
    setError(null);
    setToolsError(null);
    // Hiding preserves this result and requests at most one fresh snapshot after restoration.
    const toolsRead = invoke<McpServer[]>('list_mcp_servers').then(async servers => {
      if (!current()) return;
      if (!currentActivity.current) { flight.refresh = true; return; }
      const groups = await Promise.allSettled(servers.filter(s => s.enabled && s.status === 'ready').map(async server => {
        const listed = await invoke<McpTool[]>('list_mcp_tools', { serverId: server.id });
        return listed.map(tool => ({ ...tool, server_name: server.name }));
      }));
      const tools: AvailableTool[] = [];
      for (const group of groups) {
        if (group.status === 'rejected') throw group.reason;
        tools.push(...group.value);
      }
      if (current()) setTools(tools);
    }).catch(e => { if (current()) { setToolsError(t('workbench.toolsUnavailable')); report.current(e); } })
      .finally(() => { if (current()) setToolsLoading(false); });
    const configRead = Promise.allSettled([
      command<Provider[]>('list_providers', {}),
      invoke<Skill[]>('list_installed_skills', {}),
    ]).then(([providerResult, skillResult]) => {
      if (!current()) return;
      if (providerResult.status === 'rejected') throw providerResult.reason;
      if (skillResult.status === 'rejected') throw skillResult.reason;
      const allProviders = providerResult.value;
      const allSkills = skillResult.value;
      setProviders(allProviders.filter(p => p.enabled && p.credential_status === 'stored'));
      setSkills(allSkills.filter(s => s.enabled && s.call_type === 'content'));
    }).catch(e => { if (current()) { setError(t('workbench.loadFailed', { error: message(e) })); report.current(e); } })
      .finally(() => { if (current()) setLoading(false); });
    const historyRead = invoke<TaskRecord[]>('list_workbench_tasks', {})
      .then(records => { if (current()) { setHistory(records); setHistoryError(null); } })
      .catch(e => { if (current()) { setHistoryError(t('workbench.historyUnavailable')); report.current(e); } });
    void Promise.all([toolsRead, configRead, historyRead]).finally(() => {
      metadataFlight.current = null;
      if (mounted.current && currentActivity.current && (flight.refresh || useTaskRunner.getState().epoch !== accountEpoch)) setMetadataWake(value => value + 1);
    });
  }, [surfaceActive, reload, accountEpoch, metadataWake]);

  useEffect(() => {
    if (!surfaceActive) {
      if (modelsFlight.current) modelsFlight.current.refresh = true;
      return;
    }
    if (!providerId || currentProvider.current !== providerId) return;
    setModelsLoading(true);
    if (modelsFlight.current) {
      if (modelsFlight.current.reload !== reload) modelsFlight.current.refresh = true;
      return;
    }
    const flight = { reload, refresh: false };
    modelsFlight.current = flight;
    const current = () => mounted.current && useTaskRunner.getState().epoch === accountEpoch && currentProvider.current === providerId;
    // Keep a successful same-provider snapshot while refreshing, including after a failure.
    void command<Model[]>('list_models', { providerId }).then(allModels => {
      if (current()) setModels(allModels.filter(m => m.provider_id === providerId && (m.model_type === 'text-generation' || m.model_type === 'chat')));
    }).catch(e => { if (current()) { setError(t('workbench.modelsFailed')); report.current(e); } })
      .finally(() => {
        modelsFlight.current = null;
        if (current()) setModelsLoading(false);
        if (mounted.current && currentActivity.current && (flight.refresh || !current())) setModelsWake(value => value + 1);
      });
  }, [surfaceActive, providerId, reload, accountEpoch, modelsWake]);

  const bytes = new TextEncoder().encode(prompt).length;
  const ready = !loading && !modelsLoading && !toolsLoading && !toolsError && providers.some(p => p.id === providerId)
    && models.some(m => m.model_id === modelId && m.provider_id === providerId) && prompt.trim() && bytes <= 16384
    && (skillIds.length ? skillIds.every(id => skills.some(s => s.skill_id === id)) : withoutSkill);

  async function run() {
    if (!ready || locked) return;
    const chosen = tools.filter(tool => selectedTools.includes(toolKey(tool)));
    if (chosen.length !== selectedTools.length) { setError(t('workbench.selectedToolsChanged')); return; }
    const epoch = useTaskRunner.getState().epoch;
    await runner.run({ provider_id: providerId, model_id: modelId, prompt, skill_ids: skillIds, generation, allow_without_skill: !skillIds.length && withoutSkill }, chosen.map(tool => ({ server_id: tool.server_id, tool_name: tool.tool_name, schema_hash: tool.schema_hash })));
    if (!mounted.current || useTaskRunner.getState().epoch !== epoch) return;
    const result = useTaskRunner.getState().outcome;
    if (result) setHistory(old => [result.record, ...old.filter(row => row.task.id !== result.record.task.id)].slice(0, 50));
  }
  const cancelTask = runner.cancel;
  const decide = runner.decide;

  const proposalExpired = !!toolTask?.proposal && now >= toolTask.proposal.expires_at * 1000;
  const providerOptions = [{ value: NONE, label: t('workbench.chooseProvider') }, ...providers.map(p => ({ value: p.id, label: p.name }))];
  const modelPlaceholder = modelsLoading ? t('workbench.loadingModels') : t('workbench.cachedModel');
  const modelOptions = [{ value: NONE, label: modelPlaceholder }, ...models.map(m => ({ value: m.model_id, label: m.model_name }))];
  const fromSelect = (value: string) => value === NONE ? '' : value;

  return <div className="task-workbench">
    <header className="task-header">
      <div className="min-w-0">
        <h1 className="m-0 text-lg font-semibold">{t('workbench.single')}</h1>
        <p className={PARA}>{t('workbench.subtitle')}</p>
      </div>
      <Button variant="secondary" disabled={locked || loading} onClick={() => setReload(n => n + 1)}>{t('workbench.refreshConfig')}</Button>
    </header>
    <div className="task-columns">
      <section className="min-w-0 grow basis-64" aria-labelledby="task-heading">
        <Card>
          <Card.Header><h2 id="task-heading" className="m-0 text-lg font-semibold">{t('workbench.newTask')}</h2></Card.Header>
          <Card.Body>
            <Stack gap={4}>
              <p className={PARA}>{t('workbench.configureFirst')} <Link to="/settings/providers" className={LINK}>{t('workbench.provider')}</Link> · <Link to="/skills" className={LINK}>{t('nav.skills')}</Link></p>
              {loading && <Loading label={t('workbench.loadingConfig')} />}
              {!loading && !providers.length && <p className={PARA}>{t('workbench.noReadyProvider')}</p>}
              <form onSubmit={e => { e.preventDefault(); void run(); }}>
                <fieldset disabled={locked || loading} className="m-0 min-w-0 border-0 p-0">
                  <Stack gap={4}>
                    <div className="flex flex-wrap gap-4">
                      <div className="min-w-0 grow basis-40">
                        <Select label={t('workbench.provider')} options={providerOptions} value={providerId} placeholder={t('workbench.chooseProvider')} className="overflow-hidden"
                          onChange={value => { setProviderId(fromSelect(value)); setModelId(''); setError(null); }} />
                      </div>
                      <div className="min-w-0 grow basis-40">
                        <Select label={t('workbench.model')} options={modelOptions} value={modelId} placeholder={modelPlaceholder} className="overflow-hidden"
                          disabled={!providerId || modelsLoading} onChange={value => setModelId(fromSelect(value))} />
                      </div>
                    </div>
                    {!!providerId && !modelsLoading && !models.length && <p className={PARA}>{t('workbench.refreshModels')}</p>}
                    <OrderedSkills skills={skills} selected={skillIds} onChange={ids => { setSkillIds(ids); setWithoutSkill(false); }} without={withoutSkill} onWithoutChange={setWithoutSkill} disabled={locked} />
                    <GenerationControls value={generation} onChange={setGeneration} model={models.find(m => m.model_id === modelId)} disabled={locked} />
                    <section aria-labelledby="tools-heading">
                      <Card className="bg-surface-canvas shadow-none">
                        <Card.Body>
                          <Stack gap={3}>
                            <h3 id="tools-heading" className="m-0 text-base font-semibold">{t('workbench.tools')} <span className="font-normal text-ink-secondary">{t('workbench.toolCount', { selected: selectedTools.length, limit: 8 })}</span></h3>
                            <p className={PARA}>{t('workbench.optionalTools')}</p>
                            {toolsLoading && <Loading label={t('workbench.loadingTools')} />}
                            {toolsError && <p className={PARA}>{toolsError}</p>}
                            {!toolsLoading && !toolsError && !tools.length && <p className={PARA}>{t('workbench.noTools')}</p>}
                            {tools.map(tool => {
                              const key = toolKey(tool);
                              const checked = selectedTools.includes(key);
                              return <Checkbox key={key} className="max-w-full items-start wrap-anywhere" checked={checked}
                                disabled={!checked && selectedTools.length >= 8}
                                onChange={on => setSelectedTools(old => on ? [...old, key] : old.filter(k => k !== key))}
                                 label={<span>{tool.server_name} / {tool.tool_name}<small className="mt-1 block text-xs text-ink-secondary">{tool.description || t('workbench.noDescription')}</small></span>} />;
                            })}
                          </Stack>
                        </Card.Body>
                      </Card>
                    </section>
                    <Input multiline label={t('workbench.taskInput')} rows={9} value={prompt} onChange={e => setPrompt(e.target.value)} className="type-reading resize-y"
                      placeholder={t('workbench.promptPlaceholder')} aria-describedby="input-budget" />
                    <p id="input-budget" className={PARA}>{formatNumber(bytes)} / {formatNumber(16_384)} {t('detail.inputBudget')}{selectedTools.length ? t('workbench.toolsIncluded') : ''} {t('detail.sentToProvider')}{selectedTools.length ? t('workbench.toolsExcluded') : t('workbench.noToolsSelected')}</p>
                    <div><Button type="submit" loading={running} disabled={!ready || locked}>{running ? t('workbench.running') : skillIds.length ? t('workbench.runWithSkill') : t('workbench.runWithoutSkill')}</Button></div>
                  </Stack>
                </fieldset>
              </form>
              {running && <Loading label={taskActive ? t('workbench.waitingTool') : t('workbench.waitingModel')} />}
              {(running || taskActive) && taskRef.current && <div><Button variant="secondary" disabled={cancelPending} loading={cancelPending} onClick={() => void cancelTask()}>{cancelPending ? t('workbench.cancelling') : taskActive ? t('workbench.cancelToolTask') : t('workbench.cancelTask')}</Button></div>}
              {error && <Notice tone="error">{error}</Notice>}
              {runnerError && <Notice tone="error">{runnerError}</Notice>}
              {runner.resultUnknown && <Button variant="secondary" onClick={() => void runner.verify()}>{t('common.retry')}</Button>}
            </Stack>
          </Card.Body>
        </Card>
      </section>
      <section className="min-w-0 grow basis-64" aria-labelledby="result-heading" aria-busy={running}>
        <Card>
          <Card.Header><h2 id="result-heading" className="m-0 text-lg font-semibold">{t('workbench.result')}</h2></Card.Header>
          <Card.Body>
            <Stack gap={3}>
              {!outcome && <p className={PARA}>{t('workbench.noResult')}</p>}
              {outcome && <>
                <p className="m-0 text-sm wrap-anywhere">{t('detail.task')}{outcome.record.task.id} · {outcome.record.task.status}</p>
                {outcome.record.skills.map(s => <p key={s.version_id} className="m-0 font-mono text-xs text-ink-secondary wrap-anywhere">{t('detail.boundVersion')}{s.version_id}<br />{t('common.sha256')} {s.content_hash}</p>)}
                {!outcome.record.skills.length && <p className={PARA}>{t('workbench.explicitNoSkill')}</p>}
                {outcome.record.tools?.map(tool => <p key={JSON.stringify([tool.server_id, tool.tool_name])} className="m-0 font-mono text-xs text-ink-secondary wrap-anywhere">{t('detail.selectedTool')}{tool.tool_name}<br />{t('detail.server')}{tool.server_id}<br />{t('detail.schema')}{tool.schema_hash}</p>)}
                {outcome.content && <div className="task-reading"><Markdown source={outcome.content} /></div>}
                {outcome.audit_warning && <Notice tone="warning">{t('workbench.providerAuditWarning')}</Notice>}
                {toolTask?.audit_warning && <Notice tone="warning">{t('workbench.providerToolAuditWarning')}</Notice>}
              </>}
            </Stack>
          </Card.Body>
        </Card>
        {taskActive && toolTask?.proposal && <section className="mt-6" aria-labelledby="proposal-heading" aria-busy={running}>
          <Card className="border-2 border-accent-default">
            <Card.Body>
              <Stack gap={3}>
                <h3 id="proposal-heading" ref={proposalHeading} tabIndex={-1} className={cn('m-0 text-base font-semibold', FOCUS_RING)}>{t('workbench.reviewTool')}</h3>
                <p className="m-0 text-sm wrap-anywhere">{t('workbench.modelRequests')}<strong>{toolTask.proposal.server_name} / {toolTask.proposal.tool_name}</strong>{t('detail.reviewArgs')}</p>
                <pre tabIndex={0} aria-label={t('workbench.exactToolArgs')} className={cn(PRE, 'font-mono')}>{JSON.stringify(toolTask.proposal.arguments, null, 2)}</pre>
                <p className={PARA}>{Math.max(0, Math.ceil((toolTask.proposal.expires_at * 1000 - now) / 1000))} {t('detail.remaining')}</p>
                <div className="flex flex-wrap gap-3">
                  <Button disabled={running || cancelPending || proposalExpired} onClick={() => void decide('approve')}>{t('workbench.approveOnce')}</Button>
                  <Button variant="secondary" disabled={running || cancelPending || proposalExpired} onClick={() => void decide('deny')}>{t('workbench.rejectStop')}</Button>
                </div>
              </Stack>
            </Card.Body>
          </Card>
        </section>}
      </section>
    </div>
    <section className="task-history" aria-labelledby="history-heading">
      <Card>
        <Card.Header><h2 id="history-heading" className="m-0 text-lg font-semibold">{t('workbench.history')}</h2></Card.Header>
        <Card.Body>
          <Stack gap={3}>
            <p className={PARA}>{t('workbench.historyPrivacy')}</p>
            {historyError && <Notice tone="error">{historyError}</Notice>}
            {!history.length && <p className={PARA}>{t('workbench.noSavedTasks')}</p>}
            {!!history.length && <ul className="m-0 list-none p-0">{history.map(row => <li key={row.task.id} className="border-t border-border-subtle py-4 wrap-anywhere first:border-t-0 first:pt-0">
              <strong>{t('workbench.status')}: {row.task.status}</strong> · {row.task.model_id} · {formatDateTime(row.task.created_at * 1000)}
              <p className={cn(PARA, 'mt-1')}>{row.skills.map(s => s.version_id).join(', ') || t('workbench.noSkillHistory')}{row.task.error_code ? ` · ${row.task.error_code}` : ''}{row.task.total_tokens != null ? ` · ${t('workbench.tokenCount', { count: formatNumber(row.task.total_tokens) })}` : ''}</p>
              {!!row.tools?.length && <p className={cn(PARA, 'mt-1')}>{row.tools.map(tool => tool.tool_name).join(', ')} · {row.tools.length} {t('detail.toolReferences')}</p>}
              <small className="mt-1 block text-xs text-ink-secondary">{row.task.id}</small>
            </li>)}</ul>}
          </Stack>
        </Card.Body>
      </Card>
    </section>
  </div>;
}
