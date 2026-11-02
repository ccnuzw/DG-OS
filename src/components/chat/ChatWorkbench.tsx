import { t, useLocale as usePageLocale } from '../../i18n';
import { useEffect, useRef, useState } from 'react';
import { Button, Input, Modal } from '@dgos/ui';
import { useChatStore, type PendingDestination } from '../../stores/chatStore';
import { ChatNotice } from './ChatNotice';
import Composer from './Composer';
import Disclosures from './Disclosures';
import MessageList from './MessageList';
import SessionSidebar from './SessionSidebar';
import TargetBar from './TargetBar';

function SessionHeader() {
  usePageLocale();
  const session = useChatStore(state => state.activeSession);
  const rename = useChatStore(state => state.renameSession);
  const remove = useChatStore(state => state.deleteSession);
  const [renaming, setRenaming] = useState(false);
  const [title, setTitle] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const renameButton = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    setRenaming(false);
    setError(null);
  }, [session?.id]);

  if (!session) return null;

  const submit = async () => {
    const failure = await rename(session.id, title);
    if (failure) { setError(failure); return; }
    setRenaming(false);
    setError(null);
    requestAnimationFrame(() => renameButton.current?.focus());
  };

  return (
    <header className="flex flex-wrap items-start justify-between gap-2">
      {renaming ? (
        <form className="flex min-w-0 grow flex-wrap items-start gap-2" onSubmit={event => { event.preventDefault(); void submit(); }}
          onKeyDown={event => { if (event.key === 'Escape') { event.stopPropagation(); setRenaming(false); setError(null); } }}>
          <div className="min-w-0 grow basis-48">
            <Input aria-label={t('chat.title')} value={title} error={error ?? undefined} autoFocus onChange={event => setTitle(event.target.value)} />
          </div>
          <Button type="submit">{t('chat.saveTitle')}</Button>
          <Button variant="secondary" onClick={() => { setRenaming(false); setError(null); }}>{t('chat.cancel')}</Button>
        </form>
      ) : (
        <h2 className="m-0 min-w-0 text-lg font-semibold wrap-anywhere">{session.title}</h2>
      )}
      {!renaming ? (
        <div className="flex gap-2">
          <Button ref={renameButton} variant="secondary" onClick={() => { setTitle(session.title); setError(null); setRenaming(true); }}>{t('chat.rename')}</Button>
          <Button variant="secondary" onClick={() => setConfirmDelete(true)}>{t('chat.delete')}</Button>
        </div>
      ) : null}
      {error && !renaming ? <ChatNotice tone="error" className="w-full">{error}</ChatNotice> : null}
      <Modal
        open={confirmDelete}
        onOpenChange={setConfirmDelete}
        title={t('chat.delete')}
        icon="warning"
        description={t('chat.deleteConfirm')}
        confirmText="删除"
        confirmVariant="danger"
        onConfirm={async () => {
          const failure = await remove(session.id);
          if (failure) {
            setError(failure);
            setConfirmDelete(false);
          }
        }}
      />
    </header>
  );
}

function DestinationDialog() {
  usePageLocale();
  const pending = useChatStore(state => state.pendingDestination);
  const confirm = useChatStore(state => state.confirmDestination);
  const dismiss = useChatStore(state => state.dismissDestination);
  // Keep the last request so the dialog can fade through a close without its text jumping.
  const shown = useRef<PendingDestination | null>(null);
  if (pending) shown.current = pending;
  return (
    <Modal
      open={pending !== null}
      onOpenChange={open => { if (!open) dismiss(pending ?? undefined); }}
      title={t('chat.destination')}
      icon="warning"
      description={<span>{t('chat.destinationBefore')}{shown.current?.host ? t('chat.destinationHost', { host: shown.current.host }) : ''}{t('chat.destinationAfter')}</span>}
      confirmText={t('chat.confirmSend')}
      onConfirm={confirm}
    />
  );
}

export default function ChatWorkbench() {
  usePageLocale();
  const session = useChatStore(state => state.activeSession);
  const stale = useChatStore(state => state.activeStale);
  const notice = useChatStore(state => state.notice);
  const loadSessions = useChatStore(state => state.loadSessions);
  const dismissNotice = useChatStore(state => state.dismissNotice);
  const cancelOnLeave = useChatStore(state => state.cancelOnLeave);
  const createSession = useChatStore(state => state.createSession);

  useEffect(() => {
    void loadSessions();
    // Leaving the page cancels any running generation (B06-61). The partial text is kept by the host as interrupted.
    return () => cancelOnLeave();
  }, [loadSessions, cancelOnLeave]);

  return (
    <div className="flex h-full min-h-0 gap-4 p-4">
      <aside className="flex w-72 shrink-0 flex-col" aria-label={t('chat.sidebar')}>
        <SessionSidebar />
      </aside>
      <section aria-label={t('chat.chat')} className="flex min-h-0 min-w-0 grow flex-col gap-3">
        {session ? (
          <>
            <SessionHeader />
            <TargetBar session={session} />
            {stale ? <ChatNotice tone="error">{t('chat.stale')}</ChatNotice> : null}
            <MessageList />
            {notice ? <ChatNotice tone={notice.tone} onDismiss={dismissNotice}>{notice.text}</ChatNotice> : null}
            <Composer key={session.id} />
          </>
        ) : (
          <div className="flex grow flex-col items-start justify-center gap-3">
            <h2 className="m-0 text-lg font-semibold">{t('chat.start')}</h2>
            <p className="m-0 text-sm text-muted-foreground">{t('chat.startHelp')}</p>
            <Button onClick={() => void createSession()}>{t('chat.newSession')}</Button>
          </div>
        )}
        <Disclosures />
      </section>
      <DestinationDialog />
    </div>
  );
}
