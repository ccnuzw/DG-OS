import { useEffect, useId, useMemo, useRef, useState } from 'react';
import { Button, Input } from '@dgos/ui';
import { Popover as Primitive } from 'radix-ui';
import { Check, ChevronDown, Cpu, KeyRound, Search, Settings2 } from 'lucide-react';
import { useLocale } from '../../i18n';
import { readCliProviderCatalog, type CliProviderCatalog, type CliProviderConfiguration } from '../../stores/developmentCapabilities';
import { useDevelopment } from '../../stores/developmentStore';
import { contextGeneration, useContextSelection } from '../../stores/developmentSelection';
import { useSurfaceActive } from '../../workspace/SurfaceContext';
import { WorkspaceLink } from '../chat/RuntimeNavigation';
import './composer-model-picker.css';

type Engine = CliProviderConfiguration['engine'];
type Route = { engine: Engine; revision: number; available: boolean; reason: string | null };
type Model = { id: string; name: string; routes: Route[] };
type Provider = { id: string; name: string; platform: string; models: Model[] };
type Scope = { projectId: string; epoch: number; generation: number };
type CatalogState = Scope & { reload: number; ticket: number; providers: Provider[]; failed: boolean; loading: boolean };
const engineNames = { codex: 'Codex', claude: 'Claude' } as const;

function sameScope(left: Scope, right: Scope) {
  return left.projectId === right.projectId && left.epoch === right.epoch && left.generation === right.generation;
}

function mergeCatalogs(catalogs: CliProviderCatalog[]): Provider[] {
  const providers = new Map<string, Provider>();
  for (const catalog of catalogs) {
    for (const source of catalog.providers) {
      let provider = providers.get(source.id);
      if (!provider) {
        provider = { id: source.id, name: source.name, platform: source.provider_type ?? 'other', models: [] };
        providers.set(source.id, provider);
      }
      // Missing or contradictory availability is not evidence of a usable route.
      const available = source.enabled && source.availability?.available === true && source.availability.reason === null;
      const reason = !source.enabled ? 'provider_disabled' : source.availability?.reason ?? (available ? null : 'availability_unknown');
      for (const item of source.models) {
        let model = provider.models.find(entry => entry.id === item.id);
        if (!model) {
          model = { id: item.id, name: item.name, routes: [] };
          provider.models.push(model);
        }
        model.routes.push({ engine: catalog.engine, revision: source.config_revision, available, reason });
      }
    }
  }
  return Array.from(providers.values());
}

