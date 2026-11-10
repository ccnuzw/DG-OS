import { Checkbox, Input, Stack } from '@dgos/ui';
import { t, useLocale } from '../../i18n';
import { useChatStore } from '../../stores/chatStore';
import { ChatNotice } from './ChatNotice';
import './chatResources.css';

/** The host validates the contract after generation; enabling it never grants another call. */
export default function OutputContractControls({ sessionId, disabled }: { sessionId: string; disabled: boolean }) {
  useLocale();
  const settings = useChatStore(state => state.sessionFoundation[sessionId]);
  const setFoundation = useChatStore(state => state.setFoundation);
  const schema = settings?.output_schema ?? null;
  let invalidSchema = false;
  if (schema !== null) { try { JSON.parse(schema); } catch { invalidSchema = true; } }
  const update = (output_schema: string | null) => {
    const current = useChatStore.getState().sessionFoundation[sessionId];
    setFoundation(sessionId, { context_refs: current?.context_refs ?? [], output_schema });
  };
  return <Stack gap={3}>
    <Checkbox label={t('chat.structuredOutput')} checked={schema !== null} disabled={disabled} onChange={checked => update(checked ? '' : null)} />
    <p className="m-0 text-xs text-ink-secondary">{t('chat.outputSchemaHelp')}</p>
    {schema !== null ? <details open className="chat-material-contract">
      <summary>{t('chat.materialAdvancedSchema')}</summary>
      <Input label={t('chat.outputSchema')} multiline rows={6} value={schema} disabled={disabled} onChange={event => update(event.target.value)} />
    </details> : null}
    {invalidSchema ? <ChatNotice tone="error">{t('chat.invalidOutputSchema')}</ChatNotice> : null}
  </Stack>;
}
