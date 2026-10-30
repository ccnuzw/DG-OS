import { useState } from 'react';
import { Button, Loading, Modal, Stack, Switch, cn } from '@dgos/ui';
import { resolveVendorId } from '../stores/providerStore';
import type { Provider, ProviderVendor } from '../stores/providerStore';

interface ProviderListProps {
  providers: Provider[];
  vendors: ProviderVendor[];
  selectedProvider: Provider | null;
  onSelect: (provider: Provider) => void;
  onNewProvider: () => void;
  onToggleEnabled: (provider: Provider, enabled: boolean) => Promise<boolean>;
  togglingProviderId: string | null;
  onDelete: (provider: Provider) => void | Promise<void>;
  hasUnsavedChanges: boolean;
}

export default function ProviderList({
  vendors,
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

  const handleConfirmDelete = async () => {
    const provider = providers.find((p) => p.id === deleteConfirm);
    if (provider) await onDelete(provider);
  };

  return (
    <aside className="flex min-w-0 max-w-full shrink-0 basis-80 flex-col gap-4 p-4">
      <div className="flex items-center justify-between gap-3">
        <div className="min-w-0">
          <p className="m-0 text-xs font-semibold uppercase text-muted-foreground">Configured services</p>
          <h2 className="m-0 text-lg font-semibold text-foreground">Providers</h2>
        </div>
        <span
          className="inline-flex min-w-8 items-center justify-center rounded-full bg-secondary px-2 py-1 text-sm font-semibold text-foreground"
          aria-label={`${providers.length} providers`}
        >
          {providers.length}
        </span>
      </div>

      <Stack gap={3}>
        {providers.length === 0 && (
          <div className="flex flex-col items-center gap-1 rounded-lg border border-dashed border-border p-6 text-center text-sm text-muted-foreground">
            <span aria-hidden="true" className="text-xl font-semibold text-foreground">+</span>
            <strong className="text-foreground">No providers yet</strong>
            <span>Add an AI service to get started.</span>
          </div>
        )}
        {providers.map((provider) => {
          const selected = selectedProvider?.id === provider.id;
          return (
            <div
              key={provider.id}
              className={cn(
                'flex cursor-pointer flex-col gap-3 rounded-lg border bg-card p-4 text-card-foreground transition-colors hover:bg-muted',
                selected ? 'border-primary' : 'border-border',
                !provider.enabled && 'text-muted-foreground',
              )}
              onClick={() => onSelect(provider)}
            >
              <div className="flex items-center justify-between gap-2">
                <div className="flex min-w-0 items-center gap-2">
                  <h3 className="m-0 min-w-0 wrap-anywhere text-base font-semibold">{provider.name}</h3>
                  {selected && hasUnsavedChanges && (
                    <span role="img" aria-label="Unsaved changes" title="Unsaved changes" className="text-error-text">
                      ●
                    </span>
                  )}
                </div>
                <Button
                  variant="ghost"
                  size="sm"
                  iconOnly
                  title={`Delete ${provider.name}`}
                  aria-label={`Delete ${provider.name}`}
                  onClick={(e) => {
                    e.stopPropagation();
                    setDeleteConfirm(provider.id);
                  }}
                >
                  <svg
                    viewBox="0 0 20 20"
                    aria-hidden="true"
                    focusable="false"
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="1.5"
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    className="size-4"
                  >
                    <path d="M4 6h12M8 6V4h4v2m-6 0 .7 10h6.6L14 6M8.5 9v4m3-4v4" />
                  </svg>
                </Button>
              </div>

              <div className="flex flex-wrap items-center justify-between gap-2">
                <div className="flex min-w-0 flex-col">
                  {(() => {
                    // Before the vendor table has loaded a legacy provider cannot be inferred, so show nothing rather than guess.
                    const saved = typeof provider.config?.vendor_id === 'string' && provider.config.vendor_id !== '';
                    if (!saved && vendors.length === 0) return null;
                    const vendorId = resolveVendorId(provider, vendors);
                    const vendorName = vendors.find((vendor) => vendor.vendor_id === vendorId)?.display_name ?? vendorId;
                    return <span className="min-w-0 wrap-anywhere text-sm font-semibold">{vendorName}</span>;
                  })()}
                  <span className="min-w-0 wrap-anywhere text-xs uppercase text-muted-foreground">
                    {provider.provider_type === 'openai_compatible'
                      ? 'OpenAI compatible'
                      : provider.provider_type.replace(/[_-]/g, ' ')}
                  </span>
                </div>
                <span
                  className={cn(
                    'shrink-0 rounded-full px-2 py-1 text-xs font-semibold',
                    provider.enabled ? 'bg-success-bg text-success-text' : 'bg-secondary text-secondary-foreground',
                  )}
                >
                  {provider.enabled ? 'Enabled' : 'Disabled'}
                </span>
              </div>

              <div className="flex items-center justify-between gap-3">
                <div className="flex min-w-0 flex-1 flex-col">
                  <span className="text-sm font-semibold">{provider.enabled ? 'Available' : 'Paused'}</span>
                  <span className="wrap-anywhere text-xs text-muted-foreground">
                    {!provider.enabled
                      ? 'Excluded from requests'
                      : provider.credential_status === 'migration_required'
                        ? 'Credential migration pending'
                        : provider.credential_status === 'missing'
                          ? 'Add a credential'
                          : 'Configured for requests'}
                  </span>
                </div>
                <div className="flex shrink-0 items-center gap-2" onClick={(e) => e.stopPropagation()}>
                  {togglingProviderId === provider.id && <Loading size="sm" aria-label={`Updating ${provider.name}`} />}
                  <Switch
                    aria-label={`${provider.enabled ? 'Disable' : 'Enable'} ${provider.name}`}
                    checked={provider.enabled}
                    disabled={togglingProviderId !== null}
                    onChange={(enabled) => void onToggleEnabled(provider, enabled)}
                  />
                </div>
              </div>
            </div>
          );
        })}
      </Stack>

      <Button size="lg" onClick={onNewProvider} className="w-full">
        + New Provider
      </Button>

      <Modal
        open={deleteConfirm !== null}
        onOpenChange={(open) => {
          if (!open) setDeleteConfirm(null);
        }}
        title="Delete Provider?"
        description="Are you sure you want to delete this provider? This action cannot be undone."
        confirmText="Delete"
        cancelText="Cancel"
        confirmVariant="danger"
        onConfirm={handleConfirmDelete}
      />
    </aside>
  );
}
