import { useLocale as usePageLocale } from '../i18n';
import { useEffect, useRef, useState } from 'react';
import { useProviderStore } from '../stores/providerStore';
import ProviderList from '../components/ProviderList';
import ProviderForm from '../components/ProviderForm';
import type { Provider, ProviderConfig, Model } from '../stores/providerStore';
import { Button, Modal, Stack } from '@dgos/ui';
import { usePageError } from '../pageError';
import { useLocale } from '../i18n';

const EMPTY_MODELS: Model[] = [];

export default function ProviderManagement() {
  usePageLocale();
  const { t } = useLocale();
  const {
    providers,
    models,
    vendors,
    modelsProviderId,
    modelsLoading,
    loading,
    error,
    fetchProviders,
    fetchVendors,
    createProvider,
    updateProvider,
    toggleProvider,
    reorderProviders,
    migrateCredential,
    deleteProvider,
    fetchModels,
    clearModels,
    clearError,
  } = useProviderStore();

  const [selectedProvider, setSelectedProvider] = useState<Provider | null>(null);
  const [isCreatingProvider, setIsCreatingProvider] = useState(false);
  const [hasUnsavedChanges, setHasUnsavedChanges] = useState(false);
  const [togglingProviderId, setTogglingProviderId] = useState<string | null>(null);
  const [reordering, setReordering] = useState(false);
  const [migratingProviderId, setMigratingProviderId] = useState<string | null>(null);
  const selectionGeneration = useRef(0);
  const reportError = usePageError('ProviderManagement');
  // The store keeps the raw failure of the last create/update/toggle/migrate/delete next to its message string.
  const reportFailure = () => reportError(useProviderStore.getState().lastFailure);

  useEffect(() => {
    void fetchVendors();
    void fetchProviders().then(() => {
      if (useProviderStore.getState().error !== null) reportFailure();
    });
  }, []);

  useEffect(() => {
    if (error) {
      console.error('Provider error:', error);
    }
  }, [error]);

  // 当选中 Provider 时，获取模型列表
  useEffect(() => {
    if (selectedProvider) {
      const id = selectedProvider.id;
      const pending = fetchModels(id);
      // fetchModels bumps `modelRequest` before its first await; a later selection or unmount bumps it again,
      // so a superseded call never reports.
      const request = useProviderStore.getState().modelRequest;
      void pending.then(() => {
        const state = useProviderStore.getState();
        if (state.modelRequest === request && state.modelsProviderId === id && state.error !== null) reportFailure();
      });
    } else {
      clearModels();
    }
    return clearModels;
  }, [selectedProvider?.id, selectedProvider?.config_revision, fetchModels, clearModels]);

  // At most one confirm dialog at a time. `settle` resolves the pending promise when the page unmounts with the dialog open.
  const activeDialog = useRef<{ close: () => void; settle: () => void } | null>(null);
  useEffect(() => () => {
    activeDialog.current?.close();
    activeDialog.current?.settle();
    activeDialog.current = null;
  }, []);

  // Resolves true once the user confirmed and `action` finished (a boolean `false` from `action` resolves false),
  // false when cancelled, dismissed, ignored because another dialog is open, or the page unmounted.
  const confirmAction = (
    options: { title: string; content: string; confirmText: string; confirmVariant?: 'danger' },
    action: () => void | boolean | Promise<void | boolean>,
  ): Promise<boolean> => {
    if (activeDialog.current) return Promise.resolve(false);
    return new Promise<boolean>(resolve => {
      const finish = (value: boolean) => {
        activeDialog.current = null;
        resolve(value);
      };
      const handle = Modal.confirm({
        ...options,
        cancelText: t('module.cancel'),
        onConfirm: async () => finish((await action()) !== false),
        onCancel: () => finish(false),
      });
      activeDialog.current = { close: handle.close, settle: () => resolve(false) };
    });
  };

  const discardThen = (proceed: () => void) => {
    if (!hasUnsavedChanges) {
      proceed();
      return;
    }
    void confirmAction(
      { title: 'Discard unsaved changes?', content: 'You have unsaved changes. Discard them?', confirmText: 'Discard', confirmVariant: 'danger' },
      () => {
        proceed();
        setHasUnsavedChanges(false);
      },
    );
  };

  const handleSelectProvider = (provider: Provider) => discardThen(() => {
    selectionGeneration.current += 1;
    setSelectedProvider(provider);
    setIsCreatingProvider(false);
    clearError();
  });

  const handleNewProvider = () => discardThen(() => {
    selectionGeneration.current += 1;
    setSelectedProvider(null);
    setIsCreatingProvider(true);
    clearError();
  });

  const handleSave = async (config: ProviderConfig, models: Model[], probeModel?: string) => {
    const generation = selectionGeneration.current;
    const savedId = selectedProvider?.id;
    let success = false;
    
    if (selectedProvider) {
      // 编辑模式：更新
      success = await updateProvider(selectedProvider.id, config);
    } else {
      // 新建模式：创建
      success = await createProvider(config, models, probeModel);
    }
    
    if (success) {
      if (generation !== selectionGeneration.current) return true;
      setHasUnsavedChanges(false);
      if (savedId) {
        const updated = useProviderStore.getState().providers.find(provider => provider.id === savedId);
        if (updated) setSelectedProvider(current => current?.id === savedId ? updated : current);
      } else {
        setIsCreatingProvider(false);
      }
    }
    
    if (!success) reportFailure();
    return success;
  };

  const handleDelete = async () => {
    if (!selectedProvider) return false;
    const target = selectedProvider;
    let deleted = false;
    await confirmAction(
      { title: t('detail.deleteProvider'), content: `Are you sure you want to delete "${target.name}"? This action cannot be undone.`, confirmText: t('providerForm.delete'), confirmVariant: 'danger' },
      async () => {
        const success = await deleteProvider(target.id);
        if (success) {
          setSelectedProvider(current => (current?.id === target.id ? null : current));
          setIsCreatingProvider(false);
          setHasUnsavedChanges(false);
        } else {
          reportFailure();
        }
        deleted = success;
        return success;
      },
    );
    return deleted;
  };

  const handleToggleEnabled = async (provider: Provider, enabled: boolean) => {
    if (togglingProviderId) return false;

    setTogglingProviderId(provider.id);
    try {
      const success = await toggleProvider(provider.id, enabled);
      if (success) {
        const updated = useProviderStore.getState().providers.find(row => row.id === provider.id);
        if (updated) setSelectedProvider(current => current?.id === provider.id ? updated : current);
      }
      if (!success) reportFailure();
      return success;
    } finally {
      setTogglingProviderId(null);
    }
  };

  const handleMoveProvider = async (provider: Provider, direction: -1 | 1) => {
    if (reordering || togglingProviderId) return false;
    const ids = useProviderStore.getState().providers.map(row => row.id);
    const from = ids.indexOf(provider.id);
    const to = from + direction;
    if (from < 0 || to < 0 || to >= ids.length) return false;
    [ids[from], ids[to]] = [ids[to], ids[from]];

    setReordering(true);
    try {
      const success = await reorderProviders(ids);
      if (success) {
        const rows = useProviderStore.getState().providers;
        setSelectedProvider(current => (current ? rows.find(row => row.id === current.id) ?? current : current));
      } else {
        reportFailure();
      }
      return success;
    } finally {
      setReordering(false);
    }
  };

  const handleDeleteProvider = async (provider: Provider) => {
    const success = await deleteProvider(provider.id);
    if (!success) reportFailure();
    if (success && selectedProvider?.id === provider.id) {
      setSelectedProvider(null);
      setIsCreatingProvider(false);
      setHasUnsavedChanges(false);
    }
  };

  const handleMigrateCredential = async (provider: Provider) => {
    if (migratingProviderId) return;
    setMigratingProviderId(provider.id);
    try {
      if (!(await migrateCredential(provider.id))) reportFailure();
    } finally {
      setMigratingProviderId(null);
    }
  };

  return (
    <div className="flex h-full min-h-0 flex-col overflow-y-auto bg-background text-foreground">
      <header className="shrink-0 border-b border-border px-6 py-6">
        <p className="m-0 text-xs font-semibold uppercase text-primary">{t('provider.kicker')}</p>
        <h1 className="m-0 mt-1 text-3xl font-semibold text-foreground">{t('provider.title')}</h1>
        <p className="m-0 mt-2 text-sm text-muted-foreground">{t('provider.subtitle')}</p>
      </header>

      {error && (
        <div
          className="mx-6 mt-4 flex items-start justify-between gap-3 rounded-lg border border-error bg-error-bg p-3 text-error-text"
          role="alert"
        >
          <span className="min-w-0 wrap-anywhere">{error}</span>
           <Button variant="ghost" size="sm" iconOnly onClick={clearError} aria-label={t('provider.dismiss')}>
            ×
          </Button>
        </div>
      )}

      <div className="mx-6 mt-4 flex shrink-0 flex-col gap-3 empty:hidden">
        {providers.filter(provider => provider.credential_status === 'migration_required').map(provider => (
          <section
            className="flex flex-wrap items-center gap-3 rounded-lg border border-warning bg-warning-bg p-3 text-warning-text"
            key={provider.id}
            aria-label={t('provider.migrationAria', { name: provider.name })}
          >
            <p className="m-0 min-w-0 flex-1 basis-80 wrap-anywhere text-sm">
              {provider.name}{t('detail.migration')}</p>
            <Button
              variant="secondary"
              loading={migratingProviderId === provider.id}
              disabled={loading || migratingProviderId !== null}
              onClick={() => void handleMigrateCredential(provider)}
            >
              {migratingProviderId === provider.id ? t('provider.migrating') : t('provider.retryMigration', { name: provider.name })}
            </Button>
          </section>
        ))}
        {providers.filter(provider => provider.credential_status === 'missing').map(provider => (
          <section
            className="flex flex-wrap items-center gap-3 rounded-lg border border-warning bg-warning-bg p-3 text-warning-text"
            key={provider.id}
            aria-label={t('provider.missingAria', { name: provider.name })}
          >
            <p className="m-0 min-w-0 flex-1 basis-80 wrap-anywhere text-sm">
              {provider.name}{t('detail.noCredential')}</p>
            <Button variant="secondary" onClick={() => handleSelectProvider(provider)}>
              {t('detail.configureCredential')}{provider.name}
            </Button>
          </section>
        ))}
      </div>
      {migratingProviderId && <p role="status" className="mx-6 mt-4 mb-0 text-sm text-muted-foreground">{t('detail.waitingVault')}</p>}

      <div className="flex min-h-0 flex-1 flex-wrap content-start items-start">
        <ProviderList
          vendors={vendors}
          providers={providers}
          selectedProvider={selectedProvider}
          onSelect={handleSelectProvider}
          onNewProvider={handleNewProvider}
          onToggleEnabled={handleToggleEnabled}
          togglingProviderId={togglingProviderId}
          onMoveProvider={handleMoveProvider}
          reordering={reordering}
          onDelete={handleDeleteProvider}
          hasUnsavedChanges={hasUnsavedChanges}
        />

        <div className="min-w-0 grow basis-120 p-6">
          <Stack gap={4}>
             {modelsLoading && <p role="status" className="m-0 text-sm text-muted-foreground">{t('provider.loadingModels')}</p>}
            {providers.length === 0 && !isCreatingProvider ? (
              <div className="flex max-w-150 flex-col items-center gap-3 px-6 py-12 text-center">
                <h2 className="m-0 text-xl font-semibold text-foreground">{t('provider.emptyTitle')}</h2>
                <p className="m-0 text-sm text-muted-foreground">{t('provider.emptyText')}</p>
                <Button onClick={handleNewProvider}>+ {t('provider.create')}</Button>
              </div>
            ) : selectedProvider === null && !isCreatingProvider ? (
              <div className="flex max-w-150 flex-col items-center gap-3 px-6 py-12 text-center">
                <span aria-hidden="true" className="text-3xl text-muted-foreground">←</span>
                <h2 className="m-0 text-xl font-semibold text-foreground">{t('provider.selectTitle')}</h2>
                <p className="m-0 text-sm text-muted-foreground">{t('provider.selectText')}</p>
                <Button onClick={handleNewProvider}>+ {t('provider.new')}</Button>
              </div>
            ) : (
              <ProviderForm
                key={selectedProvider?.id ?? 'new'}
                provider={selectedProvider}
                initialModels={selectedProvider && modelsProviderId === selectedProvider.id ? models : EMPTY_MODELS}
                onSave={handleSave}
                onDelete={selectedProvider ? handleDelete : undefined}
                onChange={() => setHasUnsavedChanges(true)}
                loading={loading}
                onModelsRefreshed={async () => {
                  const id = selectedProvider?.id;
                  await fetchProviders();
                  const updated = useProviderStore.getState().providers.find(row => row.id === id);
                  if (updated) setSelectedProvider(current => current?.id === id ? updated : current);
                }}
              />
            )}
          </Stack>
        </div>
      </div>
    </div>
  );
}
