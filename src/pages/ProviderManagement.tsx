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
    deleteProvider,
    fetchModels,
    clearError,
  } = useProviderStore();

  const [selectedProvider, setSelectedProvider] = useState<Provider | null>(null);
  const [hasUnsavedChanges, setHasUnsavedChanges] = useState(false);
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
        setHasUnsavedChanges(false);
        clearError();
      }
    } else {
      setSelectedProvider(provider);
      clearError();
    }
  };

  const handleNewProvider = () => {
    if (hasUnsavedChanges) {
      if (confirm('You have unsaved changes. Discard them?')) {
        setSelectedProvider(null);
        setHasUnsavedChanges(false);
        clearError();
      }
    } else {
      setSelectedProvider(null);
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
      
      // 如果是新建，保持在新建模式等待用户选择
      // 或者自动选中第一个（如果之前是空的）
      if (!selectedProvider && providers.length === 0) {
        // 列表会自动更新，用户可以点击查看
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
        setHasUnsavedChanges(false);
      }
      return success;
    }
    return false;
  };

  return (
    <div className="provider-management">
      <header className="page-header">
        <h1>Provider Management</h1>
      </header>

      <div className="provider-content">
        <ProviderList
          providers={providers}
          selectedProvider={selectedProvider}
          onSelect={handleSelectProvider}
          onNewProvider={handleNewProvider}
          hasUnsavedChanges={hasUnsavedChanges}
        />

        <div className="provider-main">
          {providers.length === 0 && selectedProvider === null ? (
            <div className="provider-empty-state">
              <h2>No Providers Yet</h2>
              <p>Create your first provider to get started with AI model integration</p>
              <button onClick={handleNewProvider} className="btn-primary">
                + Create Provider
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
