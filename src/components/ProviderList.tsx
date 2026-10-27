import type { Provider } from '../stores/providerStore';

interface ProviderListProps {
  providers: Provider[];
  selectedProvider: Provider | null;
  onSelect: (provider: Provider) => void;
  onNewProvider: () => void;
  hasUnsavedChanges: boolean;
}

export default function ProviderList({
  providers,
  selectedProvider,
  onSelect,
  onNewProvider,
  hasUnsavedChanges,
}: ProviderListProps) {
  return (
    <aside className="provider-list">
      <div className="list-header">
        <h2>Providers</h2>
        <span className="provider-count">{providers.length}</span>
      </div>

      <div className="list-items">
        {providers.map((provider) => (
          <div
            key={provider.id}
            className={`provider-item ${
              selectedProvider?.id === provider.id ? 'selected' : ''
            }`}
            onClick={() => onSelect(provider)}
          >
            <div className="provider-item-header">
              <h3>{provider.name}</h3>
              {selectedProvider?.id === provider.id && hasUnsavedChanges && (
                <span className="unsaved-indicator" title="Unsaved changes">
                  ●
                </span>
              )}
            </div>
            <div className="provider-item-meta">
              <span className={`status-badge ${provider.enabled ? 'enabled' : 'disabled'}`}>
                {provider.enabled ? '✓ Enabled' : '✗ Disabled'}
              </span>
            </div>
          </div>
        ))}
      </div>

      <button onClick={onNewProvider} className="btn-new-provider">
        + New Provider
      </button>
    </aside>
  );
}
