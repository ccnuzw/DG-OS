import { Button, Checkbox, Input, Select, Stack } from '@dgos/ui';
import { useLocale } from '../../i18n';
import type { Skill } from '../../stores/skillsStore';
import type { Model } from '../../stores/providerStore';
export interface GenerationParameters { temperature?: number | null; reasoning?: { effort?: string; budget_tokens?: number } | null; max_tokens?: number | null; instruction?: string }
export function GenerationControls({ value, onChange, disabled, model }: { value: GenerationParameters; onChange: (value: GenerationParameters) => void; disabled?: boolean; model?: Model }) {
  const { t } = useLocale();
  const patch = (next: Partial<GenerationParameters>) => onChange({ ...value, ...next });
  return <Stack gap={3}>
    <Select label={t('generation.length')} value={String(value.max_tokens ?? 1024)} disabled={disabled} options={[1024,4096,8192].map((tokens, index) => ({ value: String(tokens), label: t(['generation.short','generation.medium','generation.long'][index]) }))} onChange={v => patch({ max_tokens: Number(v) })} />
    <Input label={t('generation.temperature')} type="number" min={0} max={2} step={0.1} value={value.temperature ?? ''} disabled={disabled} onChange={e => patch({ temperature: e.target.value === '' ? null : Number(e.target.value) })} />
    <Select label={t('generation.reasoning')} value={value.reasoning?.effort ?? '__default__'} disabled={disabled} options={['__default__','low','medium','high'].map(v => ({ value: v, label: t(`generation.${v === '__default__' ? 'default' : v === 'medium' ? 'effortMedium' : v}`) }))} onChange={v => patch({ reasoning: v === '__default__' ? null : { effort: v, ...(value.reasoning?.budget_tokens !== undefined ? { budget_tokens: value.reasoning.budget_tokens } : {}) } })} />
    <Input label={t('generation.reasoningBudget')} type="number" min={1} step={1} value={value.reasoning?.budget_tokens ?? ''} disabled={disabled} onChange={e => patch({ reasoning: e.target.value === '' ? (value.reasoning?.effort ? { effort: value.reasoning.effort } : null) : { ...value.reasoning, budget_tokens: Number(e.target.value) } })} />
    <Input label={t('generation.instruction')} multiline rows={3} value={value.instruction ?? ''} disabled={disabled} onChange={e => patch({ instruction: e.target.value })} />
    <p className="m-0 text-xs text-ink-secondary">{t('generation.capabilityNotice')} {model?.max_output_tokens ? t('generation.outputLimit', { count: model.max_output_tokens }) : t('generation.unknownLimit')}</p>
    <p className="m-0 text-xs text-ink-secondary">{t('generation.reasoningCapability', { state: t(`generation.${model?.capabilities?.reasoning ?? 'unknown'}`) })}</p>
  </Stack>;
}
export function OrderedSkills({ skills, selected, onChange, without, onWithoutChange, disabled, id }: { skills: Skill[]; selected: string[]; onChange: (ids: string[]) => void; without: boolean; onWithoutChange: (value: boolean) => void; disabled?: boolean; id?: string }) {
  const { t } = useLocale();
  const move = (index: number, offset: number) => { const next = [...selected]; [next[index], next[index + offset]] = [next[index + offset], next[index]]; onChange(next); };
  return <fieldset id={id} tabIndex={-1} disabled={disabled} className="m-0 min-w-0 border-0 p-0">
    <legend className="mb-2 text-sm font-medium">{t('chat.skill')}</legend>
    <Stack gap={2}>
      {skills.map(skill => <Checkbox key={skill.skill_id} checked={selected.includes(skill.skill_id)} label={`${skill.name} — v${skill.version}`} onChange={on => onChange(on ? [...selected, skill.skill_id] : selected.filter(id => id !== skill.skill_id))} />)}
      {selected.map((id, index) => <div key={id} className="flex min-w-0 flex-wrap items-center gap-2"><span className="min-w-0 grow wrap-anywhere">{index + 1}. {skills.find(s => s.skill_id === id)?.name ?? id}</span><Button size="sm" variant="ghost" disabled={disabled || index === 0} onClick={() => move(index, -1)} aria-label={`${t('generation.moveUp')} ${id}`}>{t('generation.moveUp')}</Button><Button size="sm" variant="ghost" disabled={disabled || index === selected.length - 1} onClick={() => move(index, 1)} aria-label={`${t('generation.moveDown')} ${id}`}>{t('generation.moveDown')}</Button><Button size="sm" variant="ghost" disabled={disabled} onClick={() => onChange(selected.filter(item => item !== id))}>{t('common.remove')}</Button></div>)}
      {!selected.length && <Checkbox label={t('chat.explicitNoSkill')} checked={without} onChange={onWithoutChange} />}
      {selected.length > 1 && <p className="m-0 text-xs text-status-warning-text">{t('generation.combinationNotice')}</p>}
    </Stack>
  </fieldset>;
}
