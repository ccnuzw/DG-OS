import { useEffect, useRef, useState } from 'react';
import { Button, Input } from '@dgos/ui';
import { useLocale } from '../../i18n';
import { contextGeneration } from '../../stores/developmentSelection';
import { useDevelopment, type DevelopmentSession, type Project } from '../../stores/developmentStore';
import { acceptDevelopmentHandoff, listDevelopmentHandoffs, prepareDevelopmentHandoff, readDevelopmentWorkspaces, type StoredHandoff, type WorkspaceView } from '../../stores/developmentDelivery';
import { capabilitiesChanged, readCliProviderCatalog, readCliProviderRevision, readSessionConfiguration, saveCliProvider, useProjectCapabilities, type CliProviderCatalog, type CliProviderConfiguration, type CliProviderRevision } from '../../stores/developmentCapabilities';
import { developmentDraftKey, useDevelopmentDrafts, useScopedDevelopmentDraft } from '../../stores/developmentDrafts';
import { ComposerModelPicker } from './ComposerModelPicker';
import { errorText } from './shared';
import { hostCode } from '../../stores/hostError';
import { useSurfaceActive } from '../../workspace/SurfaceContext';

type HandoffDraft = { configuration: CliProviderConfiguration | null; remaining: string; summary: string; prepared: StoredHandoff | null; reserved: boolean; rejected: boolean };

const initialDraft: HandoffDraft = { configuration: null, remaining: '', summary: '', prepared: null, reserved: false, rejected: false };


