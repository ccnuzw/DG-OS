import { useEffect, useId, useMemo, useState } from 'react';
import { Button, Input, Select } from '@dgos/ui';
import { formatNumber, useLocale } from '../i18n';
import { MODEL_KINDS } from '../stores/providerStore';
import type { CapabilityState, Model, ModelCapabilities } from '../stores/providerStore';
import './provider-models.css';

export interface ProviderModelsProps {
  models: Model[];
  editMode: boolean;
  busy: boolean;
  refreshing: boolean;
  mutationBusy: string | null;
  blockedReason?: string;
  error: string | null;
  onRefresh: () => void;
  onAdd: (modelId: string, kind?: string) => Promise<boolean>;
  onRemove: (model: Model) => Promise<boolean>;
}

const PAGE_SIZE = 20;
const CAPABILITIES = ['streaming', 'vision', 'tools', 'reasoning', 'structured_output'] as const;

function kindOf(model: Model): string {
  return model.model_type === 'chat' ? 'text-generation' : model.model_type;
}

function identityOf(model: Model): string {
  return model.id || `${model.provider_id}:${model.model_id}`;
}

function capabilityOf(model: Model, capability: keyof ModelCapabilities): CapabilityState {
  if (model.capabilities) return model.capabilities[capability] ?? 'unknown';
  // Legacy booleans only establish support. False meant "not reported", not a confirmed lack of support.
  if (capability === 'streaming' && model.supports_streaming) return 'supported';
  if (capability === 'vision' && model.supports_vision) return 'supported';
  return 'unknown';
}

