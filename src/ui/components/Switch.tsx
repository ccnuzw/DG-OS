import { Switch as SwitchPrimitive } from 'radix-ui';
import { useId } from 'react';
import { cn } from '../lib/cn';
import { FOCUS_RING } from '../lib/focus';
import { accessibleName, hasVisibleLabel, type Named } from '../lib/props';

export type SwitchProps = Named & {
  checked: boolean;
  onChange: (checked: boolean) => void;
  disabled?: boolean;
  id?: string;
  name?: string;
  className?: string;
};

/** Radix Switch (D066): `role="switch"`, `aria-checked` follows `checked`, Space toggles. */
export function Switch(props: SwitchProps) {
  const { checked, onChange, disabled, id, name, className, label } = props;
  const autoId = useId();
  const fieldId = id ?? autoId;
  return (
    <div className={cn('inline-flex items-center gap-2', className)}>
      <SwitchPrimitive.Root
        id={fieldId}
        name={name}
        checked={checked}
        disabled={disabled}
        onCheckedChange={onChange}
        {...accessibleName(props)}
        className={cn(
          'control-hit-target inline-flex h-5 w-9 shrink-0 appearance-none items-center rounded-full border border-solid border-edge-control bg-surface-inset p-0 transition-colors motion-reduce:transition-none',
          'data-[state=checked]:border-accent-default data-[state=checked]:bg-accent-default',
          'disabled:cursor-not-allowed disabled:opacity-disabled',
          FOCUS_RING,
        )}
      >
        <SwitchPrimitive.Thumb
          className={cn(
            'block size-3.5 translate-x-0.5 rounded-full bg-ink-primary transition-transform motion-reduce:transition-none',
            'data-[state=checked]:translate-x-4 data-[state=checked]:bg-accent-on',
          )}
        />
      </SwitchPrimitive.Root>
      {hasVisibleLabel(label) ? (
        <label
          htmlFor={fieldId}
          className={cn('text-sm text-ink-primary', disabled ? 'cursor-not-allowed opacity-disabled' : 'cursor-pointer')}
        >
          {label}
        </label>
      ) : null}
    </div>
  );
}
