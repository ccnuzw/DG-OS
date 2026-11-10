import { useEffect, useId, useMemo, useRef, useState } from 'react';
import { Button, Input } from '@dgos/ui';
import { Dialog } from 'radix-ui';
import { Check, ChevronDown, Search, X } from 'lucide-react';
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
type Provider = { id: string; name: string; models: Model[] };
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
        provider = { id: source.id, name: source.name, models: [] };
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
  const filtered = providers.map(provider => ({
    ...provider,
    models: provider.models.filter(model => !normalizedQuery || [provider.name, provider.id, model.name, model.id].some(text => text.toLocaleLowerCase().includes(normalizedQuery))),
  })).filter(provider => provider.models.length > 0 || !normalizedQuery);

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
    <Dialog.Root open={open} onOpenChange={changeOpen}>
      <Dialog.Trigger asChild>
        <Button ref={trigger} variant="ghost" size="sm" className="composer-model-picker-trigger" disabled={disabled || pending || !active} aria-label={t('modelPicker.open', { selection: currentLabel })} aria-invalid={invalid || undefined} data-testid="composer-model-picker-trigger">
          <span>{currentLabel}</span><ChevronDown size={14} aria-hidden="true" />
        </Button>
      </Dialog.Trigger>
      <Dialog.Portal>
        <Dialog.Overlay className="composer-model-picker-overlay" />
        <Dialog.Content className="composer-model-picker-dialog" data-testid="composer-model-picker-dialog" onOpenAutoFocus={event => {
          event.preventDefault();
          document.getElementById(`${id}-search`)?.focus({ preventScroll: true });
        }} onCloseAutoFocus={event => {
          event.preventDefault();
          restoreFocus.current = true;
          requestAnimationFrame(() => {
            const target = trigger.current;
            if (latest.current.active && target?.isConnected && !target.closest('[hidden], [inert]') && !target.disabled) {
              restoreFocus.current = false;
              target.focus({ preventScroll: true });
            }
          });
        }}>
          <header className="composer-model-picker-header">
            <Dialog.Title>{t('modelPicker.title')}</Dialog.Title>
            <Dialog.Close asChild><Button variant="ghost" size="sm" iconOnly aria-label={t('modelPicker.close')}><X size={16} aria-hidden="true" /></Button></Dialog.Close>
          </header>
          <Dialog.Description className="composer-model-picker-description">{t('modelPicker.boundary')}</Dialog.Description>
          <section className="composer-model-picker-current" aria-label={t('modelPicker.current')}>
            <strong>{t('modelPicker.current')}</strong>
            <span>{value ? currentLabel : t('modelPicker.none')}</span>
            {value ? <small>{engineNames[value.engine]}</small> : null}
            {invalid ? <p role="alert" className="composer-model-picker-warning">{t('modelPicker.reselect')}{selectedRoute && !selectedRoute.available ? ` ${reasonText(selectedRoute.reason)}` : ''}</p> : null}
          </section>
          <div className="composer-model-picker-search"><Search size={16} aria-hidden="true" /><Input id={`${id}-search`} type="search" label={t('modelPicker.search')} placeholder={t('modelPicker.searchHint')} value={query} onChange={event => setQuery(event.target.value)} /></div>
          <div className="composer-model-picker-toolbar"><Button variant="secondary" size="xs" disabled={loading} onClick={refresh}>{t('modelPicker.refresh')}</Button>{loading ? <span role="status">{t('modelPicker.loading')}</span> : null}</div>
          <div className="composer-model-picker-results" aria-busy={loading}>
            {scoped?.failed ? <p role="alert" className="composer-model-picker-warning">{t('modelPicker.readFailed')}</p> : null}
            {ready && !providers.length ? <p className="composer-model-picker-empty">{t('modelPicker.empty')}</p> : null}
            {ready && providers.length > 0 && !filtered.length ? <p className="composer-model-picker-empty">{t('modelPicker.noMatches')}</p> : null}
            {ready ? filtered.map(provider => <section key={provider.id} className="composer-model-picker-provider" aria-label={provider.name}>
              <header><h3>{provider.name}</h3></header>
              {!provider.models.length ? <p className="composer-model-picker-empty">{t('modelPicker.noModels')}</p> : null}
              {provider.models.map(model => {
                const selected = value?.provider.provider_id === provider.id && value.provider.model_id === model.id;
                const available = model.routes.some(route => route.available);
                return <div key={model.id} className="composer-model-picker-model">
                  <Button variant="ghost" className="composer-model-picker-option" aria-pressed={Boolean(selected)} disabled={disabled || !available} onClick={() => chooseModel(provider, model)}>
                    <span className="composer-model-picker-model-copy"><strong>{model.name}</strong><small>{model.id}</small></span>
                    {selected ? <span className="composer-model-picker-selected"><Check size={14} aria-hidden="true" />{t('modelPicker.current')}</span> : null}
                  </Button>
                  <details className="composer-model-picker-route-summary"><summary>{t('desktop.advancedRoute')}</summary>
                    <div className="composer-model-picker-routes" role="group" aria-label={t('modelPicker.chooseRoute')}>
                      {model.routes.map(route => <Button key={route.engine} variant="secondary" disabled={disabled || !route.available} aria-pressed={Boolean(selected && value?.engine === route.engine && value.provider.provider_revision === route.revision)} onClick={() => chooseRoute(provider, model, route)}>{engineNames[route.engine]}{!route.available ? ` — ${reasonText(route.reason)}` : ''}</Button>)}
                    </div>
                  </details>
                </div>;
              })}
            </section>) : null}
          </div>
          <footer className="composer-model-picker-footer"><WorkspaceLink to="/settings/providers" onClick={() => changeOpen(false)}>{t('modelPicker.manageProviders')}</WorkspaceLink><WorkspaceLink to="/settings/cli" onClick={() => changeOpen(false)}>{t('modelPicker.manageCli')}</WorkspaceLink></footer>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
    {invalid ? <span role="status" className="composer-model-picker-warning">{t('modelPicker.reselect')}</span> : null}
  </div>;
}
