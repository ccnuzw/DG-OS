import { useLocale as usePageLocale } from '../i18n';
import { Button, Icon, Loading, Stack, Switch, cn } from '@dgos/ui';
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
  reordering: boolean;
  hasUnsavedChanges: boolean;
}

function endpointHost(address: string): string {
  try { return new URL(address).host; } catch { return address; }
}

export default function ProviderList({
  vendors,
  providers,
  selectedProvider,
  onSelect,
  onNewProvider,
  onToggleEnabled,
  togglingProviderId,
  reordering,
  hasUnsavedChanges,
}: ProviderListProps) {
  usePageLocale();
  const { t } = useLocale();

  return (
    <aside className="provider-list">
      <div className="provider-list-toolbar">
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
      <Button size="sm" variant="secondary" onClick={onNewProvider} className="w-full">
        + {t('provider.new')}
      </Button>

      <Stack gap={2} className="provider-list-cards">
        {providers.length === 0 && (
          <div className="flex flex-col items-center gap-1 rounded-lg border border-dashed border-border-subtle p-6 text-center text-sm text-ink-secondary">
            <span aria-hidden="true" className="text-xl font-semibold text-ink-primary">+</span>
             <strong className="text-ink-primary">{t('provider.emptyTitle')}</strong>
             <span>{t('provider.emptyText')}</span>
          </div>
        )}
        {providers.map((provider) => {
          const selected = selectedProvider?.id === provider.id;
          const savedVendor = typeof provider.config?.vendor_id === 'string' && provider.config.vendor_id !== '';
          const vendorId = savedVendor || vendors.length > 0 ? resolveVendorId(provider, vendors) : null;
          const vendorName = vendorId ? vendors.find((vendor) => vendor.vendor_id === vendorId)?.display_name ?? vendorId : null;
          const status = !provider.enabled
            ? t('provider.excluded')
            : provider.credential_status === 'migration_required'
              ? t('provider.migrationPending')
              : provider.credential_status === 'missing'
                ? t('provider.addCredential')
                : t('provider.configured');
          return (
            <div
              key={provider.id}
              className={cn('provider-list-item', selected && 'is-selected', !provider.enabled && 'is-disabled')}
              role="group"
              aria-label={provider.name}
              onClick={(event) => {
                if (event.target === event.currentTarget || !(event.target instanceof Element) || !event.target.closest('button, input, [role="switch"]')) onSelect(provider);
              }}
            >
              <button
                type="button"
                className="provider-card-select"
                aria-label={provider.name}
                aria-pressed={selected}
                onClick={() => onSelect(provider)}
              />

              <div className="provider-card-content">
                <div className="provider-card-heading">
                  <span className="provider-card-mark" aria-hidden="true">{vendorId === 'openai' ? 'O' : vendorId === 'anthropic' ? 'A' : <Icon name="globe" size="sm" />}</span>
                  <div className="provider-card-identity">
                    <div className="flex min-w-0 items-center gap-2">
                      <h3 className="m-0 min-w-0 wrap-anywhere text-sm font-semibold text-ink-primary">{provider.name}</h3>
                      {selected && hasUnsavedChanges && (
                        <span role="img" aria-label={t('detail.unsaved')} title={t('detail.unsaved')} className="shrink-0 text-status-error-text">●</span>
                      )}
                    </div>
                    <span className="provider-card-endpoint" title={provider.base_url}>{vendorId && vendorId !== 'custom' ? vendorName : endpointHost(provider.base_url)}</span>
                  </div>
                </div>


                <div className="provider-card-state">
                  <span className={cn('provider-card-status', provider.enabled && provider.credential_status === 'stored' && 'is-ready')}>
                    <span className="provider-card-status-dot" aria-hidden="true" />
                    <span>{status}</span>
                  </span>
                  <div className="provider-card-control">
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
            </div>
          );
        })}
      </Stack>

    </aside>
  );
}
