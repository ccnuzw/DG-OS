import { useEffect, useRef, useState } from 'react';
import { Button, Select } from '@dgos/ui';
import { useLocale } from '../../i18n';
import { useDevelopment } from '../../stores/developmentStore';
import { contextGeneration, useContextSelection } from '../../stores/developmentSelection';
import { capabilitiesChanged, readCliProviderCatalog, saveCliProvider, revokeCliProvider, useProjectCapabilities, type CliProviderCatalog, type CliProviderConfiguration, type CliProviderRevision, type CliProviderSnapshot } from '../../stores/developmentCapabilities';
import { hostCode } from '../../stores/hostError';
import { errorText } from './shared';
import { useScopedDevelopmentDraft } from '../../stores/developmentDrafts';
import { CapabilitySnapshotView } from './CapabilitySnapshotView';
import { WorkspaceLink } from '../chat/RuntimeNavigation';

export function ProjectCapabilities({ projectId, snapshot, onUseConfiguration }: { projectId: string; snapshot?: CliProviderSnapshot | null; onUseConfiguration?: (revision: CliProviderRevision) => void }) {
  const { t } = useLocale();
  const epoch = useDevelopment(state => state.epoch);
  const { generation } = useContextSelection();
  const saved = useProjectCapabilities(projectId);
  const draft = useScopedDevelopmentDraft('capabilities', [projectId], { configuration: saved.value?.configuration ?? null as CliProviderConfiguration | null, revision: saved.value?.revision ?? 0, engine: saved.value?.configuration.engine ?? 'codex' as CliProviderConfiguration['engine'] });
  const [catalog, setCatalog] = useState<CliProviderCatalog | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [reload, setReload] = useState(0);
  const operation = useRef(0);
  const configuration = draft.value.configuration;
  const engine = draft.value.engine ?? configuration?.engine ?? saved.value?.configuration.engine ?? 'codex';
  const provider = configuration?.provider;
  useEffect(() => {
    let disposed = false;
    setCatalog(null); setError(null); setNotice(null); setBusy(false);
    void readCliProviderCatalog(projectId, engine).then(value => {
      if (!disposed && epoch === useDevelopment.getState().epoch && generation === contextGeneration()) setCatalog(value);
    }, failure => {
      if (!disposed && epoch === useDevelopment.getState().epoch && generation === contextGeneration()) setError(hostCode(failure));
    });
    return () => { disposed = true; ++operation.current; };
  }, [projectId, engine, epoch, generation, reload]);
  const selectedProvider = provider && catalog?.providers.find(item => item.id === provider.provider_id);
  const unavailable = Boolean(provider && catalog && (!selectedProvider?.enabled || selectedProvider.config_revision !== provider.provider_revision || !selectedProvider.models.some(item => item.id === provider.model_id)));
  async function commit(revoke: boolean) {
    if (busy || !revoke && (!configuration || saved.loading || saved.error || unavailable)) return;
    if (revoke && !await draft.discard()) return;
    if (epoch !== useDevelopment.getState().epoch || generation !== contextGeneration()) return;
    const ticket = ++operation.current;
    setBusy(true); setError(null); setNotice(null);
    try {
      if (revoke) await revokeCliProvider(projectId);
      else if (configuration) { await saveCliProvider(projectId, draft.value.revision, configuration); draft.clear(draft.value); }
      if (ticket !== operation.current || epoch !== useDevelopment.getState().epoch || generation !== contextGeneration()) return;
      setNotice(revoke ? 'capabilities.revoked' : 'capabilities.saved');
      capabilitiesChanged();
    } catch (failure) {
      if (ticket === operation.current && epoch === useDevelopment.getState().epoch && generation === contextGeneration()) setError(hostCode(failure));
    } finally {
      if (ticket === operation.current && epoch === useDevelopment.getState().epoch && generation === contextGeneration()) setBusy(false);
    }
  }
  return <section className="dev-review-stack" aria-label={t('capabilities.title')}>
    <div className="dev-review-toolbar"><h3 className="m-0 text-sm font-semibold">{t('capabilities.title')}</h3><Button size="xs" variant="secondary" disabled={busy} onClick={() => { capabilitiesChanged(); setReload(value => value + 1); }}>{t('capabilities.reload')}</Button></div>
    <p className="m-0 text-xs text-ink-secondary">{t('capabilities.editHint')}</p>
    <nav className="dev-review-toolbar" aria-label={t('workflow.manageCapabilities')}><WorkspaceLink to="/settings/providers">{t('capabilities.provider')}</WorkspaceLink></nav>
    {snapshot ? <details><summary>{t('workflow.currentConfiguration', { revision: snapshot.revision })}</summary><CapabilitySnapshotView snapshot={snapshot} /></details> : <p className="m-0 text-xs text-ink-secondary">{t('capabilities.noSnapshot')}</p>}
    {saved.value ? <p className="m-0 text-xs" data-testid="dev-capability-comparison">{t(snapshot && snapshot.revision === saved.value.revision && snapshot.permission_epoch === saved.value.permission_epoch ? 'workflow.configurationMatches' : 'workflow.configurationDiffers', { revision: saved.value.revision })}</p> : null}
    {onUseConfiguration && saved.value ? <Button size="xs" data-testid="dev-use-capabilities" disabled={busy || saved.loading || Boolean(saved.error) || draft.dirty || unavailable} onClick={() => onUseConfiguration(saved.value!)}>{t('workflow.useConfiguration')}</Button> : null}
    {saved.loading || !catalog && !error ? <p role="status" className="text-xs">{t('capabilities.loading')}</p> : null}
    {error || saved.error ? <p role="alert" className="text-xs text-status-error">{errorText(error ?? hostCode(saved.error))}</p> : null}
    {notice ? <p role="status" className="text-xs">{t(notice)}</p> : null}
    {catalog && !saved.loading && !saved.error ? <>
      <p className="m-0 text-xs">{t('capabilities.revision', { revision: saved.value?.revision ?? 0, epoch: saved.value?.permission_epoch ?? 0 })}</p>
      <fieldset className="dev-runtime-fields" disabled={busy}><legend>{t('capabilities.provider')}</legend>
        <Select label={t('agent.engine')} value={engine} onChange={value => { const next = value === 'claude' ? 'claude' : 'codex'; draft.setValue({ ...draft.value, engine: next, configuration: null }); }} options={[{ value: 'codex', label: 'Codex' }, { value: 'claude', label: 'Claude' }]} />
        <label className="dev-field">{t('capabilities.source')}<select value={provider?.provider_id ?? ''} onChange={event => {
          const item = catalog.providers.find(entry => entry.id === event.target.value);
          if (item?.models[0]) draft.setValue({ ...draft.value, configuration: { engine, provider: { provider_id: item.id, model_id: item.models[0].id, provider_revision: item.config_revision } } });
        }}><option value="">{t('capabilities.chooseProvider')}</option>{catalog.providers.map(item => <option key={item.id} value={item.id} disabled={!item.enabled || !item.models.length}>{item.name} · {t('capabilities.providerPin', { revision: item.config_revision })}{item.enabled ? '' : ` — ${t('capabilities.disabled')}`}</option>)}</select></label>
        {provider ? <><label className="dev-field">{t('agent.model')}<select value={provider.model_id} onChange={event => draft.setValue({ ...draft.value, configuration: { ...configuration!, provider: { ...provider, model_id: event.target.value } } })}>{(selectedProvider?.models ?? []).map(item => <option key={item.id} value={item.id}>{item.name} ({item.model_type})</option>)}</select></label><p className="m-0 break-all text-xs">{t('capabilities.providerPin', { revision: provider.provider_revision })}</p>
          {selectedProvider?.enabled && selectedProvider.config_revision !== provider.provider_revision ? <Button size="xs" variant="secondary" onClick={() => draft.setValue({ ...draft.value, configuration: { ...configuration!, provider: { ...provider, provider_revision: selectedProvider.config_revision } } })}>{t('capabilities.useCurrentProvider', { revision: selectedProvider.config_revision })}</Button> : null}</> : null}
      </fieldset>
      {unavailable ? <p role="alert" className="dev-evidence-warning">{t('capabilities.staleCatalog')}</p> : null}
      <Button size="xs" disabled={busy || unavailable || saved.loading || !configuration} onClick={() => void commit(false)}>{t(busy ? 'capabilities.working' : 'capabilities.save')}</Button>
      {draft.dirty ? <Button size="xs" variant="secondary" disabled={busy} onClick={() => void draft.discard()}>{t('dev.discardDraft')}</Button> : null}
    </> : null}
    <Button size="xs" variant="secondary" disabled={busy} onClick={() => void commit(true)}>{t('capabilities.revoke')}</Button>
    <p className="m-0 text-xs text-ink-secondary">{t('capabilities.revokeHint')}</p>
  </section>;
}
