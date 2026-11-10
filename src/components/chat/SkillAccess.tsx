import { useEffect, useRef, useState } from 'react';
import { Button, Checkbox } from '@dgos/ui';
import { t, useLocale } from '../../i18n';
import { chatErrorText, useChatStore, type ChatSession, type ChatSkillEntry, type ChatSkillRecommendation } from '../../stores/chatStore';
import { useDevelopment } from '../../stores/developmentStore';
import { ChatNotice } from './ChatNotice';

/** Content for TargetBar's single skills picker; opening it never grants access. */
export default function SkillAccess({ session, content, disabled, active = true }: { session: ChatSession; content: string; disabled: boolean; active?: boolean }) {
  useLocale();
  const account = useDevelopment(state => state.epoch);
  const access = useChatStore(state => state.sessionSkillAccess[session.id]);
  const setAccess = useChatStore(state => state.setSkillAccess);
  const catalog = useChatStore(state => state.skillCatalog);
  const [items, setItems] = useState<ChatSkillEntry[]>([]);
  const [suggestions, setSuggestions] = useState<ChatSkillRecommendation[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const revision = useRef(0);
  const scope = useRef({ sessionId: session.id, account, content, active });
  scope.current = { sessionId: session.id, account, content, active };
  const pins = access?.authorized_skill_refs ?? [];
  const enabled = access?.skill_mode === 'on_demand';
  const load = async (recommend: boolean) => {
    const ticket = ++revision.current;
    const owner = scope.current;
    const current = () => ticket === revision.current && owner.sessionId === scope.current.sessionId && owner.account === useDevelopment.getState().epoch && owner.active === scope.current.active && (!recommend || owner.content === scope.current.content);
    setLoading(true); setError(null);
    try {
      const result = await catalog(recommend ? content : '');
      if (!current() || !result) return;
      setItems(result.items); setSuggestions(result.recommendations);
    } catch (failure) { if (current()) setError(chatErrorText(failure)); }
    finally { if (current()) setLoading(false); }
  };
  useEffect(() => {
    revision.current++; setItems([]); setSuggestions([]); setError(null); setLoading(false);
    if (active) void load(false);
    return () => { revision.current++; };
  }, [session.id, account, active]);
  useEffect(() => { setSuggestions([]); setLoading(false); }, [content]);
  const choose = (entry: ChatSkillEntry, selected: boolean) => {
    if (disabled) return;
    const next = selected ? (pins.some(pin => pin.skill_id === entry.pin.skill_id) ? pins : [...pins, entry.pin]) : pins.filter(pin => pin.skill_id !== entry.pin.skill_id);
    setAccess(session.id, { skill_mode: enabled ? 'on_demand' : 'manual', authorized_skill_refs: next });
  };
  return <section className="chat-skill-choice-optional" aria-label={t('chat.skillChoiceOptional')}>
    <h3 className="m-0 text-sm font-semibold">{t('chat.skillChoiceOptional')}</h3>
    <p className="chat-picker-description">{t('chat.skillChoiceOptionalHelp')}</p>
    <Button type="button" size="sm" variant="secondary" disabled={disabled || !content.trim()} loading={loading} onClick={() => void load(true)}>{t('chat.recommendSkills')}</Button>
    {error ? <ChatNotice tone="error">{error}</ChatNotice> : null}
    {suggestions.length ? <section aria-label={t('chat.skillRecommendations')} className="space-y-2"><strong className="text-xs">{t('chat.skillRecommendations')}</strong>{suggestions.map(suggestion => <div key={suggestion.entry.pin.skill_id} className="text-xs wrap-anywhere"><strong>{suggestion.entry.name}</strong><p className="m-0">{t('chat.skillLexicalReason', { reasons: suggestion.reasons.join('; ') })}</p><Button type="button" size="xs" variant="ghost" disabled={disabled || pins.some(pin => pin.skill_id === suggestion.entry.pin.skill_id)} onClick={() => choose(suggestion.entry, true)}>{t('chat.skillChoiceAuthorize')}</Button></div>)}</section> : null}
    <Checkbox disabled={disabled} checked={enabled} label={t('chat.allowSkillLoading')} onChange={checked => setAccess(session.id, { skill_mode: checked ? 'on_demand' : 'manual', authorized_skill_refs: pins })} />
    <p className="chat-picker-description">{t('chat.skillAuthorizationHelp')}</p>
    <div className="chat-inspector-list chat-skill-choice-catalog" role="group" aria-label={t('chat.authorizedSkills')} aria-busy={loading}>
      {items.map(entry => {
        const pinned = pins.find(pin => pin.skill_id === entry.pin.skill_id);
        const changed = pinned && (pinned.version !== entry.pin.version || pinned.content_hash !== entry.pin.content_hash);
        return <div key={entry.pin.skill_id} className="space-y-1"><Checkbox disabled={disabled} checked={!!pinned} onChange={checked => choose(entry, checked)} label={<span>{entry.name}<small className="block text-xs text-ink-secondary wrap-anywhere">{entry.description}</small></span>} /><details className="text-xs wrap-anywhere"><summary>{t('chat.skillPin', { version: pinned?.version ?? entry.pin.version })}</summary><p className="m-0">{pinned?.content_hash ?? entry.pin.content_hash}</p></details>{changed ? <ChatNotice tone="warning">{t('chat.skillPinChanged')}</ChatNotice> : null}</div>;
      })}
      {!loading && !items.length && !error ? <p className="text-xs text-ink-secondary">{t('chat.noAvailableSkills')}</p> : null}
      {pins.filter(pin => !items.some(entry => entry.pin.skill_id === pin.skill_id)).map(pin => <div key={pin.skill_id} className="text-xs wrap-anywhere"><details><summary>{pin.skill_id} · {pin.version}</summary>{pin.content_hash}</details><Button type="button" size="xs" variant="ghost" disabled={disabled} onClick={() => setAccess(session.id, { skill_mode: enabled ? 'on_demand' : 'manual', authorized_skill_refs: pins.filter(item => item.skill_id !== pin.skill_id) })}>{t('chat.removeUnavailableSkill')}</Button></div>)}
    </div>
    {enabled && !pins.length ? <ChatNotice tone="warning">{t('chat.chooseAuthorizedSkills')}</ChatNotice> : null}
    {!enabled && pins.length ? <p className="chat-picker-description">{t('chat.skillChoiceInactive')}</p> : null}
    <p className="chat-picker-description">{t('chat.skillReceiptsHelp')}</p>
  </section>;
}
