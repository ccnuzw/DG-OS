import { useId, type ReactNode } from 'react';
import { FOCUS_RING } from '../lib/focus';
import { hasVisibleLabel } from '../lib/props';

/** Ids shared by a labelled control and its error text. `describedBy` merges a caller's own value with the error. */
export function useFieldIds(id: string | undefined, error: string | undefined, describedBy: string | undefined) {
  const autoId = useId();
  const fieldId = id ?? autoId;
  const errorId = error ? `${fieldId}-error` : undefined;
  const merged = [describedBy, errorId].filter(Boolean).join(' ') || undefined;
  return { fieldId, errorId, describedBy: merged };
}

/** Visible label above the control, error text below it (AC06, AC27). */
export function Field({
  id,
  label,
  error,
  errorId,
  children,
}: {
  id: string;
  label?: ReactNode;
  error?: string;
  errorId?: string;
  children: ReactNode;
}) {
  return (
    <div className="flex w-full flex-col gap-1">
      {hasVisibleLabel(label) ? (
        <label htmlFor={id} className="text-sm font-medium text-ink-primary">
          {label}
        </label>
      ) : null}
      {children}
      {error ? (
        <p id={errorId} className="text-sm text-status-error-text">
          {error}
        </p>
      ) : null}
    </div>
  );
}

/** Border colour by state: error wins over success. All three pair with the page background at >= 3:1 (AC20). */
export function controlBorder(error: string | undefined, success: boolean): string {
  if (error) return 'border-status-error';
  if (success) return 'border-status-success';
  return 'border-edge-control';
}

/** Control geometry and states, without a default typography role. */
export const CONTROL_SURFACE_BASE = [
  'control-text-fit w-full appearance-none rounded-md border border-solid bg-surface-field px-3 text-ink-primary',
  'placeholder:text-ink-secondary',
  'disabled:cursor-not-allowed disabled:opacity-disabled',
  FOCUS_RING,
].join(' ');

/** Default UI typography shared by Input and the Select trigger. */
export const CONTROL_SURFACE = `${CONTROL_SURFACE_BASE} font-sans text-sm`;
