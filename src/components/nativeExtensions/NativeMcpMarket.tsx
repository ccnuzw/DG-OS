import { useEffect, useRef, useState } from 'react';
import { Button, Input, Select } from '@dgos/ui';
import { ChevronLeft, Search, Server } from 'lucide-react';
import { t } from '../../i18n';
import { useDevelopment } from '../../stores/developmentStore';
import { nativeCatalogKey, registryDeployments, registryParameters, registryToDraft, RegistryConfigurationError, useNativeMcpCatalog } from '../../stores/nativeMcpCatalog';
import { confirmDiscard } from '../../workspace/closeCoordinator';
import { useNativeManagerGuard } from './NativeManagerLifecycle';
import type { NativeDraft, NativeSelection } from '../../types/nativeExtensions';
import './native-mcp-catalog.css';

export default function NativeMcpMarket({ selection, disabled, onConfigure }: { selection: NativeSelection; disabled: boolean; onConfigure: (draft: NativeDraft) => boolean | Promise<boolean> }) {
  const epoch = useDevelopment(state => state.epoch), key = useDevelopment(() => nativeCatalogKey(selection));
  const catalog = useNativeMcpCatalog(state => state.catalogs[key]);
  const [query, setQuery] = useState(''), [selected, setSelected] = useState(''), [deploymentId, setDeploymentId] = useState('');
  const [parameters, setParameters] = useState<Record<string, string>>({}), [message, setMessage] = useState(''), [busy, setBusy] = useState(false), [narrowDetail, setNarrowDetail] = useState(false);
  const revision = useRef(0), dirty = useRef(false), inFlight = useRef(false);
  const searchRef = useRef<HTMLInputElement>(null);
  useEffect(() => { revision.current++; dirty.current = false; inFlight.current = false; setQuery(''); setSelected(''); setDeploymentId(''); setParameters({}); setMessage(''); setBusy(false); setNarrowDetail(false); }, [key, epoch]);
  useNativeManagerGuard(async reason => {
    if (inFlight.current) { setMessage(t('nativeMcpCatalog.closeBlocked')); return false; }
    if (reason === 'view') return true;
    if (!dirty.current) return true;
    const captured = revision.current;
    if (!(await confirmDiscard()) || captured !== revision.current) return false;
    return true;
  });
  const entry = catalog?.page?.servers.find(item => `${item.server.name}@${item.server.version}` === selected);
  const deployments = entry ? registryDeployments(entry) : [];
  const deployment = deployments.find(item => item.id === deploymentId);
  const fields = deployment ? registryParameters(deployment) : [];
  const choose = async (id: string) => {
    const captured = revision.current;
    if (dirty.current && !(await confirmDiscard()) || captured !== revision.current) return;
    revision.current++; dirty.current = false; setParameters({}); setMessage(''); setSelected(id); setDeploymentId(''); setNarrowDetail(true);
  };
  const configure = async () => {
    if (!entry || !deployment || inFlight.current || disabled) return;
    const captured = revision.current; inFlight.current = true; setBusy(true); setMessage('');
    const result = await useNativeMcpCatalog.getState().resolve(selection, entry);
    if (captured !== revision.current || key !== nativeCatalogKey(selection)) return;
    inFlight.current = false; setBusy(false);
    if (!result.ok) { setMessage(t(result.code.startsWith('registry_') ? `nativeMcpCatalog.error.${result.code}` : `nativeExtensions.error.${result.code}`)); return; }
    try {
      const fresh = registryDeployments(result.value).find(item => item.id === deployment.id);
      if (!fresh) { setMessage(t('nativeExtensions.error.native_extension_Conflict')); return; }
      const values = Object.fromEntries(fields.map(field => [field.key, parameters[field.key] ?? field.defaultValue]));
      const draft = registryToDraft(result.value, fresh, values, selection);
      if (await onConfigure(draft) && captured === revision.current && key === nativeCatalogKey(selection)) dirty.current = false;
    } catch (error) { setMessage(t(`nativeMcpCatalog.conversion.${error instanceof RegistryConfigurationError ? error.code : 'invalidParameter'}`)); }
  };
  return <section className="native-mcp-market" aria-label={t('nativeMcpCatalog.market')}>
    <form className="native-mcp-catalog-toolbar" onSubmit={event => { event.preventDefault(); if (!disabled && !busy) void useNativeMcpCatalog.getState().search(selection, query.trim()); }}>
      <Input ref={searchRef} type="search" label={t('nativeMcpCatalog.searchLabel')} value={query} onChange={event => setQuery(event.target.value)} disabled={disabled || busy}/>
      <Button type="submit" disabled={disabled || busy || catalog?.loading || Boolean(query.trim() && query.trim().length < 3)}><Search aria-hidden="true" />{t('nativeMcpCatalog.search')}</Button>
    </form>
    <p className="native-mcp-catalog-note">{t('nativeMcpCatalog.registryNotice')}</p>
    {catalog?.loading && <p role="status">{t('nativeMcpCatalog.fetching')}</p>}
    {catalog?.error && <p role="alert">{t(catalog.error.startsWith('registry_') ? `nativeMcpCatalog.error.${catalog.error}` : `nativeExtensions.error.${catalog.error}`)}</p>}
    {catalog?.page && <p role="status">{t(catalog.page.stale ? 'nativeMcpCatalog.stale' : 'nativeMcpCatalog.fetched', { time: new Date(catalog.page.fetched_at * 1000).toLocaleString() })}{catalog.query !== query.trim() && ` ${t('nativeMcpCatalog.previousQuery', { query: catalog.query })}`}</p>}
    <div className={`native-mcp-catalog-columns${narrowDetail ? ' native-mcp-catalog-show-detail' : ''}`}>
      <div className="native-mcp-catalog-results">
        {!catalog && <div className="native-manager-empty"><Server aria-hidden="true" /><p>{t('nativeMcpCatalog.explicitFetch')}</p></div>}
        {catalog?.page?.servers.length === 0 && !catalog.loading && !catalog.error && <div className="native-manager-empty"><Search aria-hidden="true" /><p>{t('nativeMcpCatalog.empty')}</p></div>}
        <ul className="native-mcp-catalog-list">{catalog?.page?.servers.map(item => {
          const id = `${item.server.name}@${item.server.version}`;
          return <li key={id}><button type="button" aria-pressed={selected === id} disabled={disabled || busy} onClick={() => void choose(id)}><span className="native-resource-icon" aria-hidden="true"><Server /></span><span className="native-mcp-catalog-row-body"><strong>{item.server.title || item.server.name}</strong><span className="native-mcp-catalog-description">{item.server.description}</span><small>{item.server.name} · {item.server.version}</small></span></button></li>;
        })}</ul>
        {catalog?.page?.next_cursor && <Button variant="secondary" disabled={disabled || busy || catalog.loading || query.trim() !== catalog.query || Boolean(catalog.error)} onClick={() => void useNativeMcpCatalog.getState().search(selection, catalog.query, true)}>{t('nativeMcpCatalog.nextPage')}</Button>}
      </div>
      <div className="native-mcp-catalog-detail">
        {entry ? <>
          <Button className="native-mcp-catalog-back" variant="ghost" onClick={() => { setNarrowDetail(false); requestAnimationFrame(() => searchRef.current?.focus()); }}><ChevronLeft aria-hidden="true" />{t('nativeMcpCatalog.back')}</Button>
          <header><span className="native-resource-icon" aria-hidden="true"><Server /></span><h3>{entry.server.title || entry.server.name}</h3></header><p>{entry.server.description}</p>
          <dl><dt>{t('nativeMcpCatalog.namespace')}</dt><dd>{entry.server.name.split('/')[0]}</dd><dt>{t('nativeMcpCatalog.version')}</dt><dd>{entry.server.version}</dd><dt>{t('nativeMcpCatalog.source')}</dt><dd>{entry.server.repository?.url ?? catalog?.page?.source}</dd></dl>
          <Select label={t('nativeMcpCatalog.deployment')} value={deploymentId} disabled={disabled || busy} options={[{ value: '', label: t('nativeMcpCatalog.chooseDeployment') }, ...deployments.map(item => ({ value: item.id, label: item.label }))]} onChange={async id => { const captured = revision.current; if (dirty.current && !(await confirmDiscard()) || captured !== revision.current) return; revision.current++; dirty.current = false; setDeploymentId(id); setParameters({}); setMessage(''); }}/>
          {deployment?.package && <p className="native-mcp-catalog-note">{t('nativeMcpCatalog.packageWarning', { runtime: deployment.package.runtimeHint ?? deployment.package.registryType })}</p>}
          {deployment?.package?.registryType === 'cargo' && <p className="native-mcp-catalog-note">{t('nativeMcpCatalog.cargoNotice')}</p>}
          {deployment && fields.map(field => <div key={field.key} className="native-mcp-catalog-field">
            {field.choices?.length ? <Select label={`${field.label}${field.required ? ' *' : ''}`} options={field.choices.map(value => ({ value, label: value }))} value={parameters[field.key] ?? field.defaultValue} disabled={disabled || busy} onChange={value => { revision.current++; dirty.current = true; setParameters(current => ({ ...current, [field.key]: value })); }}/>
              : field.repeated ? <Input multiline label={`${field.label}${field.required ? ' *' : ''}`} value={parameters[field.key] ?? field.defaultValue} disabled={disabled || busy} onChange={event => { revision.current++; dirty.current = true; setParameters(current => ({ ...current, [field.key]: event.target.value })); }}/>
                : <Input label={`${field.key === 'entrypoint' ? t('nativeMcpCatalog.entrypoint') : field.label}${field.required ? ' *' : ''}`} value={parameters[field.key] ?? field.defaultValue} disabled={disabled || busy} onChange={event => { revision.current++; dirty.current = true; setParameters(current => ({ ...current, [field.key]: event.target.value })); }}/>} 
            {field.description && <p>{field.description}</p>}{field.reference && <p>{t('nativeMcpCatalog.environmentReference')}</p>}
            {field.repeated && <p>{t('nativeMcpCatalog.repeatedParameter')}</p>}
          </div>)}
          {message && <p role="alert">{message}</p>}
          <footer className="native-mcp-catalog-detail-actions"><Button disabled={disabled || busy || !deployment} onClick={() => void configure()}>{t(busy ? 'nativeMcpCatalog.revalidating' : 'nativeMcpCatalog.configure')}</Button><p className="native-mcp-catalog-note">{t('nativeMcpCatalog.configureNotice')}</p></footer>
        </> : <div className="native-manager-empty"><Server aria-hidden="true" /><p>{t('nativeMcpCatalog.selectEntry')}</p></div>}
      </div>
    </div>
  </section>;
}
