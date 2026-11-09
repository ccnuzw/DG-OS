import { useEffect, useState } from 'react';
import { useDevelopment } from '../../stores/developmentStore';
import { contextGeneration, useContextSelection } from '../../stores/developmentSelection';
import { readSessionConfiguration, type CliProviderSnapshot, type SessionConfigurationView } from '../../stores/developmentCapabilities';
import { hostCode } from '../../stores/hostError';
import { errorText } from './shared';
import { useLocale } from '../../i18n';

interface RuntimeContext { workspace?: string | null; cli_version?: string | null }

export function CapabilitySnapshotView({ snapshot, runtime }: { snapshot: CliProviderSnapshot | null | undefined; runtime?: RuntimeContext }) {
  const { t } = useLocale();
  if (!snapshot) return <p className="m-0 text-xs text-ink-secondary">{t('capabilities.noSnapshot')}</p>;
  return <section className="dev-operations dev-detail" aria-label={t('capabilities.actual')}>
    <strong className="text-xs">{t('capabilities.actual')}</strong>
    <p className="m-0 text-xs">{t('capabilities.revision', { revision: snapshot.revision, epoch: snapshot.permission_epoch })} · {snapshot.engine === 'codex' ? 'Codex' : 'Claude'}</p>
    <dl className="dev-repository m-0 grid min-w-0 gap-x-3 gap-y-1 text-xs">
      <dt>{t('capabilities.cli')}</dt><dd className="m-0 break-all">{runtime?.cli_version || t('capabilities.notRecorded')}</dd>
      <dt>{t('capabilities.workspace')}</dt><dd className="m-0 break-all">{runtime?.workspace || t('capabilities.notRecorded')}</dd>
      <dt>{t('capabilities.provider')}</dt><dd className="m-0 break-all">{snapshot.provider_id} / {snapshot.model_id}</dd>
      <dt>{t('capabilities.source')}</dt><dd className="m-0">{t('capabilities.catalogSource')}</dd>
    </dl>
    <p className="m-0 text-xs">{t('capabilities.providerPin', { revision: snapshot.provider_revision })}</p>
    <p className="m-0 text-xs text-ink-secondary">{t('capabilities.configurationEvidence')}</p>
    <div className="dev-review-stack text-xs"><strong>{t('capabilities.nativeSkillsSource')}</strong><span>{t('capabilities.nativeMcpSource')}</span></div>
    <p className="m-0 text-xs text-ink-secondary">{t('capabilities.discoveryEvidence')}</p>
    <p className="m-0 text-xs text-ink-secondary">{t('capabilities.nativeBoundary')}</p>
    <details><summary className="text-xs">{t('capabilities.details')}</summary><pre className="dev-event-text">{JSON.stringify(snapshot, null, 2)}</pre></details>
  </section>;
}

export function SessionConfigurationViewDisplay({ value, runtime }: { value: SessionConfigurationView | null; runtime?: RuntimeContext }) {
  const { t } = useLocale();
  if (!value) return <p className="m-0 text-xs text-ink-secondary">{t('capabilities.noSnapshot')}</p>;
  if (value.format === 'legacy_history') return <section className="dev-operations dev-detail" aria-label={t('capabilities.actual')}>
    <strong className="text-xs">{t('capabilities.legacyReadOnly')}</strong>
    <p className="m-0 text-xs text-ink-secondary">{t('capabilities.legacyCannotResume')}</p>
    <details><summary className="text-xs">{t('capabilities.details')}</summary><pre className="dev-event-text">{JSON.stringify(value.history, null, 2)}</pre></details>
  </section>;
  return <CapabilitySnapshotView snapshot={value.snapshot} runtime={runtime} />;
}

export function SessionCapabilitySnapshot({ sessionId, snapshot, runtime }: { sessionId: string; snapshot?: SessionConfigurationView | null; runtime?: RuntimeContext }) {
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
  return <>{current?.error && snapshot === undefined ? <p role="alert" className="text-xs text-status-error">{errorText(current.error)}</p> : null}<SessionConfigurationViewDisplay value={snapshot === undefined ? current?.value ?? null : snapshot} runtime={runtime} /></>;
}
