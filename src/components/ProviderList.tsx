import { useLocale as usePageLocale } from '../i18n';
import { useState } from 'react';
import { Button, Loading, Modal, Stack, Switch, cn } from '@dgos/ui';
import { resolveVendorId } from '../stores/providerStore';
import type { Provider, ProviderVendor } from '../stores/providerStore';
import { useLocale } from '../i18n';

interface ProviderListProps {
  providers: Provider[];
  vendors: ProviderVendor[];
  selectedProvider: Provider | null;
  onSelect: (provider: Provider) => void;
  onNewProvider: () => void;
  onToggleEnabled: (provider: Provider, enabled: boolean) => Promise<boolean>;
  togglingProviderId: string | null;
  onMoveProvider: (provider: Provider, direction: -1 | 1) => void | Promise<boolean>;
  reordering: boolean;
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
  onMoveProvider,
  reordering,
  onDelete,
  hasUnsavedChanges,
}: ProviderListProps) {
  usePageLocale();
  const { t } = useLocale();
  const [deleteConfirm, setDeleteConfirm] = useState<string | null>(null);

  const handleConfirmDelete = async () => {
    const provider = providers.find((p) => p.id === deleteConfirm);
    if (provider) await onDelete(provider);
  };

  return (
    <aside className="provider-list">
      <div className="flex items-center justify-between gap-2">
        <div className="min-w-0">
           <h2 className="m-0 text-sm font-medium text-ink-primary">{t('nav.providers')}</h2>
        </div>
        <span
          className="inline-flex min-w-8 items-center justify-center rounded-full bg-surface-inset px-2 py-1 text-sm font-semibold text-ink-primary"
          aria-label={t('provider.count', { count: providers.length })}
        >
          {providers.length}
        </span>
      </div>

      <Stack gap={3}>
        {providers.length === 0 && (
          <div className="flex flex-col items-center gap-1 rounded-lg border border-dashed border-border-subtle p-6 text-center text-sm text-ink-secondary">
            <span aria-hidden="true" className="text-xl font-semibold text-ink-primary">+</span>
             <strong className="text-ink-primary">{t('provider.emptyTitle')}</strong>
             <span>{t('provider.emptyText')}</span>
          </div>
        )}
        {providers.map((provider, index) => {
          const moveDisabled = reordering || togglingProviderId !== null;
          const selected = selectedProvider?.id === provider.id;
          return (
            <div
              key={provider.id}
              className={cn(
                'provider-list-item',
                selected && 'is-selected',
                !provider.enabled && 'text-ink-secondary',
              )}
              role="group" aria-label={provider.name}
            >
              <div className="flex items-center justify-between gap-2">
                <div className="flex min-w-0 items-center gap-2">
                  <h3 className="m-0 min-w-0 wrap-anywhere text-sm font-medium"><button type="button" className="provider-select" onClick={() => onSelect(provider)}>{provider.name}</button></h3>
                  {selected && hasUnsavedChanges && (
                    <span role="img" aria-label={t('detail.unsaved')} title={t('detail.unsaved')} className="text-status-error-text">
                      ●
                    </span>
                  )}
                </div>
                <div className="flex shrink-0 items-center">
                <Button
                  variant="ghost"
                  size="sm"
                  iconOnly
                  title={t('provider.moveUp', { name: provider.name })}
                  aria-label={t('provider.moveUp', { name: provider.name })}
                  disabled={index === 0 || moveDisabled}
                  onClick={(e) => {
                    e.stopPropagation();
                    void onMoveProvider(provider, -1);
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
                    <path d="m5 12 5-5 5 5" />
                  </svg>
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  iconOnly
                  title={t('provider.moveDown', { name: provider.name })}
                  aria-label={t('provider.moveDown', { name: provider.name })}
                  disabled={index === providers.length - 1 || moveDisabled}
                  onClick={(e) => {
                    e.stopPropagation();
                    void onMoveProvider(provider, 1);
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
                    <path d="m5 8 5 5 5-5" />
                  </svg>
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  iconOnly
                  title={t('provider.deleteName', { name: provider.name })}
                  aria-label={t('provider.deleteName', { name: provider.name })}
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
                </div>
                <span
                  className={cn(
                    'shrink-0 rounded-full px-2 py-1 text-xs font-semibold',
                    provider.enabled ? 'bg-status-success-surface text-status-success-text' : 'bg-surface-inset text-ink-primary',
                  )}
                >
                   {provider.enabled ? t('common.enabled') : t('common.disabled')}
                </span>
              </div>

              <div className="flex items-center justify-between gap-3">
                <div className="flex min-w-0 flex-1 flex-col">
                  <span className="wrap-anywhere text-xs text-ink-secondary">
                    {!provider.enabled
                      ? t('provider.excluded')
                      : provider.credential_status === 'migration_required'
                        ? t('provider.migrationPending')
                        : provider.credential_status === 'missing'
                          ? t('provider.addCredential')
                          : t('provider.configured')}
                  </span>
                </div>
                <div className="flex shrink-0 items-center gap-2" onClick={(e) => e.stopPropagation()}>
                  {togglingProviderId === provider.id && <Loading size="sm" aria-label={t('provider.updating', { name: provider.name })} />}
                  <Switch
                    aria-label={`${provider.enabled ? t('module.disable') : t('module.enableButton')} ${provider.name}`}
                    checked={provider.enabled}
                    disabled={togglingProviderId !== null || reordering}
                    onChange={(enabled) => void onToggleEnabled(provider, enabled)}
                  />
                </div>
              </div>
            </div>
          );
        })}
      </Stack>

      <Button size="sm" variant="secondary" onClick={onNewProvider} className="w-full">
         + {t('provider.new')}
      </Button>

      <Modal
        open={deleteConfirm !== null}
        onOpenChange={(open) => {
          if (!open) setDeleteConfirm(null);
        }}
        title={t('detail.deleteProvider')}
        description={t('provider.deleteConfirm')}
        confirmText={t('providerForm.delete')}
        cancelText={t('module.cancel')}
        confirmVariant="danger"
        onConfirm={handleConfirmDelete}
      />
    </aside>
  );
}
