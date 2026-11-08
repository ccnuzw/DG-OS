import { useEffect, useId, useState } from 'react';
import { RadioGroup as RadioPrimitive } from 'radix-ui';
import { Button, Input, Popover, Select, Stack } from '@dgos/ui';
import { Field, CONTROL_SURFACE } from '../ui/components/Field';
import { CheckGlyph, ChevronGlyph } from '../ui/components/glyphs';
import { FOCUS_ITEM } from '../ui/lib/focus';
import { fontAvailability, type FontSelection } from '../ui/theme/fonts';
import { applyFontSample } from '../ui/theme/apply';
import { loadFontCatalogue, useFontCatalogue } from '../ui/theme/fontCatalogue';
import type { FontRole } from '../ui/theme/typography';
import type { Appearance } from '../ui/appearance/model';
import { useLocale } from '../i18n';

type Typography = Appearance['typography'];
const PAGE_SIZE = 40;

function FontSample({ font, role, sample, testId }: { font: FontSelection; role: FontRole; sample: string; testId?: string }) {
  return <span data-testid={testId} aria-hidden={testId ? undefined : true} className="appearance-font-sample" ref={root => { if (root) applyFontSample(root, font, role); }}>{sample}</span>;
}
function FontPicker({ role, value, disabled, onChange }: { role: FontRole; value: FontSelection; disabled: boolean; onChange: (value: FontSelection) => void }) {
  const { t } = useLocale();
  const id = useId();
  const [opened, setOpened] = useState(false);
  const [search, setSearch] = useState('');
  const [page, setPage] = useState(0);
  const catalogue = useFontCatalogue();
  const availability = fontAvailability(value);
  const system: FontSelection = { kind: role === 'code' ? 'system-mono' : 'system-ui' };
  const systemLabel = t(role === 'code' ? 'appearance.font.systemMono' : 'appearance.font.systemUi');
  const label = value.kind === 'local' ? value.family : systemLabel;
  const options = catalogue.families.filter(font => (role !== 'code' || font.monospace) && font.family.toLocaleLowerCase().includes(search.trim().toLocaleLowerCase()));
  const sample = role === 'code' ? '0 O 1 l I {} => 中文路径' : '中文示例 Aa 0123';
  const select = (font: FontSelection) => { onChange(font); setOpened(false); };
  const status = catalogue.status === 'failed' ? 'catalogueFailed' : availability === 'missing' ? 'fontMissing' : availability === 'unconfirmed' ? 'fontUnconfirmed' : null;
  return <div className="min-w-0">
    <Field id={id} label={t(`appearance.font.${role}`)}>
      <Popover label={t(`appearance.font.${role}`)} open={opened} onOpenChange={open => { setOpened(open); if (open) { setSearch(''); setPage(0); void loadFontCatalogue(); } }} align="start"
        trigger={<button type="button" id={id} data-testid={`appearance-${role}-font`} disabled={disabled} aria-haspopup="listbox" className={`${CONTROL_SURFACE} border-edge-control appearance-font-trigger`}><span>{label}{availability === 'missing' || availability === 'unconfirmed' ? ` · ${systemLabel}` : ''}</span><ChevronGlyph /></button>}
        content={<div className="appearance-font-menu" onKeyDown={event => {
          if (!['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) return;
          if (event.target instanceof HTMLInputElement && !['ArrowDown', 'ArrowUp'].includes(event.key)) return;
          const items = [...event.currentTarget.querySelectorAll<HTMLButtonElement>('[role="option"]')];
          const index = items.indexOf(document.activeElement as HTMLButtonElement);
          const next = event.key === 'Home' ? 0 : event.key === 'End' ? items.length - 1 : event.key === 'ArrowDown' ? (index + 1) % items.length : (index <= 0 ? items.length : index) - 1;
          event.preventDefault(); items[next]?.focus();
        }}>
          <Input type="search" aria-label={t('appearance.fontSearch')} placeholder={t('appearance.fontSearch')} value={search} onChange={event => { setSearch(event.target.value); setPage(0); }} />
          <div role="listbox" aria-label={t(`appearance.font.${role}`)} className="appearance-font-options">
            <button type="button" role="option" aria-selected={value.kind !== 'local'} className={`appearance-font-option ${FOCUS_ITEM}`} onClick={() => select(system)}>
              {value.kind !== 'local' ? <span className="appearance-font-check"><CheckGlyph /></span> : null}<span>{systemLabel}</span><FontSample font={system} role={role} sample={sample} />
            </button>
            {options.slice(page * PAGE_SIZE, (page + 1) * PAGE_SIZE).map(font => <button type="button" role="option" key={font.family} data-font-family={font.family} aria-selected={value.kind === 'local' && value.family === font.family} className={`appearance-font-option ${FOCUS_ITEM}`} onClick={() => select({kind:'local', family:font.family})}>
              {value.kind === 'local' && value.family === font.family ? <span className="appearance-font-check"><CheckGlyph /></span> : null}<span>{font.family}</span><FontSample font={{kind:'local',family:font.family}} role={role} sample={sample} />
            </button>)}
          </div>
          {catalogue.status === 'loading' || catalogue.status === 'idle' ? <p role="status">{t('appearance.fontLoading')}</p> : null}
          {catalogue.status === 'ready' && options.length === 0 ? <p role="status">{t('appearance.fontNoResults')}</p> : null}
          {options.length > PAGE_SIZE ? <div className="flex items-center justify-between gap-2"><Button size="xs" variant="secondary" disabled={page === 0} onClick={() => setPage(page - 1)}>{t('appearance.fontPrevious')}</Button><span>{page + 1} / {Math.ceil(options.length / PAGE_SIZE)}</span><Button size="xs" variant="secondary" disabled={(page + 1) * PAGE_SIZE >= options.length} onClick={() => setPage(page + 1)}>{t('appearance.fontNext')}</Button></div> : null}
        </div>} />
    </Field>
    <FontSample testId={`appearance-${role}-font-sample`} font={value} role={role} sample={sample} />
    {status ? <p role="status" className="text-xs text-ink-secondary">{t(`appearance.${status}`, {family:label, fallback:systemLabel})}</p> : null}
    {catalogue.status === 'failed' ? <Button size="xs" variant="secondary" disabled={disabled} onClick={() => void loadFontCatalogue(true)}>{t('appearance.fontRetry')}</Button> : null}
    {role === 'code' ? <p className="text-xs text-ink-secondary">{t('appearance.fontMonoHint')}</p> : null}
  </div>;
}

function SizeControl({ role, value, disabled, onChange }: { role: FontRole; value: number; disabled: boolean; onChange: (value: number) => void }) {
  const { t } = useLocale();
  const id = useId();
  const maximum = role === 'ui' ? 20 : 24;
  const [text, setText] = useState(String(value));
  useEffect(() => { setText(String(value)); }, [value]);
  const valid = /^\d+$/.test(text) && Number(text) >= 12 && Number(text) <= maximum;
  const label = t(`appearance.size.${role}`);
  return <div className="appearance-size-control">
    <label htmlFor={id}>{label}</label>
    <div className="appearance-size-inputs">
      <input id={id} aria-label={`${label} ${t('appearance.slider')}`} type="range" min={12} max={maximum} step={1} value={value} disabled={disabled} onChange={event => { const size = Number(event.target.value); setText(String(size)); onChange(size); }} />
      <Input data-testid={`appearance-${role}-size-input`} aria-label={`${label} px`} type="number" min={12} max={maximum} step={1} disabled={disabled} value={text} error={valid ? undefined : t('appearance.sizeRange', { min: 12, max: maximum })} onChange={event => {
        const next = event.target.value;
        setText(next);
        if (/^\d+$/.test(next) && Number(next) >= 12 && Number(next) <= maximum) onChange(Number(next));
      }} onBlur={() => { if (!valid) setText(String(value)); }} />
      <span>{t('appearance.px')}</span>
    </div>
  </div>;
}

export function AppearanceTypography({ value, disabled, onChange }: { value: Typography; disabled: boolean; onChange: (next: Typography) => void }) {
  const { t } = useLocale();
  const change = (patch: Partial<Typography>) => onChange({ ...value, ...patch });
  const row = (role: FontRole) => <div className="appearance-type-row" key={role}>
    <FontPicker role={role} value={value[`${role}Font`]} disabled={disabled} onChange={font => onChange({ ...value, [`${role}Font`]: font })} />
    <SizeControl role={role} value={value[`${role}Size`]} disabled={disabled} onChange={size => change({ [`${role}Size`]: size })} />
  </div>;
  const modeId = useId();
  return <section className="appearance-section" aria-labelledby="appearance-typography-heading"><Stack gap={4}>
    <h2 id="appearance-typography-heading" className="settings-section-title">{t('appearance.typography')}</h2>
    <p className="m-0 text-xs text-ink-secondary">{t('appearance.typographyHint')}</p>
    {row('ui')}{row('code')}
    <div className="appearance-reading-settings">
      <h3 className="appearance-reading-title">{t('appearance.readingTypography')}</h3>
      <div className="appearance-reading-controls">
        <div className="appearance-mode-control">
          <span id={modeId}>{t('appearance.readingMode')}</span>
          <RadioPrimitive.Root aria-labelledby={modeId} className="appearance-segments" value={value.readingFollowUi ? 'follow' : 'independent'} disabled={disabled} onValueChange={mode => change({ readingFollowUi: mode === 'follow' })}>
            <RadioPrimitive.Item value="follow" className="ui-focus">{t('appearance.followUi')}</RadioPrimitive.Item>
            <RadioPrimitive.Item value="independent" className="ui-focus">{t('appearance.independent')}</RadioPrimitive.Item>
          </RadioPrimitive.Root>
        </div>
        <Select label={t('appearance.lineHeight')} disabled={disabled} value={String(value.lineHeight)} options={[1.5, 1.75, 2].map(height => ({ value: String(height), label: String(height) }))} onChange={height => change({ lineHeight: Number(height) as Typography['lineHeight'] })} />
      </div>
      {value.readingFollowUi ? <p className="m-0 text-xs text-ink-secondary">{t('appearance.followHint')}</p> : row('reading')}
    </div>
  </Stack></section>;
}
