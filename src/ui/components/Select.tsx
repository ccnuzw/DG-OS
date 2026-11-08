import { Select as SelectPrimitive } from 'radix-ui';
import { useId } from 'react';
import { cn } from '../lib/cn';
import { FOCUS_ITEM } from '../lib/focus';
import { accessibleName, type Named } from '../lib/props';
import { CONTROL_SURFACE, Field, controlBorder, useFieldIds } from './Field';
import { CheckGlyph, ChevronGlyph } from './glyphs';

export interface SelectOption {
  value: string;
  label: string;
}

export type SelectProps = Named & {
  options: readonly SelectOption[];
  /** Controlled value. An empty string or a value not in `options` shows the placeholder. */
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  error?: string;
  success?: boolean;
  disabled?: boolean;
  id?: string;
  name?: string;
  className?: string;
};

/**
 * Radix Select (D066). Positioning, typeahead and focus handling come from the primitive; jsdom can verify the
 * roles and the value contract but not the popper geometry (see tests/ui-controls.test.tsx header).
 */
export function Select(props: SelectProps) {
  const { options, value, onChange, placeholder, error, success = false, disabled, id, name, className, label } = props;
  const ids = useFieldIds(id, error, undefined);
  // Radix keeps showing nothing for a value without an item. '' is the only value it renders as the placeholder,
  // so a value that is not (or is no longer) in `options` is shown as empty; no other option is selected for it.
  const selected = options.find((option) => option.value === value);
  const shown = selected ? value : '';
  const valueId = `${ids.fieldId}-value`;
  const listId = useId();
  return (
    <Field id={ids.fieldId} label={label} error={error} errorId={ids.errorId}>
      <SelectPrimitive.Root value={shown} onValueChange={onChange} disabled={disabled} name={name}>
        <SelectPrimitive.Trigger
          id={ids.fieldId}
          {...accessibleName(props)}
          aria-invalid={error ? true : undefined}
          aria-describedby={[selected ? valueId : undefined, ids.describedBy].filter(Boolean).join(' ') || undefined}
          title={selected?.label}
          className={cn(
            CONTROL_SURFACE,
            'flex h-(--control-height-compact) min-w-0 items-center justify-between gap-2 text-left',
            'data-[placeholder]:text-ink-secondary',
            controlBorder(error, success),
            className,
          )}
        >
          <span className="min-w-0 flex-1 truncate">
            <SelectPrimitive.Value id={valueId} placeholder={placeholder} />
          </span>
          <SelectPrimitive.Icon className="shrink-0">
            <ChevronGlyph />
          </SelectPrimitive.Icon>
        </SelectPrimitive.Trigger>
        <SelectPrimitive.Portal>
          <SelectPrimitive.Content
            id={listId}
            position="popper"
            sideOffset={4}
            className="z-popover w-(--radix-select-trigger-width) max-h-(--radix-select-content-available-height) overflow-hidden rounded-md border border-border-subtle bg-surface-popover text-ink-primary shadow-md"
          >
            <SelectPrimitive.Viewport className="p-1">
              {options.map((option) => (
                <SelectPrimitive.Item
                  key={option.value}
                  value={option.value}
                  className={cn(
                    'relative flex cursor-pointer select-none items-center rounded-sm py-1.5 pl-8 pr-3 text-sm',
                    'data-[highlighted]:bg-surface-hover data-[highlighted]:text-ink-primary',
                    FOCUS_ITEM,
                  )}
                >
                  <SelectPrimitive.ItemIndicator className="absolute left-2 inline-flex items-center">
                    <CheckGlyph />
                  </SelectPrimitive.ItemIndicator>
                  <SelectPrimitive.ItemText><span className="wrap-anywhere">{option.label}</span></SelectPrimitive.ItemText>
                </SelectPrimitive.Item>
              ))}
            </SelectPrimitive.Viewport>
          </SelectPrimitive.Content>
        </SelectPrimitive.Portal>
      </SelectPrimitive.Root>
    </Field>
  );
}
