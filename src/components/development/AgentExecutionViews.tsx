import { useMemo, useState, type ReactNode } from 'react';
import { Button } from '@dgos/ui';
import { Check, Circle, CircleDot, HelpCircle } from 'lucide-react';
import { useLocale } from '../../i18n';
import type { Json } from '../../stores/agentRunnerStore';
import { COPY_TEXT, useCopyFeedback } from '../chat/clipboard';
import { parseUnifiedDiff } from './GitFileDiff';
import './agent-reply.css';

export function LazyDetails({ summary, children, className = '', ...props }: { summary: ReactNode; children: ReactNode | (() => ReactNode); className?: string; 'data-testid'?: string; 'data-operation-state'?: string }) {
  const [mounted, setMounted] = useState(false);
  return <details {...props} className={`agent-reply-disclosure ${className}`} onToggle={event => { if (event.currentTarget.open) setMounted(true); }}>
    <summary>{summary}</summary>
    {mounted ? <div className="agent-reply-detail-body">{typeof children === 'function' ? children() : children}</div> : null}
  </details>;
}

const privateField = /^(?:thinking|thinking_delta|signature|signature_delta|redacted_thinking|encrypted_content|encryptedContent)$/;
export function NativeValue({ value }: { value: Json | undefined }) {
  if (value === undefined || value === null) return null;
  if (Array.isArray(value)) return <div className="agent-reply-values">{value.map((entry, index) => <NativeValue key={index} value={entry} />)}</div>;
  if (typeof value === 'object') {
    if (['thinking', 'redacted_thinking', 'signature', 'thinking_delta', 'signature_delta', 'reasoning'].includes(typeof value.type === 'string' ? value.type : '')) return null;
    return <dl className="agent-reply-fields">{Object.entries(value).filter(([key]) => !privateField.test(key)).map(([key, entry]) => <div key={key}><dt>{key}</dt><dd><NativeValue value={entry} /></dd></div>)}</dl>;
  }
  return <pre className="type-code agent-reply-native-value">{String(value)}</pre>;
}

export function PlanView({ plan, explanation }: { plan: readonly Json[]; explanation?: string }) {
  const { t } = useLocale();
  return <section className="agent-reply-plan" aria-label={t('execution.plan')}>
    {explanation ? <p>{explanation}</p> : null}
    {plan.length ? <ol>{plan.map((value, index) => {
      const item: Record<string, Json> = value && typeof value === 'object' && !Array.isArray(value) ? value : {};
      const raw = typeof item.status === 'string' ? item.status : '';
      const status = raw === 'inProgress' ? 'in_progress' : raw;
      const known = ['pending', 'in_progress', 'completed'].includes(status);
      const Icon = status === 'completed' ? Check : status === 'in_progress' ? CircleDot : status === 'pending' ? Circle : HelpCircle;
      return <li key={index} data-plan-status={known ? status : 'unknown'}><Icon size={14} aria-hidden="true" /><span>{typeof item.step === 'string' ? item.step : typeof item.content === 'string' ? item.content : t('agentExecution.missingStep')}</span><small>{t(`execution.${known ? status : 'unknown'}`)}{raw && !known ? ` (${raw})` : ''}</small></li>;
    })}</ol> : <p className="agent-reply-note">{t('agentExecution.emptyPlan')}</p>}
  </section>;
}

export function AgentDiff({ diff, source = 'native', seq, path }: { diff: string; source?: 'native' | 'disk'; seq?: number; path?: string }) {
  const { t } = useLocale();
  const rows = useMemo(() => parseUnifiedDiff(diff), [diff]);
  const [state, copy] = useCopyFeedback();
  return <section className="agent-reply-diff" aria-label={t('execution.diff')} data-diff-source={source}>
    <header><span>{t(source === 'disk' ? 'agentExecution.diskDiff' : 'agentExecution.nativeDiff')}{path ? <code>{path}</code> : null}</span><span className="agent-reply-copy"><span role="status">{COPY_TEXT[state]}</span><Button size="xs" variant="ghost" onClick={() => copy(diff)}>{t('chat.copyCode')}</Button></span></header>
    <p className="agent-reply-note">{t(source === 'disk' ? 'agentExecution.diskDiffBoundary' : 'agentExecution.nativeDiffBoundary')}</p>
    <div className="agent-reply-diff-scroll type-code" role="region" aria-label={t('execution.diff')} tabIndex={0}>
      {rows.map((row, index) => <div className={`agent-reply-diff-line agent-reply-diff-${row.kind}`} key={index} data-before-line={row.before ?? undefined} data-after-line={row.after ?? undefined}>
        <span className="agent-reply-diff-number" aria-label={t('agentExecution.beforeLine')}>{row.before}</span><span className="agent-reply-diff-number" aria-label={t('agentExecution.afterLine')}>{row.after}</span><code>{row.text || '\u00a0'}</code>
      </div>)}
    </div>
    {seq !== undefined ? <small className="agent-reply-note">{t('agentExecution.source', { seq })}</small> : null}
  </section>;
}
