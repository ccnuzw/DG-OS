import { useEffect, useRef, useState } from 'react';
import { Button, Select } from '@dgos/ui';
import { useLocale } from '../../i18n';
import { useDevelopment } from '../../stores/developmentStore';
import { contextGeneration, useContextSelection } from '../../stores/developmentSelection';
import { capabilitiesChanged, readCapabilityCatalog, saveCapabilities, revokeCapabilities, useProjectCapabilities, type CapabilityCatalog, type CapabilityConfiguration } from '../../stores/developmentCapabilities';
import { hostCode } from '../../stores/hostError';
import { errorText } from './shared';

export function ProjectCapabilities({ projectId }: { projectId: string }) {
  const { t } = useLocale();
  const epoch = useDevelopment(state => state.epoch);
  const { generation } = useContextSelection();
  const saved = useProjectCapabilities(projectId);
  const [catalog, setCatalog] = useState<CapabilityCatalog | null>(null);
  const [configuration, setConfiguration] = useState<CapabilityConfiguration | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [reload, setReload] = useState(0);
  const operation = useRef(0);
  useEffect(() => {
    let disposed = false;
    setCatalog(null); setError(null); setNotice(null); setBusy(false);
    void readCapabilityCatalog().then(value => {
      if (!disposed && epoch === useDevelopment.getState().epoch && generation === contextGeneration()) setCatalog(value);
    }, failure => {
      if (!disposed && epoch === useDevelopment.getState().epoch && generation === contextGeneration()) setError(hostCode(failure));
    });
    return () => { disposed = true; ++operation.current; };
  }, [projectId, epoch, generation, reload]);
  useEffect(() => { setConfiguration(saved.loading ? null : saved.value?.configuration ?? { engine: 'codex', provider: { source: 'native' }, skills: [], mcp: [] }); }, [saved.loading, saved.value, projectId]);
  const provider = configuration?.provider;
  const selectedProvider = provider?.source === 'dgos' ? catalog?.providers.find(item => item.id === provider.provider_id) : null;
  const compatibleProviderType = configuration?.engine === 'claude' ? 'anthropic' : 'openai_compatible';
  const unavailable = Boolean(configuration && catalog && (
    provider?.source === 'dgos' && (!selectedProvider?.enabled || selectedProvider.provider_type !== compatibleProviderType || selectedProvider.config_revision !== provider.revision || !selectedProvider.models.some(item => item.id === provider.model_id)) ||
    configuration.skills.some(skill => !catalog.skills.some(item => item.enabled && item.id === skill.id && item.version === skill.version && item.hash === skill.hash)) ||
    configuration.mcp.some(tool => !catalog.mcp.some(item => item.server_id === tool.server_id && item.tool_name === tool.tool_name && item.schema_hash === tool.schema_hash))
  ));
  async function commit(revoke: boolean) {
    if (busy || !revoke && (!configuration || saved.loading || saved.error)) return;
    const ticket = ++operation.current;
    setBusy(true); setError(null); setNotice(null);
    try {
      if (revoke) await revokeCapabilities(projectId);
      else if (configuration) await saveCapabilities(projectId, saved.value?.revision ?? 0, configuration);
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
    <p className="m-0 text-xs text-muted-foreground">{t('capabilities.editHint')}</p>
    <p className="dev-evidence-warning">{t('capabilities.unverified')}</p>
    {saved.loading || !catalog && !error ? <p role="status" className="text-xs">{t('capabilities.loading')}</p> : null}
    {error || saved.error ? <p role="alert" className="text-xs text-destructive">{errorText(error ?? hostCode(saved.error))}</p> : null}
    {notice ? <p role="status" className="text-xs">{t(notice)}</p> : null}
    {configuration && catalog && !saved.error ? <>
      <p className="m-0 text-xs">{t('capabilities.revision', { revision: saved.value?.revision ?? 0, epoch: saved.value?.permission_epoch ?? 0 })}</p>
      <fieldset className="dev-runtime-fields" disabled={busy}><legend>{t('capabilities.provider')}</legend>
        <Select label={t('agent.engine')} value={configuration.engine} onChange={value => setConfiguration({ ...configuration, engine: value === 'claude' ? 'claude' : 'codex' })} options={[{ value: 'codex', label: 'Codex' }, { value: 'claude', label: 'Claude' }]} />
        <p className="m-0 text-xs text-muted-foreground">{t(configuration.engine === 'codex' ? 'capabilities.responses' : 'capabilities.messages')}</p>
        <label className="dev-field">{t('capabilities.source')}<select value={provider?.source === 'dgos' ? provider.provider_id : ''} onChange={event => {
          const item = catalog.providers.find(entry => entry.id === event.target.value);
          setConfiguration({ ...configuration, provider: item ? { source: 'dgos', provider_id: item.id, model_id: item.models[0]?.id ?? '', revision: item.config_revision } : { source: 'native' } });
        }}><option value="">{t('capabilities.native')}</option>{catalog.providers.map(item => <option key={item.id} value={item.id} disabled={!item.enabled || !item.models.length || item.provider_type !== compatibleProviderType}>{t('capabilities.catalogProvider', { name: item.name, type: item.provider_type, revision: item.config_revision })}{item.enabled ? '' : ` — ${t('capabilities.disabled')}`}</option>)}{provider?.source === 'dgos' && !selectedProvider ? <option value={provider.provider_id} disabled>{provider.provider_id} — {t('capabilities.unavailable')}</option> : null}</select></label>
        {provider?.source === 'dgos' ? <><label className="dev-field">{t('agent.model')}<select value={provider.model_id} onChange={event => setConfiguration({ ...configuration, provider: { ...provider, model_id: event.target.value } })}>{(selectedProvider?.models ?? []).map(item => <option key={item.id} value={item.id}>{item.name} ({item.model_type})</option>)}{!selectedProvider?.models.some(item => item.id === provider.model_id) ? <option value={provider.model_id} disabled>{provider.model_id} — {t('capabilities.unavailable')}</option> : null}</select></label><p className="m-0 break-all text-xs">{t('capabilities.providerPin', { revision: provider.revision })}</p></> : <p className="m-0 text-xs text-muted-foreground">{t('capabilities.nativeHint')}</p>}
        {provider?.source === 'dgos' && selectedProvider?.enabled && selectedProvider.config_revision !== provider.revision ? <Button size="xs" variant="secondary" onClick={() => setConfiguration({ ...configuration, provider: { ...provider, revision: selectedProvider.config_revision } })}>{t('capabilities.useCurrentProvider', { revision: selectedProvider.config_revision })}</Button> : null}
      </fieldset>
      <fieldset className="dev-runtime-fields" disabled={busy}><legend>{t('capabilities.skills')}</legend>
        <p className="m-0 text-xs text-muted-foreground">{t('capabilities.skillBoundary')}</p>
        {!catalog.skills.length ? <p className="text-xs">{t('capabilities.emptySkills')}</p> : null}
        {catalog.skills.map(item => <label className="dev-checkbox" key={`${item.id}:${item.version}:${item.hash}`}><input type="checkbox" disabled={!item.enabled && !configuration.skills.some(skill => skill.id === item.id && skill.version === item.version && skill.hash === item.hash)} checked={configuration.skills.some(skill => skill.id === item.id && skill.version === item.version && skill.hash === item.hash)} onChange={event => setConfiguration({ ...configuration, skills: event.target.checked ? [...configuration.skills.filter(skill => skill.id !== item.id), { id: item.id, version: item.version, hash: item.hash }] : configuration.skills.filter(skill => skill.id !== item.id || skill.version !== item.version || skill.hash !== item.hash) })} /><span className="min-w-0 break-all">{item.name} · {item.version}{!item.enabled ? ` — ${t('capabilities.disabled')}` : ''}<br /><span className="text-muted-foreground">{item.hash}</span></span></label>)}
        {configuration.skills.filter(skill => !catalog.skills.some(item => item.id === skill.id && item.version === skill.version && item.hash === skill.hash)).map(skill => <label key={`${skill.id}:${skill.hash}`} className="dev-checkbox"><input type="checkbox" checked onChange={() => setConfiguration({ ...configuration, skills: configuration.skills.filter(item => item !== skill) })} /><span className="break-all">{skill.id} · {skill.version} · {skill.hash} — {t('capabilities.unavailable')}</span></label>)}
      </fieldset>
      <fieldset className="dev-runtime-fields" disabled={busy}><legend>{t('capabilities.mcp')}</legend>
        <p className="m-0 text-xs text-muted-foreground">{t('capabilities.mcpBoundary')}</p>
        {!catalog.mcp.length ? <p className="text-xs">{t('capabilities.emptyMcp')}</p> : null}
        {catalog.mcp.map(item => <div className="dev-review-stack" key={`${item.server_id}:${item.tool_name}:${item.schema_hash}`}><label className="dev-checkbox"><input type="checkbox" checked={configuration.mcp.some(tool => tool.server_id === item.server_id && tool.tool_name === item.tool_name && tool.schema_hash === item.schema_hash)} onChange={event => setConfiguration({ ...configuration, mcp: event.target.checked ? [...configuration.mcp.filter(tool => tool.server_id !== item.server_id || tool.tool_name !== item.tool_name), { server_id: item.server_id, tool_name: item.tool_name, schema_hash: item.schema_hash }] : configuration.mcp.filter(tool => tool.server_id !== item.server_id || tool.tool_name !== item.tool_name || tool.schema_hash !== item.schema_hash) })} /><span className="min-w-0 break-all">{item.server_name} / {item.tool_name}<br /><span className="text-muted-foreground">{t('capabilities.schema')}: {item.schema_hash}</span></span></label><details><summary className="text-xs">{t('capabilities.inputSchema')}</summary><pre className="dev-event-text">{JSON.stringify(item.input_schema, null, 2)}</pre></details></div>)}
        {configuration.mcp.filter(tool => !catalog.mcp.some(item => item.server_id === tool.server_id && item.tool_name === tool.tool_name && item.schema_hash === tool.schema_hash)).map(tool => <label key={`${tool.server_id}:${tool.tool_name}:${tool.schema_hash}`} className="dev-checkbox"><input type="checkbox" checked onChange={() => setConfiguration({ ...configuration, mcp: configuration.mcp.filter(item => item !== tool) })} /><span className="break-all">{tool.server_id} / {tool.tool_name} · {tool.schema_hash} — {t('capabilities.unavailable')}</span></label>)}
      </fieldset>
      {unavailable ? <p role="alert" className="dev-evidence-warning">{t('capabilities.staleCatalog')}</p> : null}
      <Button size="xs" disabled={busy || unavailable || saved.loading} onClick={() => void commit(false)}>{t(busy ? 'capabilities.working' : 'capabilities.save')}</Button>
    </> : null}
    <Button size="xs" variant="secondary" disabled={busy} onClick={() => void commit(true)}>{t('capabilities.revoke')}</Button>
    <p className="m-0 text-xs text-muted-foreground">{t('capabilities.revokeHint')}</p>
  </section>;
}
