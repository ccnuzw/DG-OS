// Toast queue (FR-005 B05-13, B05-14, B05-15, B05-16, B05-18, B05-21).
// The queue lives outside React and is read with `useSyncExternalStore` (ToastViewport). The store owns *what*
// is shown and in which order; the viewport owns *when* a visible toast closes (timers, pause, fade-out), then
// calls `removeToast`, which promotes the next queued toast.

export type ToastType = 'success' | 'error' | 'warning' | 'info';

export interface ToastAction {
  label: string;
  onClick: () => void;
}

export interface ToastOptions {
  description?: string;
  /** Milliseconds before the fade-out starts. Defaults: 5000 for `error`, 3000 otherwise. */
  duration?: number;
  action?: ToastAction;
}

export interface ToastItem {
  id: string;
  type: ToastType;
  message: string;
  description?: string;
  duration: number;
  action?: ToastAction;
}

export interface ToastSnapshot {
  visible: readonly ToastItem[];
  queued: readonly ToastItem[];
}

/** At most this many toasts are on screen; the rest wait in the queue (AC17). */
export const MAX_VISIBLE = 3;
export const DEFAULT_DURATION = 3000;
export const ERROR_DURATION = 5000;
/** Fade-out length; the viewport removes the toast this long after it starts closing (AC09). */
export const FADE_OUT_MS = 250;

let sequence = 0;
let visible: ToastItem[] = [];
let queued: ToastItem[] = [];
let snapshot: ToastSnapshot = { visible, queued };
const listeners = new Set<() => void>();

function publish(): void {
  snapshot = { visible: [...visible], queued: [...queued] };
  for (const listener of [...listeners]) listener();
}

export function subscribeToasts(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function getToastSnapshot(): ToastSnapshot {
  return snapshot;
}

/** Messages are text. Anything else is coerced, never interpreted, so no value can become markup (AC17). */
function asText(value: unknown): string {
  return typeof value === 'string' ? value : value == null ? '' : String(value);
}

function resolveDuration(type: ToastType, duration: unknown): number {
  if (typeof duration === 'number' && Number.isFinite(duration) && duration > 0) return duration;
  return type === 'error' ? ERROR_DURATION : DEFAULT_DURATION;
}

// B05-16
function processQueue(): void {
  while (visible.length < MAX_VISIBLE && queued.length > 0) visible.push(queued.shift()!);
}

// B05-15
function showToast(type: ToastType, message: string, options?: ToastOptions): string {
  const description = options?.description === undefined ? undefined : asText(options.description);
  const action = options?.action;
  const item: ToastItem = {
    id: `toast-${++sequence}`,
    type,
    message: asText(message),
    description: description || undefined,
    duration: resolveDuration(type, options?.duration),
    action:
      action && typeof action.onClick === 'function' ? { label: asText(action.label), onClick: action.onClick } : undefined,
  };
  queued.push(item);
  processQueue();
  publish();
  return item.id;
}

// B05-18: called by the viewport when the fade-out is over, and by `toast.dismiss` for a toast that is not shown.
export function removeToast(id: string): void {
  const before = visible.length + queued.length;
  visible = visible.filter((item) => item.id !== id);
  queued = queued.filter((item) => item.id !== id);
  if (visible.length + queued.length === before) return;
  processQueue();
  publish();
}

/** Drops every toast at once (visible and queued). Used by teardown paths and tests. */
export function clearToasts(): void {
  if (visible.length === 0 && queued.length === 0) return;
  visible = [];
  queued = [];
  publish();
}

// B05-21: the public API. Each call returns the toast id.
export const toast = {
  success: (message: string, options?: ToastOptions) => showToast('success', message, options),
  error: (message: string, options?: ToastOptions) => showToast('error', message, options),
  warning: (message: string, options?: ToastOptions) => showToast('warning', message, options),
  info: (message: string, options?: ToastOptions) => showToast('info', message, options),
  /** Removes a toast immediately, without the fade-out. */
  dismiss: removeToast,
};
