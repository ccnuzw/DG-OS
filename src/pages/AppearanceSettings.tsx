import { useEffect, useRef, useState } from 'react';
import {
  Button, Card, DENSITIES, Input, RadioGroup, Stack, THEMES, toast,
  getCurrentDensity, getCurrentTheme, initAppearance, updateAppearance,
  type Density, type Theme,
} from '@dgos/ui';
import { usePageError } from '../pageError';
import { LANGUAGE_PREFERENCES, initLocale, updateLanguage, useLocale, type LanguagePreference } from '../i18n';

// V1-FR-007 §7.3 (D071). The page only calls the FR-005 appearance controller: it applies a choice at once,
// saves it through `set_appearance_preferences`, and on a failed save rolls back and raises the error Toast
// itself (AC21). The page therefore never reports a write failure a second time; it only makes the controls
// show the value the controller ends up with.

const DENSITY_OPTIONS = DENSITIES.map(value => ({ value: String(value), label: `${value}%` }));
const LANGUAGE_OPTIONS: Array<{ value: LanguagePreference; labelKey: 'languages.zhCN' | 'languages.enUS' | 'languages.system' }> = [
  { value: 'zh-CN', labelKey: 'languages.zhCN' },
  { value: 'en-US', labelKey: 'languages.enUS' },
  { value: 'system', labelKey: 'languages.system' },
];

export default function AppearanceSettings() {
  const reportError = usePageError('AppearanceSettings');
  const { preference, t } = useLocale();
  const themeOptions = THEMES.map(value => ({
    value,
    label: t(value === 'light' ? 'appearance.themeLight' : value === 'dark' ? 'appearance.themeDark' : 'appearance.themeAuto'),
  }));
  const [theme, setTheme] = useState<Theme>(getCurrentTheme);
  const [density, setDensity] = useState<Density>(getCurrentDensity);
  const mounted = useRef(true);

  // The controller's getters return the value now applied. Once its write queue is idle that is the host's
  // confirmed value (after a failed save, the rolled-back one), and while a later change is still queued it
  // already includes that change, so reading here never moves a control backwards.
  const settle = () => {
    if (!mounted.current) return;
    setTheme(getCurrentTheme());
    setDensity(getCurrentDensity());
  };

  // `main.tsx` does not wait for the first read, so on a full page load this component can render before the
  // host's values arrive and its initial state would be the defaults. `initAppearance` is memoised and never
  // rejects (D065), so waiting on it here only picks up the values once they are known.
  useEffect(() => {
    mounted.current = true;
    void Promise.all([initAppearance(), initLocale()]).then(settle);
    return () => { mounted.current = false; };
  }, []);

  const save = (patch: { theme: Theme } | { density: Density }) => {
    // `updateAppearance` handles write failures itself; a rejection here is unexpected, so it goes to the shared reporter.
    updateAppearance(patch).catch(reportError).then(settle);
  };

  // B07-12
  const changeTheme = (value: string) => {
    const next = THEMES.find(candidate => candidate === value);
    if (!next) return;
    setTheme(next);
    save({ theme: next });
  };

  // B07-13: interface scale (D064 folded the old font size and compact mode into it)
  const changeDensity = (value: string) => {
    const next = DENSITIES.find(candidate => String(candidate) === value);
    if (next === undefined) return;
    setDensity(next);
    save({ density: next });
  };

  const changeLanguage = (value: string) => {
    if (!(LANGUAGE_PREFERENCES as readonly string[]).includes(value)) return;
    void updateLanguage(value as LanguagePreference).catch(reportError);
  };

  return (
    <div className="mx-auto flex max-w-2xl flex-col gap-6 p-6">
      <h1 className="m-0 text-2xl font-semibold text-foreground">{t('appearance.title')}</h1>
      <Card>
        <Card.Body>
          <Stack gap={6}>
            <RadioGroup label={t('appearance.theme')} options={themeOptions} value={theme} onChange={changeTheme} />
            <RadioGroup label={t('appearance.density')} options={DENSITY_OPTIONS} value={String(density)} onChange={changeDensity} />
            <RadioGroup
              label={t('appearance.language')}
              options={LANGUAGE_OPTIONS.map(option => ({ value: option.value, label: t(option.labelKey) }))}
              value={preference}
              onChange={changeLanguage}
            />
          </Stack>
        </Card.Body>
      </Card>
      <Card>
        <Card.Header>
          <h2 className="m-0 text-base font-semibold">{t('appearance.preview')}</h2>
        </Card.Header>
        <Card.Body>
          <Stack gap={4}>
            <Stack gap={3} direction="horizontal">
              <Button variant="primary">{t('appearance.primaryButton')}</Button>
              <Button variant="secondary">{t('appearance.secondaryButton')}</Button>
            </Stack>
            <Input label={t('appearance.inputExample')} placeholder={t('appearance.inputExample')} />
            <div>
              <Button variant="secondary" onClick={() => toast.info(t('appearance.noticeText'))}>{t('appearance.showNotice')}</Button>
            </div>
          </Stack>
        </Card.Body>
      </Card>
    </div>
  );
}
