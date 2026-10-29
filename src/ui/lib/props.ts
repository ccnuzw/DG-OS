import type { ComponentPropsWithRef, ElementType, ReactNode } from 'react';

/** Props of a DOM element without `style`: components never accept inline styles (D068 R1, FR-005 §7.2.1). */
export type DomProps<T extends ElementType> = Omit<ComponentPropsWithRef<T>, 'style'>;

/**
 * Accessible name of a form control (D069): the visible `label` wins; `aria-label` or `aria-labelledby` is
 * accepted only when there is no visible text. The three members are mutually exclusive, so "no name at all"
 * and "a name that can disagree with the visible text" are both type errors. Untyped callers are handled by
 * `accessibleName` at runtime.
 */
export type Named =
  | { label: ReactNode; 'aria-label'?: undefined; 'aria-labelledby'?: undefined }
  | { label?: undefined; 'aria-label': string; 'aria-labelledby'?: undefined }
  | { label?: undefined; 'aria-label'?: undefined; 'aria-labelledby': string };

/** True when `label` renders visible text. `0` counts; `undefined`, `null`, booleans and `''` do not. */
export function hasVisibleLabel(label: ReactNode): boolean {
  return label !== undefined && label !== null && typeof label !== 'boolean' && label !== '';
}

/**
 * The ARIA naming attributes a control may carry (D069). A visible label names the control (through `<label>` or
 * the component's own `aria-labelledby`), so both attributes are dropped: an `aria-label` would override that text
 * (WCAG 2.5.3). Without a visible label, `aria-labelledby` wins over `aria-label`, as in the accname computation.
 */
export function accessibleName(props: {
  label?: ReactNode;
  'aria-label'?: string;
  'aria-labelledby'?: string;
}): { 'aria-label'?: string; 'aria-labelledby'?: string } {
  if (hasVisibleLabel(props.label)) return {};
  if (props['aria-labelledby']) return { 'aria-labelledby': props['aria-labelledby'] };
  if (props['aria-label']) return { 'aria-label': props['aria-label'] };
  return {};
}

/**
 * Runtime guard for callers without types (module code is plain JS, D067): drops a `style` prop before the
 * rest is spread onto a DOM element. The type system already forbids it; this keeps the promise at runtime.
 */
export function omitStyle<T extends object>(props: T): T {
  const { style: _dropped, ...rest } = props as T & { style?: unknown };
  return rest as T;
}
