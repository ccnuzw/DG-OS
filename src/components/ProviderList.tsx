import { Provider } from '../stores/providerStore';

interface ProviderListProps {
  providers: Provider[];
  selectedProvider: Provider | null;
  onSelect: (provider: Provider) => void;
  onEdit: (provider: Provider) => void;
  onDelete: (provider: Provider) => void;
  loading?: boolean;
}

export default function ProviderList({
  providers,
  selectedProvider,
  onSelect,
  onEdit,
  onDelete,
  loading = false,
}: ProviderListProps) {
  if (loading) {
    return <div className="provider-list-loading">Loading providers...</div>;
  }

  if (providers.length === 0) {
    return (
      <div className="provider-list-empty">
        <p>No providers configured yet.</p>
        <p>Click "Add Provider" to get started.</p>
      </div>
    );
  }

  return (
    <div className="provider-list">
      {providers.map((provider) => (
        <div
          key={provider.id}
          className={`provider-item ${
            selectedProvider?.id === provider.id ? 'selected' : ''
          } ${!provider.enabled ? 'disabled' : ''}`}
          onClick={() => onSelect(provider)}
        >
          <div className="provider-item-header">
            <div className="provider-item-title">
              <h3>{provider.name}</h3>
              {!provider.enabled && (
                <span className="badge badge-disabled">Disabled</span>
              )}
            </div>
            <div className="provider-item-actions">
              <button
                onClick={(e) => {
                  e.stopPropagation();
                  onEdit(provider);
                }}
                className="btn-icon"
                title="Edit"
              >
                ✎
              </button>
              <button
                onClick={(e) => {
                  e.stopPropagation();
                  if (
                    confirm(
                      `Are you sure you want to delete "${provider.name}"?`
                    )
                  ) {
                    onDelete(provider);
                  }
                }}
                className="btn-icon btn-danger"
                title="Delete"
              >
                🗑
              </button>
            </div>
          </div>

          <div className="provider-item-details">
            <div className="provider-detail">
              <span className="label">Type:</span>
              <span className="value">{provider.provider_type}</span>
            </div>
            <div className="provider-detail">
              <span className="label">Base URL:</span>
              <span className="value">{provider.base_url}</span>
            </div>
            <div className="provider-detail">
              <span className="label">Created:</span>
              <span className="value">
                {new Date(provider.created_at).toLocaleDateString()}
              </span>
            </div>
          </div>
        </div>
      ))}
    </div>
  );
}
