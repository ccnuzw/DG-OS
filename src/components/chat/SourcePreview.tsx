import { createContext, useContext, useState, type ReactNode } from 'react';
import { Button, Modal, cn } from '@dgos/ui';
import { FOCUS_RING } from '@dgos/ui/lib/focus';
import { openUrl } from '@tauri-apps/plugin-opener';
import { formatDateTime, t, useLocale } from '../../i18n';
import type { ChatWebSource } from '../../stores/chatStore';
import { safeSourceUrl } from './sources';

export const TrustedSourcesContext = createContext<ChatWebSource[]>([]);

export function ConfirmedExternalLink({ url, children }: { url: string; children: ReactNode }) {
  useLocale();
  const [open, setOpen] = useState(false);
  return <>
    <Button type="button" variant="link" className={cn('chat-markdown-link inline h-auto whitespace-normal text-left underline wrap-anywhere', FOCUS_RING)} onClick={() => setOpen(true)}>{children}</Button>
    <Modal open={open} onOpenChange={setOpen} title={t('chat.openExternal')} icon="warning" description={<span>{t('chat.externalHelp')}<span className="mt-2 block font-mono text-xs wrap-anywhere">{url}</span></span>} confirmText={t('chat.open')} onConfirm={() => openUrl(url)} />
  </>;
}

export function SourcePreview({ source, children }: { source: ChatWebSource; children?: ReactNode }) {
  useLocale();
  const [open, setOpen] = useState(false);
  const url = safeSourceUrl(source.url);
  return <>
    <Button type="button" variant="link" className="chat-source-link" onClick={() => setOpen(true)}>{children ?? (source.title || source.url)}</Button>
    <Modal open={open} onOpenChange={setOpen} kind="alert" title={source.title || t('chat.sourcePreview')} description={url ? new URL(url).hostname : t('chat.sourceUnavailable')} footer={<Button variant="secondary" onClick={() => setOpen(false)}>{t('chat.activityClose')}</Button>}>
      <section className="chat-source-preview">
        <p>{t(source.kind === 'web_page' ? 'chat.sourcePage' : 'chat.sourceSearch')}</p>
        <p className="chat-source-excerpt">{source.excerpt || t('chat.sourceNoExcerpt')}</p>
        <p className="text-xs text-ink-secondary">{t('chat.sourceRetrieved', { time: formatDateTime(new Date(source.retrieved_at * 1000)) })}</p>
        {source.truncated ? <p className="text-xs text-status-warning-text">{t('chat.sourceTruncated')}</p> : null}
        <p className="text-xs text-ink-secondary">{t('chat.sourceSnapshotHelp')}</p>
        {url ? <ConfirmedExternalLink url={url}>{t('chat.sourceOpen')}</ConfirmedExternalLink> : null}
      </section>
    </Modal>
  </>;
}

export function SourceCitation({ id, children }: { id: string; children: ReactNode }) {
  useLocale();
  const source = useContext(TrustedSourcesContext).find(item => item.id === id);
  return source ? <SourcePreview source={source}>{children}</SourcePreview> : <span className="chat-source-unavailable" title={t('chat.sourceUnavailable')}>{children} ({t('chat.sourceUnavailable')})</span>;
}