export default function ProviderModels({
  models, editMode, busy, refreshing, mutationBusy, blockedReason, error, onRefresh, onAdd, onRemove,
}: ProviderModelsProps) {
  const { locale, t } = useLocale();
  const id = useId();
  const [search, setSearch] = useState('');
  const [kind, setKind] = useState('all');
  const [page, setPage] = useState(1);
  const [expanded, setExpanded] = useState<string | null>(null);
  const [manualOpen, setManualOpen] = useState(false);
  const [manualId, setManualId] = useState('');
  const [manualKind, setManualKind] = useState('auto');
  const [adding, setAdding] = useState(false);
  const [removing, setRemoving] = useState<string | null>(null);
  const disabled = busy || !!blockedReason;
  const mutationDisabled = disabled || mutationBusy !== null || adding || removing !== null;
  const kindLabel = (value: string) => MODEL_KINDS.some((known) => known === value)
    ? t(`provider.catalog.kind.${value}`)
    : t('provider.catalog.unknown');
  const number = (value: number) => formatNumber(value, locale);

  const counts = useMemo(() => {
    const result = new Map<string, number>();
    for (const model of models) {
      const value = kindOf(model);
      result.set(value, (result.get(value) ?? 0) + 1);
    }
    return result;
  }, [models]);
  const filtered = useMemo(() => {
    const query = search.trim().toLowerCase();
    return models.filter((model) => (kind === 'all' || kindOf(model) === kind)
      && (!query || model.model_name.toLowerCase().includes(query) || model.model_id.toLowerCase().includes(query)));
  }, [models, search, kind]);
  const pageCount = Math.max(1, Math.ceil(filtered.length / PAGE_SIZE));
  const currentPage = Math.min(page, pageCount);
  useEffect(() => {
    setPage((current) => Math.min(current, pageCount));
  }, [pageCount]);
  const start = (currentPage - 1) * PAGE_SIZE;
  const visibleModels = filtered.slice(start, start + PAGE_SIZE);
  const kindOptions = [
    { value: 'all', label: t('provider.catalog.allKinds', { count: number(models.length) }) },
    ...[...MODEL_KINDS, ...[...counts.keys()].filter((value) => !MODEL_KINDS.some((known) => known === value))]
      .filter((value) => counts.has(value) || value === kind)
      .map((value) => ({ value, label: t('provider.catalog.kindCount', { kind: kindLabel(value), count: number(counts.get(value) ?? 0) }) })),
  ];
  // Keep a vanished category selectable until the user clears it; never silently change the chosen filter.
  if (kind !== 'all' && !kindOptions.some((option) => option.value === kind)) {
    kindOptions.push({ value: kind, label: t('provider.catalog.kindCount', { kind: kindLabel(kind), count: number(0) }) });
  }
  const clearFilters = () => {
    setSearch('');
    setKind('all');
    setPage(1);
    setExpanded(null);
  };
  const addModel = async () => {
    if (mutationDisabled || !manualId.trim()) return;
    setAdding(true);
    try {
      if (await onAdd(manualId.trim(), manualKind === 'auto' ? undefined : manualKind)) {
        setManualId('');
        setManualKind('auto');
        setManualOpen(false);
      }
    } finally {
      setAdding(false);
    }
  };
  const removeModel = async (model: Model) => {
    if (mutationDisabled) return;
    setRemoving(identityOf(model));
    try {
      if (await onRemove(model)) setExpanded(null);
    } finally {
      setRemoving(null);
    }
  };

  return (
    <section className="provider-catalog settings-section" aria-labelledby={`${id}-title`} aria-busy={refreshing || undefined}>
      <header className="provider-catalog-header">
        <div className="provider-catalog-intro">
          <h3 id={`${id}-title`} className="settings-section-title">{t('provider.catalog.title')}</h3>
          <p>{t(editMode ? 'provider.catalog.savedHint' : 'provider.catalog.previewHint')}</p>
        </div>
        <div className="provider-catalog-actions">
          <Button variant="secondary" onClick={onRefresh} loading={refreshing} disabled={disabled || refreshing}>
            {t(refreshing ? 'provider.catalog.refreshing' : editMode ? 'provider.catalog.refresh' : 'provider.catalog.fetchPreview')}
          </Button>
          {editMode && (
            <Button variant="secondary" aria-expanded={manualOpen} aria-controls={`${id}-manual`}
              disabled={mutationDisabled} onClick={() => setManualOpen(!manualOpen)}>
              {t('provider.catalog.manualAdd')}
            </Button>
          )}
        </div>
      </header>

      {editMode && manualOpen && (
        <div id={`${id}-manual`} className="provider-catalog-manual" role="group" aria-label={t('provider.catalog.manualAdd')}>
          <Input id={`${id}-manual-id`} label={t('provider.catalog.modelId')} value={manualId}
            onChange={(event) => setManualId(event.target.value)} disabled={mutationDisabled} />
          <Select id={`${id}-manual-kind`} label={t('provider.catalog.manualKind')} value={manualKind}
            onChange={setManualKind} disabled={mutationDisabled}
            options={[{ value: 'auto', label: t('provider.catalog.autoKind') }, ...MODEL_KINDS.map((value) => ({ value, label: kindLabel(value) }))]} />
          <div className="provider-catalog-actions">
            <Button variant="secondary" onClick={() => void addModel()} loading={adding || mutationBusy === 'add'}
              disabled={mutationDisabled || !manualId.trim()}>{t('provider.catalog.add')}</Button>
            <Button variant="ghost" onClick={() => setManualOpen(false)} disabled={mutationDisabled}>{t('provider.catalog.cancel')}</Button>
          </div>
        </div>
      )}

      {blockedReason && <p className="provider-catalog-notice" role="status">{blockedReason}</p>}
      {error && <p className="provider-catalog-error" role="alert">{error}</p>}

      <div className="provider-catalog-filters">
        <Input id={`${id}-search`} type="search" label={t('provider.catalog.search')} value={search}
          onChange={(event) => { setSearch(event.target.value); setPage(1); setExpanded(null); }} disabled={disabled} />
        <Select id={`${id}-kind`} label={t('provider.catalog.type')} value={kind} options={kindOptions}
          onChange={(value) => { setKind(value); setPage(1); setExpanded(null); }} disabled={disabled} />
        {(search !== '' || kind !== 'all') && <Button variant="ghost" onClick={clearFilters} disabled={disabled}>{t('provider.catalog.clearFilters')}</Button>}
      </div>

      <p className="provider-catalog-count" role="status">
        {t('provider.catalog.results', { count: number(filtered.length), total: number(models.length) })}
      </p>
      {visibleModels.length === 0 ? (
        <div className="provider-catalog-empty">
          <h4>{t(models.length === 0 && search === '' && kind === 'all' ? 'provider.catalog.emptyTitle' : 'provider.catalog.noResults')}</h4>
          <p>{t(models.length === 0 && search === '' && kind === 'all'
            ? editMode ? 'provider.catalog.emptySaved' : 'provider.catalog.emptyPreview'
            : 'provider.catalog.noResultsHint')}</p>
        </div>
      ) : (
        <ul className="provider-catalog-list">
          {visibleModels.map((model, index) => {
            const identity = identityOf(model);
            const open = expanded === identity;
            const detailsId = `${id}-details-${start + index}`;
            const name = model.model_name || model.model_id;
            const abilities = CAPABILITIES.filter((capability) => capabilityOf(model, capability) === 'supported');
            return (
              <li key={identity} className="provider-catalog-item">
                <button type="button" className="provider-catalog-row" aria-expanded={open} aria-controls={detailsId}
                  disabled={disabled} onClick={() => setExpanded(open ? null : identity)}>
                  <span className="provider-catalog-identity">
                    <span className="provider-catalog-name">{name}</span>{' '}
                    {name !== model.model_id && <span className="provider-catalog-id">{model.model_id}{' '}</span>}
                  </span>
                  {' '}
                  <span className="provider-catalog-summary">
                    <span className="provider-catalog-kind">{kindLabel(kindOf(model))}{' '}</span>
                    {model.kind_source === 'default' && <span className="provider-catalog-pending">{t('provider.catalog.pending')}{' '}</span>}
                    {abilities.slice(0, 2).map((ability) => <span key={ability}>{t(`provider.catalog.capability.${ability}`)}{' '}</span>)}
                    {!!model.context_window && model.context_window > 0 && <span>{t('provider.catalog.contextCompact', { count: number(model.context_window) })}</span>}
                  </span>
                  <svg className="provider-catalog-chevron" data-open={open} width="16" height="16" viewBox="0 0 16 16" fill="none" aria-hidden="true">
                    <path d="m6 4 4 4-4 4" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
                  </svg>
                </button>
                {open && (
                  <div id={detailsId} className="provider-catalog-details">
                    <dl>
                      <div><dt>{t('provider.catalog.modelId')}</dt><dd className="provider-catalog-full-id">{model.model_id}</dd></div>
                      <div><dt>{t('provider.catalog.source')}</dt><dd>{t(`provider.catalog.source.${model.source ?? 'unknown'}`)}</dd></div>
                      <div><dt>{t('provider.catalog.type')}</dt><dd>{kindLabel(kindOf(model))}</dd></div>
                      <div><dt>{t('provider.catalog.classificationSource')}</dt><dd>{t(`provider.catalog.classification.${model.kind_source ?? 'unknown'}`)}</dd></div>
                      {CAPABILITIES.map((capability) => (
                        <div key={capability}><dt>{t(`provider.catalog.capability.${capability}`)}</dt><dd>{t(`provider.catalog.state.${capabilityOf(model, capability)}`)}</dd></div>
                      ))}
                      <div><dt>{t('provider.catalog.context')}</dt><dd>{model.context_window && model.context_window > 0
                        ? t('provider.catalog.tokens', { count: number(model.context_window) }) : t('provider.catalog.unknown')}</dd></div>
                      <div><dt>{t('provider.catalog.output')}</dt><dd>{model.max_output_tokens && model.max_output_tokens > 0
                        ? t('provider.catalog.tokens', { count: number(model.max_output_tokens) }) : t('provider.catalog.unknown')}</dd></div>
                    </dl>
                    {editMode && model.source === 'manual' && (
                      <Button variant="ghost" className="provider-catalog-remove" aria-label={t('provider.catalog.removeNamed', { id: model.model_id })}
                        disabled={mutationDisabled} loading={removing === identity || mutationBusy === model.model_id}
                        onClick={() => void removeModel(model)}>{t('provider.catalog.remove')}</Button>
                    )}
                  </div>
                )}
              </li>
            );
          })}
        </ul>
      )}
      {filtered.length > PAGE_SIZE && (
        <nav className="provider-catalog-pagination" aria-label={t('provider.catalog.pagination')}>
          <Button variant="ghost" disabled={disabled || currentPage === 1}
            onClick={() => { setPage(currentPage - 1); setExpanded(null); }}>{t('provider.catalog.previous')}</Button>
          <span>{t('provider.catalog.page', { page: number(currentPage), total: number(pageCount) })}</span>
          <Button variant="ghost" disabled={disabled || currentPage === pageCount}
            onClick={() => { setPage(currentPage + 1); setExpanded(null); }}>{t('provider.catalog.next')}</Button>
        </nav>
      )}
    </section>
  );
}
