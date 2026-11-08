import { useEffect, useId, useRef, useState } from 'react';
import { HexColorPicker } from 'react-colorful';
import { useLocale } from '../../i18n';
import { Button } from '../components/Button';
import { Input } from '../components/Input';
import { Popover } from '../components/Popover';
import { Switch } from '../components/Switch';
import { PRESETS, PRESET_IDS } from '../theme/kernel';
import './color-field.css';

export interface ColorFieldProps {
  label: string;
  value: string;
  override?: string;
  recommendations: readonly string[];
  disabled?: boolean;
  error?: string;
  onChange: (value: string | undefined) => void;
  onValidityChange?: (invalid: boolean) => void;
}

function normalizeHex(raw: string): string | undefined {
  const hex = raw.trim();
  return /^#?[\da-f]{6}$/i.test(hex) ? `#${hex.replace(/^#/, '').toUpperCase()}` : undefined;
}

// Reuse the token palette, including neutral seeds, rather than defining another palette.
const COMMON_COLORS = [...new Set([
  ...PRESET_IDS.flatMap(id => [PRESETS[id].light.accent, PRESETS[id].dark.accent]),
  PRESETS.precision.light.background,
  PRESETS.precision.light.foreground,
  PRESETS.precision.dark.background,
  PRESETS.precision.dark.foreground,
].map(color => color.toUpperCase()))];

function Swatch({ color }: { color: string }) {
  return <svg className="appearance-color-swatch" viewBox="0 0 24 24" aria-hidden="true" focusable="false">
    <rect x="1" y="1" width="22" height="22" rx="3" fill={color} />
  </svg>;
}

