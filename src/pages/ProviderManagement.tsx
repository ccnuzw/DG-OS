import { useEffect, useState } from 'react';
import { useProviderStore } from '../stores/providerStore';
import ProviderList from '../components/ProviderList';
import ProviderForm from '../components/ProviderForm';
import type { Provider, ProviderConfig, Model } from '../stores/providerStore';
import './ProviderManagement.css';

export default function ProviderManagement() {
  const {
    providers,
    models,
    loading,
    error,
    fetchProviders,
    createProvider,
    updateProvider,
    toggleProvider,
    deleteProvider,
    fetchModels,
    clearError,
  } = useProviderStore();

  const [selectedProvider, setSelectedProvider] = useState<Provider | null>(null);
  const [isCreatingProvider, setIsCreatingProvider] = useState(false);
  const [hasUnsavedChanges, setHasUnsavedChanges] = useState(false);
  const [togglingProviderId, setTogglingProviderId] = useState<string | null>(null);
  const currentUserId = 'default-user';

  useEffect(() => {
    fetchProviders(currentUserId);
  }, []);

  useEffect(() => {
    if (error) {
      console.error('Provider error:', error);
    }
  }, [error]);

  // 当选中 Provider 时，获取模型列表
  useEffect(() => {
    if (selectedProvider) {
      fetchModels(selectedProvider.id);
    }
  }, [selectedProvider]);

  const handleSelectProvider = (provider: Provider) => {
    if (hasUnsavedChanges) {
      if (confirm('You have unsaved changes. Discard them?')) {
        setSelectedProvider(provider);
        setIsCreatingProvider(false);
        setHasUnsavedChanges(false);
        clearError();
      }
    } else {
      setSelectedProvider(provider);
      setIsCreatingProvider(false);
      clearError();
    }
  };

  const handleNewProvider = () => {
    if (hasUnsavedChanges) {
      if (confirm('You have unsaved changes. Discard them?')) {
        setSelectedProvider(null);
        setIsCreatingProvider(true);
        setHasUnsavedChanges(false);
        clearError();
      }
    } else {
      setSelectedProvider(null);
      setIsCreatingProvider(true);
      clearError();
    }
  };

  const handleSave = async (config: ProviderConfig, models: Model[]) => {
    let success = false;
    
    if (selectedProvider) {
      // 编辑模式：更新
      success = await updateProvider(selectedProvider.id, config);
    } else {
      // 新建模式：创建
      success = await createProvider(config, currentUserId, models);
    }
    
    if (success) {
      setHasUnsavedChanges(false);
      // 刷新列表
      await fetchProviders(currentUserId);

      // 创建完成后退出新建模式，避免保存后继续显示空白表单。
      if (!selectedProvider) {
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
      if (success && selectedProvider?.id === provider.id) {
        setSelectedProvider((current) => current ? { ...current, enabled } : current);
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
              provider={selectedProvider}
              initialModels={models}
              onSave={handleSave}
              onDelete={selectedProvider ? handleDelete : undefined}
              onChange={() => setHasUnsavedChanges(true)}
              loading={loading}
            />
          )}
        </div>
      </div>
    </div>
  );
}
