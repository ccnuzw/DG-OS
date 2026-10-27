import { useEffect, useState } from 'react';
import { useProviderStore } from '../stores/providerStore';
import ProviderList from '../components/ProviderList';
import ProviderForm from '../components/ProviderForm';
import type { Provider, ProviderConfig, Model } from '../stores/providerStore';
import './ProviderManagement.css';

export default function ProviderManagement() {
  const {
    providers,
    selectedProvider,
    models,
    loading,
    error,
    fetchProviders,
    createProvider,
    updateProvider,
    deleteProvider,
    fetchModels,
    setSelectedProvider,
    clearError,
  } = useProviderStore();

  const [showForm, setShowForm] = useState(false);
  const [editingProvider, setEditingProvider] = useState<Provider | null>(null);
  
  // TODO: Get actual user ID from auth system
  const currentUserId = 'default-user';

  useEffect(() => {
    fetchProviders(currentUserId);
  }, []);

  useEffect(() => {
    if (selectedProvider) {
      fetchModels(selectedProvider.id);
    }
  }, [selectedProvider]);

  const handleCreate = () => {
    setEditingProvider(null);
    setShowForm(true);
  };

  const handleEdit = (provider: Provider) => {
    setEditingProvider(provider);
    setShowForm(true);
  };

  const handleSubmit = async (config: ProviderConfig, models: Model[]) => {
    let success = false;
    
    if (editingProvider) {
      success = await updateProvider(editingProvider.id, config);
    } else {
      success = await createProvider(config, currentUserId, models);
    }
    
    if (success) {
      setShowForm(false);
      setEditingProvider(null);
    }
    
    return success;
  };

  const handleCancel = () => {
    setShowForm(false);
    setEditingProvider(null);
  };

  const handleDelete = async (provider: Provider) => {
    await deleteProvider(provider.id);
  };


  return (
    <div className="provider-management">
      <header className="page-header">
        <h1>Provider Management</h1>
        <p>Configure AI model providers for your workspace</p>
      </header>

      {error && (
        <div className="alert alert-error">
          <span>{error}</span>
          <button onClick={clearError} className="btn-close">
            ×
          </button>
        </div>
      )}

      <div className="provider-content">
        <div className="provider-sidebar">
          <div className="sidebar-header">
            <h2>Providers ({providers.length})</h2>
            <button onClick={handleCreate} className="btn-primary btn-sm">
              + Add Provider
            </button>
          </div>

          <ProviderList
            providers={providers}
            selectedProvider={selectedProvider}
            onSelect={setSelectedProvider}
            onEdit={handleEdit}
            onDelete={handleDelete}
            loading={loading}
          />
        </div>

        <div className="provider-main">
          {showForm ? (
            <div className="provider-form-container">
              <h2>{editingProvider ? 'Edit Provider' : 'Add New Provider'}</h2>
              <ProviderForm
                initialData={
                  editingProvider
                    ? {
                        id: editingProvider.id,
                        name: editingProvider.name,
                        provider_type: editingProvider.provider_type,
                        base_url: editingProvider.base_url,
                        api_key: '', // Don't pre-fill API key for security
                      }
                    : undefined
                }
                onSubmit={handleSubmit}
                onCancel={handleCancel}
                loading={loading}
              />
            </div>
          ) : selectedProvider ? (
            <div className="provider-details">
              <h2>{selectedProvider.name}</h2>
              
              <div className="detail-section">
                <h3>Configuration</h3>
                <dl className="detail-list">
                  <dt>Provider Type</dt>
                  <dd>{selectedProvider.provider_type}</dd>
                  
                  <dt>Base URL</dt>
                  <dd>{selectedProvider.base_url}</dd>
                  
                  <dt>Status</dt>
                  <dd>
                    <span
                      className={`badge ${
                        selectedProvider.enabled ? 'badge-success' : 'badge-disabled'
                      }`}
                    >
                      {selectedProvider.enabled ? 'Enabled' : 'Disabled'}
                    </span>
                  </dd>
                  
                  <dt>Created</dt>
                  <dd>{new Date(selectedProvider.created_at).toLocaleString()}</dd>
                  
                  <dt>Last Updated</dt>
                  <dd>{new Date(selectedProvider.updated_at).toLocaleString()}</dd>
                </dl>
              </div>

              <div className="detail-section">
                <h3>Available Models ({models.length})</h3>
                {loading ? (
                  <p>Loading models...</p>
                ) : models.length > 0 ? (
                  <div className="models-grid">
                    {models.map((model) => (
                      <div key={model.id} className="model-card">
                        <h4>{model.model_name}</h4>
                        <div className="model-meta">
                          <span className="badge">{model.model_type}</span>
                          {model.supports_streaming && (
                            <span className="badge badge-info">Streaming</span>
                          )}
                          {model.supports_vision && (
                            <span className="badge badge-info">Vision</span>
                          )}
                        </div>
                        <p className="model-context">
                          Context: {model.context_window?.toLocaleString() || 'N/A'} tokens
                        </p>
                      </div>
                    ))}
                  </div>
                ) : (
                  <p>No models available. Check provider configuration.</p>
                )}
              </div>
            </div>
          ) : (
            <div className="provider-empty-state">
              <h2>Select a provider</h2>
              <p>Choose a provider from the list to view details and manage models.</p>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