export function ComposerModelPicker({ projectId, value, preferred, disabled = false, onChange, onOpenChange }: {
  projectId: string;
  value: CliProviderConfiguration | null;
  preferred: CliProviderConfiguration | null;
  disabled?: boolean;
  onChange: (value: CliProviderConfiguration) => void;
  onOpenChange?: (open: boolean) => void;
}) {
  const { t } = useLocale();
  const id = useId();
  const epoch = useDevelopment(state => state.epoch);
  const { generation, pending } = useContextSelection();
  const active = useSurfaceActive();
  const scope = useMemo(() => ({ projectId, epoch, generation }), [projectId, epoch, generation]);
  const latest = useRef({ ...scope, pending, active, disabled });
  latest.current = { ...scope, pending, active, disabled };
  const operation = useRef(0);
  const trigger = useRef<HTMLButtonElement | null>(null);
  const restoreFocus = useRef(false);
  const [openScope, setOpenScope] = useState<Scope | null>(null);
  const [query, setQuery] = useState('');
  const [platformFilter, setPlatformFilter] = useState<string | null>(null);
  const [providerFilter, setProviderFilter] = useState<string | null>(null);
  const [reload, setReload] = useState(0);
  const [state, setState] = useState<CatalogState | null>(null);
  const open = Boolean(openScope && sameScope(openScope, scope) && active && !pending);
  useEffect(() => { onOpenChange?.(open); }, [open, onOpenChange]);
  const scoped = state && sameScope(state, scope) && state.reload === reload && !pending && active ? state : null;
  const loading = !scoped || scoped.loading;
  const providers = useMemo(() => scoped?.providers ?? [], [scoped]);
  const ready = Boolean(scoped && !scoped.loading && !scoped.failed);
  const selectedProvider = value && providers.find(provider => provider.id === value.provider.provider_id);
  const selectedModel = value && selectedProvider?.models.find(model => model.id === value.provider.model_id);
  const selectedRoute = value && selectedModel?.routes.find(route => route.engine === value.engine && route.revision === value.provider.provider_revision);
  const invalid = Boolean(value && ready && !selectedRoute?.available);
  const normalizedQuery = query.trim().toLocaleLowerCase();
  const platforms = [...new Set(providers.map(provider => provider.platform))];
  const platform = platformFilter && platforms.includes(platformFilter) ? platformFilter : selectedProvider?.platform ?? platforms[0];
  const platformProviders = providers.filter(provider => provider.platform === platform);
  const browsingProvider = platformProviders.find(provider => provider.id === providerFilter) ?? platformProviders.find(provider => provider.id === selectedProvider?.id) ?? platformProviders[0];
  const filteredModels = browsingProvider?.models.filter(model => !normalizedQuery || [model.name, model.id].some(text => text.toLocaleLowerCase().includes(normalizedQuery))) ?? [];
  const platformName = (name: string) => name === 'openai_compatible' ? 'OpenAI' : name === 'anthropic' ? 'Anthropic' : name === 'other' ? t('modelPicker.other') : name;

  function refresh() {
    ++operation.current;
    setReload(current => current + 1);
  }

  useEffect(() => {
    const ticket = ++operation.current;
    const current = () => ticket === operation.current && sameScope(scope, latest.current)
      && epoch === useDevelopment.getState().epoch && generation === contextGeneration()
      && !latest.current.pending && latest.current.active;
    if (!active || pending || !projectId) return;
    setState({ ...scope, reload, ticket, providers: [], failed: false, loading: true });
    void Promise.all([readCliProviderCatalog(projectId, 'codex'), readCliProviderCatalog(projectId, 'claude')]).then(catalogs => {
      if (!current()) return;
      if (catalogs.some((catalog, index) => catalog.project_id !== projectId || catalog.engine !== (index === 0 ? 'codex' : 'claude'))) {
        setState({ ...scope, reload, ticket, providers: [], failed: true, loading: false });
        return;
      }
      setState({ ...scope, reload, ticket, providers: mergeCatalogs(catalogs), failed: false, loading: false });
    }, () => {
      if (current()) setState({ ...scope, reload, ticket, providers: [], failed: true, loading: false });
    });
    return () => { ++operation.current; };
  }, [scope, projectId, epoch, generation, pending, active, reload]);

  useEffect(() => {
    if (!active || pending) return;
    const focus = () => { ++operation.current; setReload(current => current + 1); };
    window.addEventListener('focus', focus);
    return () => window.removeEventListener('focus', focus);
  }, [active, pending]);

  useEffect(() => {
    const target = trigger.current;
    if (!active || !restoreFocus.current || !target) return;
    if (target.isConnected && !target.closest('[hidden], [inert]') && (document.activeElement === document.body || document.activeElement?.closest('[hidden], [inert]'))) {
      restoreFocus.current = false;
      target.focus({ preventScroll: true });
    }
  }, [active]);

  function changeOpen(next: boolean) {
    if (next && (disabled || pending || !active)) return;
    setOpenScope(next ? scope : null);
    setQuery('');
    if (next) { setPlatformFilter(null); setProviderFilter(null); }
    if (next) refresh();
  }

  function canChoose() {
    return ready && !latest.current.disabled && !latest.current.pending && latest.current.active
      && sameScope(scope, latest.current) && epoch === useDevelopment.getState().epoch
      && generation === contextGeneration() && state?.ticket === operation.current;
  }

  function chooseRoute(provider: Provider, model: Model, route: Route) {
    if (!canChoose() || !route.available) return;
    onChange({ engine: route.engine, provider: { provider_id: provider.id, model_id: model.id, provider_revision: route.revision } });
    changeOpen(false);
  }

  function chooseModel(provider: Provider, model: Model) {
    if (!canChoose()) return;
    const routes = model.routes.filter(route => route.available);
    const preferredRoute = preferred && preferred.provider.provider_id === provider.id
      ? routes.find(route => route.engine === preferred.engine && route.revision === preferred.provider.provider_revision)
      : undefined;
    const route = preferredRoute ?? routes.find(item => item.engine === 'codex') ?? routes[0];
    if (route) chooseRoute(provider, model, route);
  }

  function reasonText(reason: string | null) {
    switch (reason) {
      case 'provider_disabled': return t('modelPicker.providerDisabled');
      case 'provider_incompatible': return t('modelPicker.providerIncompatible');
      case 'cli_incompatible': return t('modelPicker.cliUnavailable');
      case 'cli_not_installed':
      case 'cli_not_executable':
      case 'cli_probe_timeout':
      case 'cli_probe_failed':
      case 'cli_start_failed': return t(`dev.error.${reason}`);
      case null:
      case 'availability_unknown': return t('modelPicker.availabilityUnknown');
      default: return t('modelPicker.unavailableReason', { reason });
    }
  }

  const currentLabel = value
    ? `${selectedProvider?.name ?? value.provider.provider_id} / ${selectedModel?.name ?? value.provider.model_id}`
    : t('modelPicker.choose');

  return <div className="composer-model-picker">
    <Primitive.Root open={open} onOpenChange={changeOpen}>
      <Primitive.Trigger asChild>
        <Button ref={trigger} variant="ghost" size="xs" className="composer-model-picker-trigger" disabled={disabled || pending || !active} aria-label={t('modelPicker.open', { selection: currentLabel })} title={currentLabel} aria-invalid={invalid || undefined} data-testid="composer-model-picker-trigger">
          <Cpu size={15} aria-hidden="true" /><span>{selectedModel?.name ?? value?.provider.model_id ?? t('modelPicker.choose')}</span>
        </Button>
      </Primitive.Trigger>
      <Primitive.Portal>
        <Primitive.Content side="top" align="end" sideOffset={10} collisionPadding={12} aria-label={t('modelPicker.title')} className="composer-model-picker-dialog" data-testid="composer-model-picker-dialog" onOpenAutoFocus={event => {
          event.preventDefault(); document.getElementById(`${id}-search`)?.focus({ preventScroll: true });
        }} onCloseAutoFocus={event => {
          event.preventDefault(); restoreFocus.current = true;
          requestAnimationFrame(() => {
            const target = trigger.current;
            if (latest.current.active && target?.isConnected && !target.closest('[hidden], [inert]') && !target.disabled) { restoreFocus.current = false; target.focus({ preventScroll: true }); }
          });
        }}>
          <aside className="composer-model-picker-platforms" aria-label={t('modelPicker.platform')}>
            <h3>{t('modelPicker.platform')}</h3>
            {platforms.map(name => <Button key={name} variant="ghost" aria-pressed={platform === name} onClick={() => { setPlatformFilter(name); setProviderFilter(null); setQuery(''); }}><Cpu size={17} aria-hidden="true" /><span>{platformName(name)}</span></Button>)}
            <WorkspaceLink className="composer-model-picker-manage" to="/settings/providers" onClick={() => changeOpen(false)}><Settings2 size={16} aria-hidden="true" />{t('modelPicker.manageProviders')}</WorkspaceLink>
          </aside>
          <div className="composer-model-picker-main">
            <div className="composer-model-picker-top">
              <label className="composer-model-picker-provider-select"><KeyRound size={19} aria-hidden="true" /><span><span className="sr-only">{t('modelPicker.provider')}</span><select aria-label={t('modelPicker.provider')} value={browsingProvider?.id ?? ''} disabled={!platformProviders.length} onChange={event => { setProviderFilter(event.target.value); setQuery(''); }}>{platformProviders.map(provider => <option key={provider.id} value={provider.id}>{provider.name}</option>)}</select><small>{t('modelPicker.configuredConnection')}</small></span><ChevronDown size={15} aria-hidden="true" /></label>
              <div className="composer-model-picker-search"><Search size={16} aria-hidden="true" /><Input id={`${id}-search`} type="search" aria-label={t('modelPicker.search')} placeholder={t('modelPicker.searchHint')} value={query} onChange={event => setQuery(event.target.value)} /></div>
            </div>
            <div className="composer-model-picker-results" aria-busy={loading}>
              {loading ? <p role="status" className="composer-model-picker-empty">{t('modelPicker.loading')}</p> : null}
              {scoped?.failed ? <p role="alert" className="composer-model-picker-warning">{t('modelPicker.readFailed')}</p> : null}
              {invalid ? <p role="alert" className="composer-model-picker-warning">{t('modelPicker.reselect')}</p> : null}
              {ready && !providers.length ? <p className="composer-model-picker-empty">{t('modelPicker.empty')}</p> : null}
              {ready && browsingProvider && !filteredModels.length ? <p className="composer-model-picker-empty">{t(normalizedQuery ? 'modelPicker.noMatches' : 'modelPicker.noModels')}</p> : null}
              {ready && browsingProvider ? filteredModels.map(model => {
                const selected = value?.provider.provider_id === browsingProvider.id && value.provider.model_id === model.id;
                const available = model.routes.some(route => route.available);
                return <div key={model.id} className="composer-model-picker-model">
                  <Button variant="ghost" className="composer-model-picker-option" aria-pressed={Boolean(selected)} disabled={disabled || !available} title={available ? model.id : reasonText(model.routes[0]?.reason ?? null)} onClick={() => chooseModel(browsingProvider, model)}>
                    <span className="composer-model-picker-model-copy">{model.name}</span>{selected ? <Check size={16} aria-hidden="true" /> : null}
                  </Button>
                  {!available ? <small className="composer-model-picker-unavailable">{reasonText(model.routes[0]?.reason ?? null)}</small> : null}
                </div>;
              }) : null}
            </div>
            <footer className="composer-model-picker-footer"><Button variant="ghost" size="xs" disabled={loading} onClick={refresh}>{t('modelPicker.refresh')}</Button><details><summary>{t('desktop.advancedRoute')}</summary><div className="composer-model-picker-routes" role="group" aria-label={t('modelPicker.chooseRoute')}>
              {browsingProvider && filteredModels.map(model => <div key={model.id}><span>{model.name}</span>{model.routes.map(route => <Button key={route.engine} size="xs" variant="ghost" disabled={disabled || !route.available} onClick={() => chooseRoute(browsingProvider, model, route)}>{engineNames[route.engine]}</Button>)}</div>)}
              <WorkspaceLink to="/settings/cli" onClick={() => changeOpen(false)}>{t('modelPicker.manageCli')}</WorkspaceLink>
            </div></details></footer>
          </div>
        </Primitive.Content>
      </Primitive.Portal>
    </Primitive.Root>
  </div>;
}
