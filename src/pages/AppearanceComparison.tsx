import { useId, useLayoutEffect, useRef, useState } from 'react';
import { Button, Icon, Input, StatusBadge } from '@dgos/ui';
import Markdown from '../components/chat/Markdown';
import { useLocale } from '../i18n';
import { effectiveColors } from '../ui/appearance/colors';
import { DEFAULTS, type Appearance, type ResolvedTheme } from '../ui/appearance/model';
import { FOCUS_RING } from '../ui/lib/focus';
import { applyAppearanceSample, applyColorPairSample } from '../ui/theme/apply';
import { useFontCatalogue } from '../ui/theme/fontCatalogue';
import { themeColorIssues, type ThemeColorIssue, type ThemeScope } from '../ui/theme/kernel';
import './appearance-comparison.css';

const MODES = ['light', 'dark'] as const;

/** Invalid colors are confined to this pair, never the complete control sample. */
function ColorPair({ draft, mode, scope }: { draft: Appearance; mode: ResolvedTheme; scope: ThemeScope }) {
  const { t } = useLocale();
  const sample = useRef<HTMLDivElement>(null);
  const pairScope = scope === 'code' ? 'code' : 'reading';
  const colors = effectiveColors(draft.variants[mode], mode, scope);
  const complete = [colors.background, colors.foreground].every(color => /^#[0-9a-fA-F]{6}$/.test(color));
  useLayoutEffect(() => {
    if (complete && sample.current) applyColorPairSample(sample.current, colors.background, colors.foreground, pairScope);
  }, [complete, colors.background, colors.foreground, pairScope]);
  if (!complete) return null;
  return <div className="appearance-comparison-pair">
    <p className="appearance-comparison-pair-label">{t(`appearance.scope.${scope}`)}</p>
    <div ref={sample} className="appearance-comparison-pair-scope">
      {pairScope === 'code'
        ? <pre className="type-code">{t('appearance.comparisonCode')}</pre>
        : <p className="type-reading">{t('appearance.comparisonPair')}</p>}
    </div>
  </div>;
}

function ModeSample({ draft, mode }: { draft: Appearance; mode: ResolvedTheme }) {
  const { t } = useLocale();
  const catalogue = useFontCatalogue();
  const titleId = useId();
  const errorId = useId();
  const sample = useRef<HTMLDivElement>(null);
  const initialized = useRef(false);
  const [invalid, setInvalid] = useState(false);
  const variant = draft.variants[mode];
  const issues: readonly ThemeColorIssue[] = themeColorIssues(variant.presetId, mode, variant.overrides);
  const pairScopes = [...new Set(issues.filter(issue => issue.kind === 'textContrast').map(issue => issue.scope))];
  const modeLabel = t(mode === 'light' ? 'appearance.themeLight' : 'appearance.themeDark');

  useLayoutEffect(() => {
    if (!sample.current) return;
    try {
      // The helper validates this mode independently and mutates only after full validation.
      applyAppearanceSample(sample.current, draft, mode);
      initialized.current = true;
      setInvalid(false);
    } catch {
      // Establish a complete baseline if no valid draft has been shown yet.
      if (!initialized.current) {
        applyAppearanceSample(sample.current, DEFAULTS, mode);
        initialized.current = true;
      }
      // Keep every token and attribute on the last complete valid sample untouched.
      setInvalid(true);
    }
  }, [draft, mode, catalogue]);

  const explainIssue = (issue: ThemeColorIssue) => t('appearance.colorValidation', {
    mode: modeLabel,
    scope: t(`appearance.scope.${issue.scope}`),
    field: issue.fields.map(field => t(`appearance.color.${field}`)).join(' / '),
    reason: t(`appearance.colorValidation.${issue.kind}`),
  });

  return <section className="appearance-comparison-mode" aria-labelledby={titleId} aria-describedby={invalid ? errorId : undefined}>
    <h3 id={titleId} className="appearance-comparison-mode-title">{modeLabel}</h3>
    <div ref={sample} className="appearance-comparison-scope" data-appearance-sample={mode} role="group" aria-label={t('appearance.colorSample', { mode: modeLabel })}>
      <nav className="appearance-comparison-nav" aria-label={t('appearance.colorSample', { mode: modeLabel })}>
        <Button variant="ghost" className="appearance-comparison-nav-item" aria-current="page"><Icon name="palette" size="sm" /><span>{t('settings.appearance')}</span></Button>
        <Button variant="ghost" className="appearance-comparison-nav-item"><Icon name="file" size="sm" /><span>{t('settings.about')}</span></Button>
      </nav>
      <div className="appearance-comparison-content">
        <div className="appearance-comparison-assistant"><Icon name="code" size="sm" /><span>{t('chat.assistant')}</span></div>
        <Markdown source={t('appearance.comparisonReading')} />
        <pre className={`appearance-comparison-code type-code ${FOCUS_RING}`} tabIndex={0} aria-label={t('appearance.scope.code')}>{t('appearance.comparisonCode')}</pre>
        <StatusBadge status="success" label={t('appearance.noticeText')} />
        <Input label={t('appearance.inputExample')} className="type-reading appearance-comparison-input" placeholder={t('appearance.comparisonInput')} />
        <div className="appearance-comparison-actions">
          <Button>{t('appearance.primaryButton')}</Button>
          <Button variant="secondary">{t('appearance.secondaryButton')}</Button>
        </div>
      </div>
    </div>
    {invalid ? <div id={errorId} className="appearance-comparison-errors" role="status">
      <p>{t('appearance.error.invalid')}</p>
      {issues.map((issue, index) => <p key={`${issue.scope}:${issue.kind}:${index}`}>{explainIssue(issue)}</p>)}
    </div> : null}
    {invalid ? pairScopes.map(scope => <ColorPair key={scope} draft={draft} mode={mode} scope={scope} />) : null}
  </section>;
}

export function AppearanceComparison({ draft }: { draft: Appearance }) {
  const { t } = useLocale();
  const titleId = useId();
  return <section className="appearance-comparison" aria-labelledby={titleId}>
    <header className="appearance-comparison-header">
      <h3 id={titleId}>{t('appearance.comparisonTitle')}</h3>
      <p>{t('appearance.comparisonHint')}</p>
    </header>
    <div className="appearance-comparison-grid">
      {MODES.map(mode => <ModeSample key={mode} draft={draft} mode={mode} />)}
    </div>
  </section>;
}
