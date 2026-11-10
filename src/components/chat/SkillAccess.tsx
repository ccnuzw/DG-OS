import { useEffect, useRef, useState } from 'react';
import { Button, Checkbox, Popover } from '@dgos/ui';
import { BookOpen } from 'lucide-react';
import { t, useLocale } from '../../i18n';
import { chatErrorText, useChatStore, type ChatSession, type ChatSkillEntry, type ChatSkillRecommendation } from '../../stores/chatStore';
import { useDevelopment } from '../../stores/developmentStore';
import { ChatNotice } from './ChatNotice';

export default function SkillAccess({ session, content, disabled }: { session: ChatSession; content: string; disabled: boolean }) {
  useLocale();
  const account = useDevelopment(state => state.epoch);
  const access = useChatStore(state => state.sessionSkillAccess[session.id]);
  const setAccess = useChatStore(state => state.setSkillAccess);
  const catalog = useChatStore(state => state.skillCatalog);
  const [open, setOpen] = useState(false);
  const [items, setItems] = useState<ChatSkillEntry[]>([]);
  const [suggestions, setSuggestions] = useState<ChatSkillRecommendation[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const revision = useRef(0);
  const pins = access?.authorized_skill_refs ?? [];
  const enabled = access?.skill_mode === 'on_demand';
  useEffect(() => { revision.current++; setItems([]); setSuggestions([]); setError(null); setLoading(false); setOpen(false); }, [session.id, account]);
  useEffect(() => { revision.current++; setSuggestions([]); setLoading(false); }, [content]);
  useEffect(() => () => { revision.current++; }, []);
  const load = async (recommend: boolean) => {
    const ticket = ++revision.current;
    setLoading(true); setError(null);
    try {
      const result = await catalog(recommend ? content : '');
      if (ticket !== revision.current || !result) return;
      setItems(result.items); setSuggestions(result.recommendations);
    } catch (failure) { if (ticket === revision.current) setError(chatErrorText(failure)); }
    finally { if (ticket === revision.current) setLoading(false); }
  };
  const choose = (entry: ChatSkillEntry, selected: boolean) => {
    setAccess(session.id, { skill_mode: enabled ? 'on_demand' : 'manual', authorized_skill_refs: selected ? [...pins.filter(pin => pin.skill_id !== entry.pin.skill_id), entry.pin] : pins.filter(pin => pin.skill_id !== entry.pin.skill_id) });
  };
  return <Popover label={t('chat.skillAccess')} side="top" align="start" open={open} onOpenChange={next => { setOpen(next); if (next) void load(false); }} trigger={<Button type="button" size="sm" variant="ghost" iconOnly aria-label={t('chat.skillAccess')} title={t('chat.skillAccess')} className={enabled ? 'chat-composer-selected' : undefined}><BookOpen size={16} aria-hidden="true" /></Button>} content={<section className="chat-inspector">
    <h3 className="m-0 text-sm font-semibold">{t('chat.skillAccess')}</h3>
    <p className="m-0 text-xs text-ink-secondary">{t('chat.skillSuggestionHelp')}</p>
    <Button type="button" size="sm" variant="secondary" disabled={disabled || !content.trim()} loading={loading} onClick={() => void load(true)}>{t('chat.recommendSkills')}</Button>
    {error ? <ChatNotice tone="error">{error}</ChatNotice> : null}
    {suggestions.length ? <section aria-label={t('chat.skillRecommendations')} className="space-y-2"><strong className="text-xs">{t('chat.skillRecommendations')}</strong>{suggestions.map(suggestion => <div key={suggestion.entry.pin.skill_id} className="text-xs wrap-anywhere"><strong>{suggestion.entry.name}</strong><p className="m-0">{t('chat.skillLexicalReason', { reasons: suggestion.reasons.join('; ') })}</p><Button type="button" size="xs" variant="ghost" disabled={disabled || pins.some(pin => pin.skill_id === suggestion.entry.pin.skill_id)} onClick={() => choose(suggestion.entry, true)}>{t('chat.addAuthorizedSkill')}</Button></div>)}</section> : null}
    <Checkbox disabled={disabled} checked={enabled} label={t('chat.allowSkillLoading')} onChange={checked => setAccess(session.id, { skill_mode: checked ? 'on_demand' : 'manual', authorized_skill_refs: pins })} />
    <p className="m-0 text-xs text-ink-secondary">{t('chat.skillAuthorizationHelp')}</p>
    <div className="chat-inspector-list" role="group" aria-label={t('chat.authorizedSkills')}>
      {items.map(entry => { const pinned = pins.find(pin => pin.skill_id === entry.pin.skill_id); const changed = pinned && (pinned.version !== entry.pin.version || pinned.content_hash !== entry.pin.content_hash); return <div key={entry.pin.skill_id} className="space-y-1"><Checkbox disabled={disabled} checked={!!pinned} onChange={checked => choose(entry, checked)} label={<span>{entry.name}<small className="block text-xs text-ink-secondary wrap-anywhere">{entry.description}</small></span>} /><details className="text-xs wrap-anywhere"><summary>{t('chat.skillPin', { version: pinned?.version ?? entry.pin.version })}</summary><p className="m-0">{pinned?.content_hash ?? entry.pin.content_hash}</p></details>{changed ? <ChatNotice tone="warning">{t('chat.skillPinChanged')}</ChatNotice> : null}</div>; })}
      {!loading && !items.length ? <p className="text-xs text-ink-secondary">{t('chat.noAvailableSkills')}</p> : null}
      {pins.filter(pin => !items.some(entry => entry.pin.skill_id === pin.skill_id)).map(pin => <div key={pin.skill_id} className="text-xs wrap-anywhere">{pin.skill_id} · {pin.version}<Button type="button" size="xs" variant="ghost" disabled={disabled} onClick={() => setAccess(session.id, { skill_mode: enabled ? 'on_demand' : 'manual', authorized_skill_refs: pins.filter(item => item.skill_id !== pin.skill_id) })}>{t('chat.removeUnavailableSkill')}</Button></div>)}
    </div>
    {enabled && !pins.length ? <ChatNotice tone="warning">{t('chat.chooseAuthorizedSkills')}</ChatNotice> : null}
    <p className="m-0 text-xs text-ink-secondary">{t('chat.skillReceiptsHelp')}</p>
  </section>} />;
}
