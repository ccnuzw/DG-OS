import { RadioGroup as RadioGroupPrimitive } from 'radix-ui';
import { useId } from 'react';
import { cn } from '../lib/cn';
import { FOCUS_RING } from '../lib/focus';
import { accessibleName, hasVisibleLabel, type Named } from '../lib/props';

export interface RadioOption {
  value: string;
  label: string;
  disabled?: boolean;
}

export type RadioGroupProps = Named & {
  options: readonly RadioOption[];
  /** Controlled value. An empty string means nothing is selected. */
  value: string;
  onChange: (value: string) => void;
  disabled?: boolean;
  name?: string;
  className?: string;
};

/**
 * Radix RadioGroup (D066): `role="radiogroup"`, one tab stop, arrow keys move the selection inside the group.
 * The group name is the visible `label`, or `aria-label`/`aria-labelledby` when there is none.
 */
export function RadioGroup(props: RadioGroupProps) {
  const { options, value, onChange, disabled, name, className, label } = props;
  const groupId = useId();
  const labelId = hasVisibleLabel(label) ? `${groupId}-label` : undefined;
  return (
    <div className="flex flex-col gap-2">
      {labelId ? (
        <span id={labelId} className="text-sm font-medium text-foreground">
          {label}
        </span>
      ) : null}
      <RadioGroupPrimitive.Root
        value={value}
        onValueChange={onChange}
        disabled={disabled}
        name={name}
        {...accessibleName(props)}
        aria-labelledby={labelId ?? accessibleName(props)['aria-labelledby']}
        className={cn('flex flex-col gap-2', className)}
      >
        {options.map((option) => {
          const itemId = `${groupId}-${option.value}`;
          const itemDisabled = disabled || option.disabled;
          return (
            <div key={option.value} className="inline-flex items-center gap-2">
              <RadioGroupPrimitive.Item
                id={itemId}
                value={option.value}
                disabled={option.disabled}
                className={cn(
                  'flex size-4 shrink-0 items-center justify-center rounded-full border border-input bg-background',
                  'data-[state=checked]:border-primary',
                  'disabled:cursor-not-allowed disabled:opacity-disabled',
                  FOCUS_RING,
                )}
              >
                <RadioGroupPrimitive.Indicator className="size-2 rounded-full bg-primary" />
              </RadioGroupPrimitive.Item>
              <label
                htmlFor={itemId}
                className={cn('text-sm text-foreground', itemDisabled ? 'cursor-not-allowed opacity-disabled' : 'cursor-pointer')}
              >
                {option.label}
              </label>
            </div>
          );
        })}
      </RadioGroupPrimitive.Root>
    </div>
  );
}
