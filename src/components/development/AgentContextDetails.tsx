import { useLocale } from '../../i18n';
import type { AgentContextUsage, AgentTokenCounts } from './agentContextUsage';

export function AgentContextDetails({ usage }: { usage: AgentContextUsage }) {
  const { t, locale } = useLocale();
  function counts(value: AgentTokenCounts, label: string) {
    return <section aria-label={t(label)}><h4>{t(label)}</h4><dl>{(['input', 'output', 'cached', 'reasoning', 'total'] as const).map(key => <div key={key}><dt>{t(`codexTranscript.tokens.${key}`)}</dt><dd>{value[key] === null ? t('agentControls.unavailable') : value[key].toLocaleString(locale)}</dd></div>)}</dl></section>;
  }
  return <><h3>{t('agentControls.context')}</h3><dl><dt>{t('agentControls.used')}</dt><dd>{usage.used === null ? t('agentControls.unavailable') : `${usage.used.toLocaleString(locale)} ${t('agentControls.estimated')}`}</dd><dt>{t('agentControls.limit')}</dt><dd>{usage.limit === null ? t('agentControls.unavailable') : usage.limit.toLocaleString(locale)}</dd><dt>{t('agentControls.source')}</dt><dd className="break-words">{usage.source || t('agentControls.noNativeData')}</dd>{usage.seq !== null ? <><dt>{t('codexTranscript.sequence')}</dt><dd>#{usage.seq}</dd></> : null}</dl>{usage.latest ? counts(usage.latest, 'codexTranscript.latestUsage') : null}{usage.cumulative ? counts(usage.cumulative, 'codexTranscript.cumulativeUsage') : null}<p>{t('agentControls.contextExplanation')}</p>{usage.latest ? <p>{t('codexTranscript.usageBoundary')}</p> : null}{usage.previous ? <p role="status">{t('agentControls.previousUsage')}</p> : null}</>;
}
