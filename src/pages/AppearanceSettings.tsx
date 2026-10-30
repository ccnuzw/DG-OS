import { useEffect, useRef, useState } from 'react';
import {
  Button, Card, DENSITIES, Input, RadioGroup, Stack, THEMES, toast,
  getCurrentDensity, getCurrentTheme, initAppearance, updateAppearance,
  type Density, type Theme,
} from '@dgos/ui';
import { usePageError } from '../pageError';

// V1-FR-007 §7.3 (D071). The page only calls the FR-005 appearance controller: it applies a choice at once,
// saves it through `set_appearance_preferences`, and on a failed save rolls back and raises the error Toast
// itself (AC21). The page therefore never reports a write failure a second time; it only makes the controls
// show the value the controller ends up with.

const THEME_LABELS: Record<Theme, string> = { light: '亮色', dark: '暗色', auto: '跟随系统' };
const THEME_OPTIONS = THEMES.map(value => ({ value, label: THEME_LABELS[value] }));
const DENSITY_OPTIONS = DENSITIES.map(value => ({ value: String(value), label: `${value}%` }));

export default function AppearanceSettings() {
  const reportError = usePageError('AppearanceSettings');
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
    void initAppearance().then(settle);
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

  return (
    <div className="mx-auto flex max-w-2xl flex-col gap-6 p-6">
      <h1 className="m-0 text-2xl font-semibold text-foreground">外观设置</h1>
      <Card>
        <Card.Body>
          <Stack gap={6}>
            <RadioGroup label="主题" options={THEME_OPTIONS} value={theme} onChange={changeTheme} />
            <RadioGroup label="界面缩放" options={DENSITY_OPTIONS} value={String(density)} onChange={changeDensity} />
          </Stack>
        </Card.Body>
      </Card>
      <Card>
        <Card.Header>
          <h2 className="m-0 text-base font-semibold">预览</h2>
        </Card.Header>
        <Card.Body>
          <Stack gap={4}>
            <Stack gap={3} direction="horizontal">
              <Button variant="primary">主要按钮</Button>
              <Button variant="secondary">次要按钮</Button>
            </Stack>
            <Input label="输入框示例" placeholder="输入框示例" />
            <div>
              <Button variant="secondary" onClick={() => toast.info('这是一条示例通知')}>显示通知</Button>
            </div>
          </Stack>
        </Card.Body>
      </Card>
    </div>
  );
}
