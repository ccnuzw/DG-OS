import { useEffect, useState } from 'react';
import { useDevelopment } from '../../stores/developmentStore';
import { contextGeneration, useContextSelection } from '../../stores/developmentSelection';
import { readSessionConfiguration, type CliProviderSnapshot, type SessionConfigurationView } from '../../stores/developmentCapabilities';
import { hostCode } from '../../stores/hostError';
import { errorText } from './shared';
import { useLocale } from '../../i18n';

export function CapabilitySnapshotView({ snapshot }: { snapshot: CliProviderSnapshot | null | undefined }) {
  const { t } = useLocale();
  if (!snapshot) return <p className="m-0 text-xs text-ink-secondary">{t('capabilities.noSnapshot')}</p>;
  return <section className="dev-operations dev-detail" aria-label={t('capabilities.actual')}>
    <strong className="text-xs">{t('capabilities.actual')}</strong>
    <p className="m-0 text-xs">{t('capabilities.revision', { revision: snapshot.revision, epoch: snapshot.permission_epoch })} · {snapshot.engine === 'codex' ? 'Codex' : 'Claude'}</p>
    <div className="dev-review-stack text-xs"><strong>{t('capabilities.provider')}</strong><span>{snapshot.provider_id} / {snapshot.model_id} · {t('capabilities.providerPin', { revision: snapshot.provider_revision })}</span></div>
    <p className="m-0 text-xs text-ink-secondary">{t('capabilities.nativeBoundary')}</p>
    <details><summary className="text-xs">{t('capabilities.details')}</summary><pre className="dev-event-text">{JSON.stringify(snapshot, null, 2)}</pre></details>
  </section>;
}

export function SessionConfigurationViewDisplay({ value }: { value: SessionConfigurationView | null }) {
  const { t } = useLocale();
  if (!value) return <p className="m-0 text-xs text-ink-secondary">{t('capabilities.noSnapshot')}</p>;
  if (value.format === 'legacy_history') return <section className="dev-operations dev-detail" aria-label={t('capabilities.actual')}>
    <strong className="text-xs">{t('capabilities.legacyReadOnly')}</strong>
    <p className="m-0 text-xs text-ink-secondary">{t('capabilities.legacyCannotResume')}</p>
    <details><summary className="text-xs">{t('capabilities.details')}</summary><pre className="dev-event-text">{JSON.stringify(value.history, null, 2)}</pre></details>
  </section>;
  return <CapabilitySnapshotView snapshot={value.snapshot} />;
}

export function SessionCapabilitySnapshot({ sessionId, snapshot }: { sessionId: string; snapshot?: SessionConfigurationView | null }) {
  const { t } = useLocale();
  const epoch = useDevelopment(state => state.epoch);
  const { generation } = useContextSelection();
  const [loaded, setLoaded] = useState<{ sessionId: string; epoch: number; generation: number; value: SessionConfigurationView | null; error: string | null } | null>(null);
  useEffect(() => {
    let disposed = false;
    if (snapshot !== undefined) return;
    void readSessionConfiguration(sessionId).then(value => {
      if (!disposed && epoch === useDevelopment.getState().epoch && generation === contextGeneration()) setLoaded({ sessionId, epoch, generation, value, error: null });
    }, failure => {
      if (!disposed && epoch === useDevelopment.getState().epoch && generation === contextGeneration()) setLoaded({ sessionId, epoch, generation, value: null, error: hostCode(failure) });
    });
    return () => { disposed = true; };
  }, [sessionId, epoch, generation, snapshot]);
  const current = loaded?.sessionId === sessionId && loaded.epoch === epoch && loaded.generation === generation ? loaded : null;
  if (snapshot === undefined && !current) return <p role="status" className="text-xs">{t('capabilities.loading')}</p>;
  return <>{current?.error && snapshot === undefined ? <p role="alert" className="text-xs text-status-error">{errorText(current.error)}</p> : null}<SessionConfigurationViewDisplay value={snapshot === undefined ? current?.value ?? null : snapshot} /></>;
}
