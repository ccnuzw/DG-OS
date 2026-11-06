import { useEffect, useState } from 'react';
import { useDevelopment } from '../../stores/developmentStore';
import { contextGeneration, useContextSelection } from '../../stores/developmentSelection';
import { readSessionCapabilities } from '../../stores/developmentCapabilities';
import { hostCode } from '../../stores/hostError';
import { errorText } from './shared';
import { useLocale } from '../../i18n';
import type { CapabilitySnapshot } from '../../stores/developmentCapabilities';

export function CapabilitySnapshotView({ snapshot }: { snapshot: CapabilitySnapshot | null | undefined }) {
  const { t } = useLocale();
  if (!snapshot) return <p className="m-0 text-xs text-ink-secondary">{t('capabilities.noSnapshot')}</p>;
  const { configuration } = snapshot;
  return <section className="dev-operations dev-detail" aria-label={t('capabilities.actual')}>
    <strong className="text-xs">{t('capabilities.actual')}</strong>
    <p className="m-0 text-xs">{t('capabilities.revision', { revision: snapshot.revision, epoch: snapshot.permission_epoch })} · {snapshot.engine === 'codex' ? 'Codex' : 'Claude'} · {t('capabilities.cli')}: {snapshot.cli_version}</p>
    <div className="dev-review-stack text-xs"><strong>{t('capabilities.provider')}</strong><span>{configuration.provider.source === 'native' ? t('capabilities.native') : t('capabilities.snapshotProvider', { source: t('capabilities.catalogSource'), provider: configuration.provider.provider_id, model: configuration.provider.model_id, revision: configuration.provider.revision })}</span><span className="break-all">{t('capabilities.hostStatus')}: {snapshot.provider_status}</span><span>{t('capabilities.unverified')}</span></div>
    <div className="dev-review-stack text-xs"><strong>{t('capabilities.skills')}</strong><span>{t('capabilities.preparedObserved')}</span><span className="break-all">{t('capabilities.hostStatus')}: {snapshot.skills_status}</span>{configuration.skills.map(skill => <span className="break-all" key={`${skill.id}:${skill.hash}`}>{skill.id} · {skill.version} · {skill.hash}</span>)}<span>{t('capabilities.skillBoundary')}</span></div>
    <div className="dev-review-stack text-xs"><strong>{t('capabilities.mcp')}</strong><span>{t(snapshot.revision > 0 ? 'capabilities.hostMcpSource' : 'capabilities.nativeMcpSource')}</span><span className="break-all">{t('capabilities.hostStatus')}: {snapshot.mcp_status}</span>{configuration.mcp.map(tool => <span className="break-all" key={`${tool.server_id}:${tool.tool_name}`}>{tool.server_id} / {tool.tool_name} · {t('capabilities.schema')}: {tool.schema_hash}</span>)}<span>{t('capabilities.mcpBoundary')}</span></div>
    <p className="m-0 text-xs text-ink-secondary">{t('capabilities.nativeBoundary')}</p>
    <details><summary className="text-xs">{t('capabilities.details')}</summary><pre className="dev-event-text">{JSON.stringify(snapshot, null, 2)}</pre></details>
  </section>;
}

export function SessionCapabilitySnapshot({ sessionId, snapshot }: { sessionId: string; snapshot: CapabilitySnapshot | null | undefined }) {
  const { t } = useLocale();
  const epoch = useDevelopment(state => state.epoch);
  const { generation } = useContextSelection();
  const [loaded, setLoaded] = useState<{ sessionId: string; epoch: number; generation: number; value: CapabilitySnapshot | null; error: string | null } | null>(null);
  useEffect(() => {
    let disposed = false;
    if (snapshot !== undefined) return;
    void readSessionCapabilities(sessionId).then(value => {
      if (!disposed && epoch === useDevelopment.getState().epoch && generation === contextGeneration()) setLoaded({ sessionId, epoch, generation, value, error: null });
    }, failure => {
      if (!disposed && epoch === useDevelopment.getState().epoch && generation === contextGeneration()) setLoaded({ sessionId, epoch, generation, value: null, error: hostCode(failure) });
    });
    return () => { disposed = true; };
  }, [sessionId, epoch, generation, snapshot]);
  const current = loaded?.sessionId === sessionId && loaded.epoch === epoch && loaded.generation === generation ? loaded : null;
  if (snapshot === undefined && !current) return <p role="status" className="text-xs">{t('capabilities.loading')}</p>;
  return <>{current?.error && snapshot === undefined ? <p role="alert" className="text-xs text-status-error">{errorText(current.error)}</p> : null}<CapabilitySnapshotView snapshot={snapshot === undefined ? current?.value : snapshot} /></>;
}
