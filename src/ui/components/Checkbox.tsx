import { Checkbox as CheckboxPrimitive } from 'radix-ui';
import { useId } from 'react';
import { cn } from '../lib/cn';
import { FOCUS_RING } from '../lib/focus';
import { accessibleName, hasVisibleLabel, type Named } from '../lib/props';
import { CheckGlyph, MinusGlyph } from './glyphs';

export type CheckboxProps = Named & {
  /** `'indeterminate'` is the mixed state (`aria-checked="mixed"`). */
  checked: boolean | 'indeterminate';
  onChange: (checked: boolean) => void;
  disabled?: boolean;
  id?: string;
  name?: string;
  className?: string;
};

/** Radix Checkbox (D066). A click toggles; from `indeterminate` the next value is `true`. */
export function Checkbox(props: CheckboxProps) {
  const { checked, onChange, disabled, id, name, className, label } = props;
  const autoId = useId();
  const fieldId = id ?? autoId;
  return (
    <div className={cn('inline-flex items-center gap-2', className)}>
      <CheckboxPrimitive.Root
        id={fieldId}
        name={name}
        checked={checked}
        disabled={disabled}
        onCheckedChange={(next) => onChange(next === true)}
        {...accessibleName(props)}
        className={cn(
          'flex size-4 shrink-0 items-center justify-center rounded-sm border border-input bg-background text-primary-foreground',
          'data-[state=checked]:border-primary data-[state=checked]:bg-primary',
          'data-[state=indeterminate]:border-primary data-[state=indeterminate]:bg-primary',
          'disabled:cursor-not-allowed disabled:opacity-disabled',
          FOCUS_RING,
        )}
      >
        <CheckboxPrimitive.Indicator>
          {checked === 'indeterminate' ? <MinusGlyph /> : <CheckGlyph />}
        </CheckboxPrimitive.Indicator>
      </CheckboxPrimitive.Root>
      {hasVisibleLabel(label) ? (
        <label
          htmlFor={fieldId}
          className={cn('text-sm text-foreground', disabled ? 'cursor-not-allowed opacity-disabled' : 'cursor-pointer')}
        >
          {label}
        </label>
      ) : null}
    </div>
  );
}