export function ColorField({ label, value, override, recommendations, disabled = false, error, onChange, onValidityChange }: ColorFieldProps) {
  const { t } = useLocale();
  const id = useId();
  const effective = value.toUpperCase();
  const [raw, setRaw] = useState(effective);
  const [open, setOpen] = useState(false);
  const [pickerRaw, setPickerRaw] = useState(effective);
  const [pickerColor, setPickerColor] = useState(effective);
  const trigger = useRef<HTMLButtonElement>(null);
  const snapshot = useRef({ override, value: effective, raw: effective });
  const published = useRef<string | undefined>(undefined);
  const validityCallback = useRef(onValidityChange);
  validityCallback.current = onValidityChange;
  const invalid = !normalizeHex(raw);
  const pickerInvalid = open && !normalizeHex(pickerRaw);
  const hintId = `${id}-picker-hint`;
  const pickerLabel = `${label} · ${t('appearance.colorPicker')}`;

  useEffect(() => {
    // A valid local edit echoes through the parent. It must not replace newer,
    // incomplete text typed while that update was being rendered.
    const isEcho = published.current === override?.toUpperCase() && published.current === effective;
    published.current = undefined;
    if (isEcho) return;
    setRaw(effective);
    setPickerRaw(effective);
    setPickerColor(effective);
  }, [effective, override]);

  useEffect(() => {
    validityCallback.current?.(invalid || pickerInvalid);
  }, [invalid, pickerInvalid]);
  useEffect(() => () => { validityCallback.current?.(false); }, []);

  function publish(color: string) {
    published.current = color;
    onChange(color);
  }

  function selectColor(color: string) {
    const normalized = normalizeHex(color);
    if (!normalized) return;
    setPickerRaw(normalized);
    setPickerColor(normalized);
    setRaw(normalized);
    publish(normalized);
  }

  function changeOpen(next: boolean) {
    if (next) {
      if (disabled) return;
      snapshot.current = { override, value: effective, raw };
      setPickerRaw(effective);
      setPickerColor(effective);
    } else {
      // Outside and Done retain only the last valid choice.
      setPickerRaw(pickerColor);
    }
    setOpen(next);
  }

  function cancelPicker() {
    const original = snapshot.current;
    published.current = undefined;
    setRaw(original.raw);
    setPickerRaw(original.value);
    setPickerColor(original.value);
    onChange(original.override === undefined ? undefined : normalizeHex(original.override));
    setOpen(false);
  }

  function finishField() {
    const normalized = normalizeHex(raw);
    if (normalized) setRaw(normalized);
  }

  function finishPicker() {
    const normalized = normalizeHex(pickerRaw);
    if (normalized) setPickerRaw(normalized);
  }

  const recommended = [...new Set(recommendations.map(normalizeHex).filter((color): color is string => color !== undefined))];
  function palette(colors: readonly string[], heading: string, suffix: string) {
    return <section className="appearance-color-palette" aria-labelledby={`${id}-${suffix}`}>
      <h3 id={`${id}-${suffix}`}>{heading}</h3>
      <div className="appearance-color-choices">
        {colors.map(color => {
          const selected = pickerColor === color;
          return <Button key={color} variant="ghost" className="appearance-color-choice" disabled={disabled}
            aria-label={selected ? `${color} · ${t('appearance.colorSelected')}` : color}
            aria-pressed={selected} title={color} onClick={() => selectColor(color)}>
            <Swatch color={color} />
            <span className="appearance-color-check" aria-hidden="true">{selected ? '✓' : '\u00a0'}</span>
          </Button>;
        })}
      </div>
    </section>;
  }

  return <div className="appearance-color-field">
    <div className="appearance-color-field-main">
      <div className="appearance-color-label-row">
        <label id={`${id}-color-name`} className="appearance-color-label" htmlFor={`${id}-hex`}>{label}</label>
        <div className="inline-flex items-center gap-2">
          <Switch id={`${id}-custom`} checked={override !== undefined || invalid} disabled={disabled} aria-labelledby={`${id}-color-name ${id}-custom-name`} onChange={checked => {
            published.current = undefined;
            setRaw(effective);
            onChange(checked ? effective : undefined);
          }} />
          <label id={`${id}-custom-name`} htmlFor={`${id}-custom`}>{t('appearance.customColor')}</label>
        </div>
      </div>
      <Popover label={pickerLabel} align="start" collisionPadding={12} open={open} onOpenChange={changeOpen}
        onEscapeKeyDown={event => {
          event.preventDefault();
          event.stopPropagation();
          cancelPicker();
        }}
        onCloseAutoFocus={event => {
          event.preventDefault();
          trigger.current?.focus();
        }}
        trigger={<Button ref={trigger} variant="secondary" iconOnly disabled={disabled}
          aria-label={pickerLabel} className="appearance-color-trigger"><Swatch color={effective} /></Button>}
        content={<div className="appearance-color-picker" aria-busy={disabled || undefined}>
          {palette(recommended, t('appearance.colorRecommendations'), 'recommendations')}
          {palette(COMMON_COLORS, t('appearance.colorCommon'), 'common')}
          <fieldset className="appearance-color-plane" disabled={disabled} inert={disabled} aria-label={pickerLabel}>
            <HexColorPicker color={pickerColor} onChange={selectColor} aria-label={pickerLabel} />
          </fieldset>
          <Input label={`${label} HEX`} value={pickerRaw} disabled={disabled} spellCheck={false} autoComplete="off"
            aria-describedby={hintId} error={pickerInvalid ? t('appearance.colorInvalidHex') : undefined}
            onChange={event => {
              const next = event.target.value;
              setPickerRaw(next);
              const normalized = normalizeHex(next);
              if (normalized) {
                setPickerColor(normalized);
                setRaw(normalized);
                publish(normalized);
              }
            }} onBlur={finishPicker} onKeyDown={event => {
              if (event.key === 'Enter' && !event.nativeEvent.isComposing) {
                event.preventDefault();
                finishPicker();
              }
            }} />
          <p className="appearance-color-hint" id={hintId}>{t('appearance.colorPickerHint')}</p>
          <div className="appearance-color-picker-actions">
            <Button variant="secondary" disabled={disabled} onClick={cancelPicker}>{t('appearance.colorPickerCancel')}</Button>
            <Button disabled={disabled} onClick={() => changeOpen(false)}>{t('appearance.colorPickerDone')}</Button>
          </div>
        </div>} />
      <div className="appearance-color-hex">
        <Input id={`${id}-hex`} aria-label={`${label} HEX`} value={raw} disabled={disabled} spellCheck={false} autoComplete="off"
          error={invalid ? t('appearance.colorInvalidHex') : error}
          onChange={event => {
            const next = event.target.value;
            setRaw(next);
            const normalized = normalizeHex(next);
            if (normalized) publish(normalized);
          }} onBlur={finishField} onKeyDown={event => {
            if (event.key === 'Enter' && !event.nativeEvent.isComposing) {
              event.preventDefault();
              finishField();
            }
          }} />
      </div>
    </div>
  </div>;
}
