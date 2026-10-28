import { useEffect, useRef, useState } from 'react';
import { useProviderStore } from '../stores/providerStore';
import ProviderList from '../components/ProviderList';
import ProviderForm from '../components/ProviderForm';
import type { Provider, ProviderConfig, Model } from '../stores/providerStore';
import './ProviderManagement.css';

const EMPTY_MODELS: Model[] = [];

export default function ProviderManagement() {
  const {
    providers,
    models,
    modelsProviderId,
    modelsLoading,
    loading,
    error,
    fetchProviders,
    createProvider,
    updateProvider,
    toggleProvider,
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
  const [migratingProviderId, setMigratingProviderId] = useState<string | null>(null);
  const selectionGeneration = useRef(0);

  useEffect(() => {
    fetchProviders();
  }, []);

  useEffect(() => {
    if (error) {
      console.error('Provider error:', error);
    }
  }, [error]);

  // 当选中 Provider 时，获取模型列表
  useEffect(() => {
    if (selectedProvider) {
      void fetchModels(selectedProvider.id);
    } else {
      clearModels();
    }
    return clearModels;
  }, [selectedProvider?.id, selectedProvider?.config_revision, fetchModels, clearModels]);

  const handleSelectProvider = (provider: Provider) => {
    if (hasUnsavedChanges) {
      if (confirm('You have unsaved changes. Discard them?')) {
        selectionGeneration.current += 1;
        setSelectedProvider(provider);
        setIsCreatingProvider(false);
        setHasUnsavedChanges(false);
        clearError();
      }
    } else {
      selectionGeneration.current += 1;
      setSelectedProvider(provider);
      setIsCreatingProvider(false);
      clearError();
    }
  };

  const handleNewProvider = () => {
    if (hasUnsavedChanges) {
      if (confirm('You have unsaved changes. Discard them?')) {
        selectionGeneration.current += 1;
        setSelectedProvider(null);
        setIsCreatingProvider(true);
        setHasUnsavedChanges(false);
        clearError();
      }
    } else {
      selectionGeneration.current += 1;
      setSelectedProvider(null);
      setIsCreatingProvider(true);
      clearError();
    }
  };

  const handleSave = async (config: ProviderConfig, models: Model[]) => {
    const generation = selectionGeneration.current;
    const savedId = selectedProvider?.id;
    let success = false;
    
    if (selectedProvider) {
      // 编辑模式：更新
      success = await updateProvider(selectedProvider.id, config);
    } else {
      // 新建模式：创建
      success = await createProvider(config, models);
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
    
    return success;
  };

  const handleDelete = async () => {
    if (!selectedProvider) return false;
    
    if (confirm(`Are you sure you want to delete "${selectedProvider.name}"?`)) {
      const success = await deleteProvider(selectedProvider.id);
      if (success) {
        setSelectedProvider(null);
        setIsCreatingProvider(false);
        setHasUnsavedChanges(false);
      }
      return success;
    }
    return false;
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
      return success;
    } finally {
      setTogglingProviderId(null);
    }
  };

  const handleDeleteProvider = async (provider: Provider) => {
    const success = await deleteProvider(provider.id);
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
      await migrateCredential(provider.id);
    } finally {
      setMigratingProviderId(null);
    }
  };

  return (
    <div className="provider-management">
      <header className="page-header">
        <div>
          <p className="page-kicker">Connections</p>
          <h1>Provider Management</h1>
          <p className="page-description">Manage the AI services available to your workspace.</p>
        </div>
      </header>

      {error && (
        <div className="alert alert-error" role="alert">
          <span>{error}</span>
          <button className="btn-close" onClick={clearError} aria-label="Dismiss error">
            ×
          </button>
        </div>
      )}

      <div className="credential-notices">
      {providers.filter(provider => provider.credential_status === 'migration_required').map(provider => (
        <section className="alert credential-notice" key={provider.id} aria-label={`Credential migration for ${provider.name}`}>
          <p>{provider.name}: credential migration is pending. Unlock your OS credential store, then retry. Requests remain unavailable until migration succeeds.</p>
          <button className="btn-secondary" disabled={loading || migratingProviderId !== null}
            onClick={() => void handleMigrateCredential(provider)}>
            {migratingProviderId === provider.id ? 'Migrating credential...' : `Retry credential migration for ${provider.name}`}
          </button>
        </section>
      ))}
      {providers.filter(provider => provider.credential_status === 'missing').map(provider => (
        <section className="alert credential-notice" key={provider.id} aria-label={`Missing credential for ${provider.name}`}>
          <p>{provider.name}: no credential is saved. Select this provider and save a new API key.</p>
          <button className="btn-secondary" onClick={() => handleSelectProvider(provider)}>Configure credential for {provider.name}</button>
        </section>
      ))}
      </div>
      {migratingProviderId && <p role="status">Waiting for the OS credential store...</p>}

      <div className="provider-content">
        <ProviderList
          providers={providers}
          selectedProvider={selectedProvider}
          onSelect={handleSelectProvider}
          onNewProvider={handleNewProvider}
          onToggleEnabled={handleToggleEnabled}
          togglingProviderId={togglingProviderId}
          onDelete={handleDeleteProvider}
          hasUnsavedChanges={hasUnsavedChanges}
        />

        <div className="provider-main">
          {modelsLoading && <p role="status">Loading saved models...</p>}
          {providers.length === 0 && !isCreatingProvider ? (
            <div className="provider-empty-state">
              <h2>No Providers Yet</h2>
              <p>Create your first provider to get started with AI model integration</p>
              <button onClick={handleNewProvider} className="btn-primary">
                + Create Provider
              </button>
            </div>
          ) : selectedProvider === null && !isCreatingProvider ? (
            <div className="provider-selection-state">
              <span className="selection-state-mark" aria-hidden="true">←</span>
              <h2>Select a provider</h2>
              <p>Choose a provider from the list to view or edit its configuration.</p>
              <button onClick={handleNewProvider} className="btn-primary">
                + New Provider
              </button>
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
        </div>
      </div>
    </div>
  );
}