export function ConversationHandoff({ project, source, preparedHandoffId, onReady, onCancel }: { project: Project; source: DevelopmentSession; preparedHandoffId?: string; onReady: (sessionId: string) => Promise<void>; onCancel: () => void }) {
  const { t } = useLocale();
  const epoch = useDevelopment(state => state.epoch);
  const capabilities = useProjectCapabilities(project.id);
  const draft = useScopedDevelopmentDraft<HandoffDraft>('handoff', [project.id, source.id, 'conversation'], initialDraft);
  const value = draft.value;
  const [sourceBinding, setSourceBinding] = useState<CliProviderConfiguration | null>(null);
  const [workspaces, setWorkspaces] = useState<WorkspaceView | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [consent, setConsent] = useState('');
  const [retainedSummary, setRetainedSummary] = useState<string | null>(null);
  const mounted = useRef(true);
  const flight = useRef(false);
  const generation = useRef(contextGeneration());
  const surfaceActive = useSurfaceActive();
  const owner = JSON.stringify([epoch, project.id, source.id, source.task_id]);
  const latest = useRef({ owner, surfaceActive });
  latest.current = { owner, surfaceActive };
  const selected = workspaces?.workspaces.find(item => workspaces.selected_workspace_id ? item.id === workspaces.selected_workspace_id : item.kind === 'original');
  const handoffKey = developmentDraftKey('handoff', project.id, source.task_id ?? '');
  const hadDraft = useRef(Boolean(useDevelopmentDrafts.getState().entries[developmentDraftKey('handoff', project.id, source.id, 'conversation')]));

  useEffect(() => {
    mounted.current = true;
    const epochAtMount = epoch;
    const generationAtMount = contextGeneration();
    generation.current = generationAtMount;
    let active = true;
    setRetainedSummary(null);
    const current = () => active && mounted.current && epochAtMount === useDevelopment.getState().epoch && generationAtMount === contextGeneration();
    if (!preparedHandoffId && !latestDraft().remaining && !latestDraft().prepared) {
      const oldDelivery = useDevelopmentDrafts.getState().entries[developmentDraftKey('delivery', project.id, source.task_id)]?.value as { sourceId?: string; remaining?: string } | undefined;
      if (oldDelivery?.sourceId === source.id && oldDelivery.remaining) update({ remaining: oldDelivery.remaining });
    }
    void readDevelopmentWorkspaces({ project_id: project.id, task_id: source.task_id }).then(result => {
      if (!current()) return;
      if (result.ok) setWorkspaces(result.value); else setError(result.code);
    });
    void readSessionConfiguration(source.id).then(result => {
      if (current() && result.format === 'provider_bound') setSourceBinding({ engine: result.snapshot.engine, provider: { provider_id: result.snapshot.provider_id, model_id: result.snapshot.model_id, provider_revision: result.snapshot.provider_revision } });
    }).catch(() => {});
    if (source.task_id) void listDevelopmentHandoffs(source.task_id).then(result => {
      if (!current()) return;
      if (!result.ok) { if (preparedHandoffId) setError(result.code); return; }
      const rows = result.value.filter(item => item.source_session_id === source.id && item.project_id === project.id && item.task_id === source.task_id && item.owner === source.owner);
      const latest = latestDraft();
      if (preparedHandoffId) {
        const requested = rows.find(item => item.id === preparedHandoffId);
        if (!requested) { setError('handoff_stale'); return; }
        const entries = useDevelopmentDrafts.getState().entries;
        const oldSummary = entries[developmentDraftKey('handoff', project.id, source.task_id, requested.id)]?.value;
        const oldDelivery = entries[developmentDraftKey('delivery', project.id, source.task_id)]?.value as { sourceId?: string; remaining?: string } | undefined;
        const summary = typeof oldSummary === 'string' ? oldSummary : requested.summary;
        const conflictingDraft = latest.prepared?.id !== requested.id
          ? Boolean(latest.prepared || latest.remaining || latest.summary || latest.reserved)
          : latest.summary !== summary;
        if (conflictingDraft) {
          if (latest.prepared?.id === requested.id) update({ prepared: requested, reserved: Boolean(requested.target_session_id) || latest.reserved });
          setRetainedSummary(summary); return;
        }
        update({ prepared: requested, summary, remaining: latest.remaining || (oldDelivery?.sourceId === source.id ? oldDelivery.remaining ?? '' : ''), reserved: Boolean(requested.target_session_id) || latest.reserved });
        return;
      }
      const known = rows.find(item => item.id === latest.prepared?.id) ?? (!hadDraft.current && !latest.prepared ? rows.filter(item => item.target_session_id).sort((left, right) => right.created_at - left.created_at)[0] : undefined);
      if (known) update({ prepared: known, ...(latest.summary ? {} : { summary: known.summary }), ...(known.target_session_id ? { reserved: true } : {}) });
    });
    return () => { active = false; mounted.current = false; };
  // Refreshes are scoped to the mounted source/account identity.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [project.id, source.id, source.task_id, epoch, preparedHandoffId]);

  useEffect(() => {
    if (!value.configuration && capabilities.value?.configuration) update({ configuration: capabilities.value.configuration });
  }, [value.configuration, capabilities.value?.configuration]);

  function current() {
    return mounted.current && latest.current.owner === owner && latest.current.surfaceActive && epoch === useDevelopment.getState().epoch && generation.current === contextGeneration()
      && useDevelopment.getState().selection.project_id === project.id && useDevelopment.getState().selection.session_id === source.id
      && useDevelopment.getState().projects.some(item => item.id === project.id && item.owner === project.owner && !item.archived);
  }
  function finish() {
    if (latest.current.owner !== owner) return;
    flight.current = false;
    if (mounted.current) setBusy(false);
  }
  function update(patch: Partial<HandoffDraft>) {
    if (epoch !== useDevelopment.getState().epoch || !useDevelopment.getState().projects.some(item => item.id === project.id && item.owner === project.owner)) return;
    const key = developmentDraftKey('handoff', project.id, source.id, 'conversation');
    useDevelopmentDrafts.setState(state => {
      const entry = state.entries[key];
      const latest = entry ? entry.value as HandoffDraft : initialDraft;
      return { entries: { ...state.entries, [key]: { kind: 'handoff', value: { ...latest, ...patch }, baseline: entry?.baseline ?? initialDraft } } };
    });
  }

  function latestDraft() {
    const key = developmentDraftKey('handoff', project.id, source.id, 'conversation');
    return (useDevelopmentDrafts.getState().entries[key]?.value as HandoffDraft | undefined) ?? initialDraft;
  }
  const configuration = value.configuration ?? capabilities.value?.configuration ?? null;
  const originalBinding = sourceBinding ?? (source.model ? { engine: source.engine, provider: { provider_id: '', model_id: source.model, provider_revision: 0 } } : null);
  const sameOriginal = Boolean(configuration && originalBinding && configuration.engine === originalBinding.engine && configuration.provider.model_id === originalBinding.provider.model_id && (!originalBinding.provider.provider_id || configuration.provider.provider_id === originalBinding.provider.provider_id));
  const trustKey = selected ? `${selected.path}\u0000${selected.directory_identity}` : '';
  const trusted = Boolean(trustKey && consent === trustKey);

  function persistConversationDraft(sessionId: string, summary: string) {
    if (epoch !== useDevelopment.getState().epoch || !useDevelopment.getState().projects.some(item => item.id === project.id && item.owner === project.owner)) return;
    const messageKey = developmentDraftKey('conversation', project.id, sessionId, 'message');
    useDevelopmentDrafts.setState(state => state.entries[messageKey] ? state : { entries: { ...state.entries, [messageKey]: { kind: 'conversation', value: summary, baseline: '' } } });
  }
  async function attach(sessionId: string, summary: string) {
    persistConversationDraft(sessionId, summary);
    if (current()) {
      try { await onReady(sessionId); }
      catch { if (current()) setError('unknown'); }
    }
  }

  async function prepare() {
    const config = configuration;
    const remaining = latestDraft().remaining;
    if (!current() || !source.task_id || source.project_id !== project.id || !config || !remaining.trim() || busy || flight.current || sameOriginal) return;
    flight.current = true; setBusy(true); setError(null);
    const result = await prepareDevelopmentHandoff({ task_id: source.task_id, source_session_id: source.id, target_engine: config.engine, remaining });
    const sameAccount = epoch === useDevelopment.getState().epoch && useDevelopment.getState().projects.some(item => item.id === project.id && item.owner === project.owner);
    const latest = latestDraft();
    const latestConfiguration = latest.configuration ?? config;
    const configurationUnchanged = latestConfiguration.engine === config.engine && latestConfiguration.provider.provider_id === config.provider.provider_id && latestConfiguration.provider.model_id === config.provider.model_id && latestConfiguration.provider.provider_revision === config.provider.provider_revision;
    if (result.ok && sameAccount && configurationUnchanged && latest.remaining === remaining) {
      update({ prepared: result.value, summary: preparedHandoffId && latest.summary ? latest.summary : result.value.summary, reserved: false });
      useDevelopmentDrafts.setState(state => ({ handoffs: { ...state.handoffs, [handoffKey]: result.value.id } }));
    } else if (current()) setError(result.ok ? 'handoff_stale' : result.code);
    finish();
  }

  async function accept() {
    const operation = latestDraft();
    const prepared = operation.prepared;
    if (!current() || !source.task_id || source.project_id !== project.id || !prepared || !configuration || prepared.target_engine !== configuration.engine || !selected || !trusted || operation.reserved || busy || flight.current || sameOriginal) return;
    flight.current = true; setBusy(true); setError(null);
    const workspacesResult = await readDevelopmentWorkspaces({ project_id: project.id, task_id: source.task_id });
    if (!current()) { finish(); return; }
    const freshWorkspace = workspacesResult.ok ? workspacesResult.value.workspaces.find(item => item.id === selected.id) : null;
    const stillSelected = workspacesResult.ok && (workspacesResult.value.selected_workspace_id ? workspacesResult.value.selected_workspace_id === selected.id : selected.kind === 'original');
    if (!workspacesResult.ok || !stillSelected || !freshWorkspace || freshWorkspace.path !== selected.path || freshWorkspace.directory_identity !== selected.directory_identity) {
      setError(workspacesResult.ok ? 'workspace_changed' : workspacesResult.code); finish(); return;
    }
    let catalog: CliProviderCatalog;
    try { catalog = await readCliProviderCatalog(project.id, configuration.engine); }
    catch (cause) {
      if (current()) setError(hostCode(cause));
      finish(); return;
    }
    if (!current()) { finish(); return; }
    const provider = catalog.providers.find(item => item.id === configuration.provider.provider_id);
    const modelAvailable = catalog.project_id === project.id && catalog.engine === configuration.engine && provider?.enabled === true && provider.availability?.available === true && provider.availability.reason === null && provider.config_revision === configuration.provider.provider_revision && provider.models.some(item => item.id === configuration.provider.model_id);
    if (!modelAvailable) { setError('provider_revision_changed'); finish(); return; }
    let liveProvider: CliProviderRevision | null;
    try { liveProvider = await readCliProviderRevision(project.id); }
    catch (cause) {
      if (current()) setError(hostCode(cause));
      finish(); return;
    }
    if (!current()) { finish(); return; }
    let revision = liveProvider?.revision ?? 0;
    const configured = liveProvider?.configuration;
    if (!configured || configured.engine !== configuration.engine || configured.provider.provider_id !== configuration.provider.provider_id || configured.provider.model_id !== configuration.provider.model_id || configured.provider.provider_revision !== configuration.provider.provider_revision) {
      try {
        const saved = await saveCliProvider(project.id, revision, configuration);
        if (!current()) { finish(); return; }
        revision = saved.revision;
        capabilitiesChanged();
      } catch (cause) {
        if (current()) setError(hostCode(cause));
        finish(); return;
      }
    }
    const latest = latestDraft();
    const selectedConfiguration = latest.configuration ?? configuration;
    if (!current() || latest.prepared?.id !== prepared.id || selectedConfiguration.engine !== configuration.engine || selectedConfiguration.provider.provider_id !== configuration.provider.provider_id || selectedConfiguration.provider.model_id !== configuration.provider.model_id || selectedConfiguration.provider.provider_revision !== configuration.provider.provider_revision) {
      if (current()) setError('handoff_stale');
      finish(); return;
    }
    const summary = latest.summary;
    update({ reserved: true, rejected: false });
    const result = await acceptDevelopmentHandoff({ handoff_id: prepared.id, summary, confirmed: true, project_revision: revision, provider: configuration.provider, options: { workspace: freshWorkspace.path, resume_id: null, executable: null, model: null, trust_workspace: true } });
    const sameAccount = epoch === useDevelopment.getState().epoch && useDevelopment.getState().projects.some(item => item.id === project.id && item.owner === project.owner);
    if (!sameAccount) { finish(); return; }
    if (!result.ok) {
      if (latestDraft().prepared?.id === prepared.id) update({ rejected: result.code !== 'unknown' });
      if (current()) setError(result.code);
      finish(); return;
    }
    update({ prepared: result.value.handoff, reserved: true });
    useDevelopmentDrafts.setState(state => ({ handoffs: { ...state.handoffs, [handoffKey]: result.value.handoff.id } }));
    await attach(result.value.session.id, summary);
    finish();
  }

  async function recheck() {
    const prepared = latestDraft().prepared;
    if (!current() || !source.task_id || !prepared || !latestDraft().reserved || busy || flight.current) return;
    flight.current = true; setBusy(true); setError(null);
    const result = await listDevelopmentHandoffs(source.task_id);
    const sameAccount = epoch === useDevelopment.getState().epoch && useDevelopment.getState().projects.some(item => item.id === project.id && item.owner === project.owner);
    if (!sameAccount) { finish(); return; }
    if (!result.ok) { if (current()) setError(result.code); finish(); return; }
    const stored = result.value.find(item => item.id === prepared.id);
    if (!stored) { if (current()) setError('unknown'); finish(); return; }
    update({ prepared: stored, reserved: Boolean(stored.target_session_id) || !latestDraft().rejected, rejected: false });
    if (stored.target_session_id) await attach(stored.target_session_id, latestDraft().summary);
    finish();
  }

  async function attachTarget() {
    const latest = latestDraft();
    const sessionId = latest.prepared?.target_session_id;
    if (!current() || !sessionId || busy || flight.current) return;
    await attach(sessionId, latest.summary);
  }

  return <section className="dev-panel dev-review-stack conversation-handoff" data-testid="conversation-handoff">
    <div className="dev-review-toolbar"><h3 className="m-0 text-sm">{t('conversationHandoff.title')}</h3><Button size="xs" variant="secondary" disabled={busy} onClick={onCancel}>{t('conversationHandoff.cancel')}</Button></div>
    {error ? <p role="alert" className="text-xs text-status-error">{errorText(error)}</p> : null}
    <p className="m-0 text-xs">{t('conversationHandoff.notice')}</p>
    <p className="m-0 text-xs">{t('conversationHandoff.historyNotice')}</p>
    {retainedSummary !== null ? <section className="dev-review-stack"><p role="status" className="text-xs">{t('conversationHandoff.existingDraft')}</p><label className="dev-field">{t('delivery.summary')}<textarea className="dev-editor type-reading" rows={6} value={retainedSummary} readOnly /></label></section> : null}
    <Input label={t('conversationHandoff.remaining')} multiline rows={2} value={value.remaining} disabled={busy || Boolean(value.prepared)} onChange={event => update({ remaining: event.target.value })} />
    <ComposerModelPicker projectId={project.id} value={configuration} preferred={capabilities.value?.configuration ?? null} disabled={busy || value.reserved && !value.prepared?.target_session_id} onChange={next => { const latest = latestDraft(); if (!latest.reserved || latest.prepared?.target_session_id) { update({ configuration: next, prepared: null, summary: preparedHandoffId ? latest.summary : '', reserved: false, rejected: false }); if (latest.prepared) useDevelopmentDrafts.setState(state => { const handoffs = { ...state.handoffs }; if (handoffs[handoffKey] === latest.prepared?.id) delete handoffs[handoffKey]; return { handoffs }; }); setConsent(''); } }} />
    {sameOriginal ? <p role="alert" className="text-xs">{t('conversationHandoff.chooseDifferent')}</p> : null}
    <Button size="xs" disabled={busy || source.project_id !== project.id || !source.task_id || !configuration || !value.remaining.trim() || Boolean(value.prepared) || sameOriginal} onClick={() => void prepare()}>{t('conversationHandoff.prepare')}</Button>
    {value.prepared ? <>
      <label className="dev-field">{t('conversationHandoff.summary')}<textarea className="dev-editor type-reading" rows={6} value={value.summary} disabled={busy || value.reserved} onChange={event => update({ summary: event.target.value })} /></label>
      <p className="m-0 break-all text-xs">{selected?.path ?? t('conversationHandoff.loadingWorkspace')}</p>
      <p className="dev-evidence-warning">{t('conversationHandoff.permissionsNotice')}</p>
      <label className="dev-checkbox"><input type="checkbox" checked={trusted} disabled={busy || !selected || value.reserved} onChange={event => setConsent(event.target.checked ? trustKey : '')} />{t('conversationHandoff.trust')}</label>
      {value.reserved ? <><p role="status" className="text-xs">{t('conversationHandoff.reserved')}</p><Button size="xs" variant="secondary" disabled={busy} onClick={() => void recheck()}>{t('conversationHandoff.recheck')}</Button>{value.prepared.target_session_id ? <Button size="xs" variant="secondary" disabled={busy} onClick={() => void attachTarget()}>{t('conversationHandoff.attach')}</Button> : null}</> : null}
      <Button size="xs" disabled={busy || !trusted || value.reserved || !value.summary.trim() || !configuration || value.prepared.target_engine !== configuration.engine || sameOriginal} onClick={() => void accept()}>{t('conversationHandoff.create')}</Button>
    </> : null}
  </section>;
}
