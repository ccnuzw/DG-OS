import { useState, type ChangeEvent } from 'react';
import type { Provider } from '../stores/providerStore';

interface ProviderListProps {
  providers: Provider[];
  selectedProvider: Provider | null;
  onSelect: (provider: Provider) => void;
  onNewProvider: () => void;
  onToggleEnabled: (provider: Provider, enabled: boolean) => Promise<boolean>;
  togglingProviderId: string | null;
  onDelete: (provider: Provider) => void;
  hasUnsavedChanges: boolean;
}

export default function ProviderList({
  providers,
  selectedProvider,
  onSelect,
  onNewProvider,
  onToggleEnabled,
  togglingProviderId,
  onDelete,
  hasUnsavedChanges,
}: ProviderListProps) {
  const [deleteConfirm, setDeleteConfirm] = useState<string | null>(null);

  const handleToggle = (e: ChangeEvent<HTMLInputElement>, provider: Provider) => {
    e.stopPropagation();
    void onToggleEnabled(provider, e.target.checked);
  };

  const handleDeleteClick = (e: React.MouseEvent, provider: Provider) => {
    e.stopPropagation();
    setDeleteConfirm(provider.id);
  };

  const handleConfirmDelete = (e: React.MouseEvent, provider: Provider) => {
    e.stopPropagation();
    onDelete(provider);
    setDeleteConfirm(null);
  };

  const handleCancelDelete = (e: React.MouseEvent) => {
    e.stopPropagation();
    setDeleteConfirm(null);
  };

  return (
    <aside className="provider-list">
      <div className="list-header">
        <div>
          <p className="list-eyebrow">Configured services</p>
          <h2>Providers</h2>
        </div>
        <span className="provider-count" aria-label={`${providers.length} providers`}>
          {providers.length}
        </span>
      </div>

      <div className="list-items">
        {providers.length === 0 && (
          <div className="list-empty">
            <span className="list-empty-mark" aria-hidden="true">+</span>
            <strong>No providers yet</strong>
            <span>Add an AI service to get started.</span>
          </div>
        )}
        {providers.map((provider) => (
          <div
            key={provider.id}
            className={`provider-item ${selectedProvider?.id === provider.id ? 'selected' : ''} ${
              provider.enabled ? '' : 'disabled'
            }`}
            onClick={() => onSelect(provider)}
          >
            <div className="provider-item-header">
              <div className="provider-item-title">
                <h3>{provider.name}</h3>
                {selectedProvider?.id === provider.id && hasUnsavedChanges && (
                  <span className="unsaved-indicator" title="Unsaved changes">
                    ●
                  </span>
                )}
              </div>
              <button
                className="btn-delete-provider"
                onClick={(e) => handleDeleteClick(e, provider)}
                title={`Delete ${provider.name}`}
                aria-label={`Delete ${provider.name}`}
              >
                <svg viewBox="0 0 20 20" aria-hidden="true">
                  <path d="M4 6h12M8 6V4h4v2m-6 0 .7 10h6.6L14 6M8.5 9v4m3-4v4" />
                </svg>
              </button>
            </div>

            <div className="provider-item-meta">
              <span className="provider-type">
                {provider.provider_type === 'openai_compatible'
                  ? 'OpenAI compatible'
                  : provider.provider_type.replace(/[_-]/g, ' ')}
              </span>
              <span className={`status-badge ${provider.enabled ? 'enabled' : 'disabled'}`}>
                {provider.enabled ? 'Enabled' : 'Disabled'}
              </span>
            </div>

            <div className="provider-item-footer">
              <div className="toggle-copy">
                <span className="toggle-label">{provider.enabled ? 'Available' : 'Paused'}</span>
                <span className="toggle-hint">{!provider.enabled ? 'Excluded from requests'
                  : provider.credential_status === 'migration_required' ? 'Credential migration pending'
                  : provider.credential_status === 'missing' ? 'Add a credential'
                  : 'Configured for requests'}</span>
              </div>
              <label
                className={`toggle-switch ${togglingProviderId === provider.id ? 'is-pending' : ''}`}
                onClick={(e) => e.stopPropagation()}
                title={provider.enabled ? 'Disable provider' : 'Enable provider'}
              >
                <input
                  type="checkbox"
                  checked={provider.enabled}
                  disabled={togglingProviderId !== null}
                  onClick={(e) => e.stopPropagation()}
                  onChange={(e) => handleToggle(e, provider)}
                  aria-label={`${provider.enabled ? 'Disable' : 'Enable'} ${provider.name}`}
                />
                <span className="toggle-slider"></span>
              </label>
            </div>
          </div>
        ))}
      </div>

      <button onClick={onNewProvider} className="btn-new-provider">
        + New Provider
      </button>

      {/* Delete Confirmation Modal */}
      {deleteConfirm && (
        <div className="modal-overlay" onClick={handleCancelDelete}>
          <div className="modal-content" onClick={(e) => e.stopPropagation()}>
            <h3>Delete Provider?</h3>
            <p>
              Are you sure you want to delete this provider? This action cannot be undone.
            </p>
            <div className="modal-actions">
              <button className="btn-secondary" onClick={handleCancelDelete}>
                Cancel
              </button>
              <button
                className="btn-danger"
                onClick={(e) => {
                  const provider = providers.find(p => p.id === deleteConfirm);
                  if (provider) handleConfirmDelete(e, provider);
                }}
              >
                Delete
              </button>
            </div>
          </div>
        </div>
      )}
    </aside>
  );
}
