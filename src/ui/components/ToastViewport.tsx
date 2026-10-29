import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from 'react';
import type { FocusEvent } from 'react';
import { createPortal } from 'react-dom';
import { cva } from 'class-variance-authority';
import { cn } from '../lib/cn';
import { FOCUS_RING } from '../lib/focus';
import {
  FADE_OUT_MS,
  getToastSnapshot,
  removeToast,
  subscribeToasts,
  type ToastItem,
} from '../feedback/toastStore';
import { Button } from './Button';
import { StatusIcon } from './StatusIcon';

// Colours come in pairs (D069): the text token for the tinted background, the plain semantic colour for the
// border and icon (non-text, >= 3:1). Classes are complete literals so Tailwind's scanner emits them.
const toastVariants = cva(
  [
    'pointer-events-auto flex min-w-75 max-w-100 items-start gap-3 rounded-lg border p-3 shadow-lg',
    'motion-reduce:animate-none',
  ],
  {
    variants: {
      type: {
        success: 'border-success bg-success-bg text-success-text',
        error: 'border-error bg-error-bg text-error-text',
        warning: 'border-warning bg-warning-bg text-warning-text',
        info: 'border-info bg-info-bg text-info-text',
      },
      closing: {
        true: 'animate-toast-out',
        false: 'animate-toast-in',
      },
    },
    defaultVariants: { closing: false },
  },
);

/**
 * One toast. It owns the auto-dismiss timer (B05-33, D069): the timer stops while the pointer is over the toast
 * or focus is inside it, and the time that was left is used when it resumes. When it runs out the toast fades for
 * `FADE_OUT_MS`, then leaves the store, which promotes the next queued toast.
 */
function ToastCard({ item }: { item: ToastItem }) {
  const { id, type, message, description, duration, action } = item;
  const [closing, setClosing] = useState(false);
  const [hovered, setHovered] = useState(false);
  const [focused, setFocused] = useState(false);
  const remaining = useRef(duration);
  const paused = hovered || focused;

  const close = useCallback(() => setClosing(true), []);

  // Count down only while nothing pauses it; remember what is left when it is paused.
  useEffect(() => {
    if (closing || paused) return;
    const startedAt = Date.now();
    const timer = setTimeout(close, remaining.current);
    return () => {
      clearTimeout(timer);
      remaining.current = Math.max(0, remaining.current - (Date.now() - startedAt));
    };
  }, [closing, paused, close]);

  useEffect(() => {
    if (!closing) return;
    const timer = setTimeout(() => removeToast(id), FADE_OUT_MS);
    return () => clearTimeout(timer);
  }, [closing, id]);

  // Focus moving between the toast's own buttons is not leaving it.
  const handleBlur = (event: FocusEvent<HTMLDivElement>) => {
    if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setFocused(false);
  };

  return (
    <div
      role={type === 'error' ? 'alert' : 'status'}
      // Same politeness as the role implies. Stated explicitly because an open Modal hides everything outside it
      // from assistive technology except `[aria-live]` elements (Radix uses aria-hidden's hideOthers).
      aria-live={type === 'error' ? 'assertive' : 'polite'}
      data-toast-type={type}
      data-state={closing ? 'closing' : 'open'}
      className={toastVariants({ type, closing })}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      onFocus={() => setFocused(true)}
      onBlur={handleBlur}
    >
      <StatusIcon name={type} className="mt-0.5" />
      <div className="min-w-0 flex-1 wrap-break-word">
        <p className="text-sm font-medium">{message}</p>
        {description ? <p className="mt-1 text-sm">{description}</p> : null}
        {action ? (
          <Button
            variant="secondary"
            size="sm"
            className="mt-2"
            onClick={() => {
              action.onClick();
              close();
            }}
          >
            {action.label}
          </Button>
        ) : null}
      </div>
      <button
        type="button"
        aria-label="关闭通知"
        onClick={close}
        className={cn('-m-1 inline-flex size-7 shrink-0 items-center justify-center rounded-md hover:opacity-hover', FOCUS_RING)}
      >
        <StatusIcon name="close" className="size-4" />
      </button>
    </div>
  );
}

/**
 * Mount once at the application root (B05-19). Without a mounted viewport `toast.*` only fills the queue.
 * Renders nothing while idle so no empty landmark is announced.
 *
 * The region is portalled to <body>. Mounted in place it would sit inside the app root, which an open Modal marks
 * `aria-hidden`; a toast raised then (e.g. `toast.success` in a confirm callback) would never be announced. As a
 * direct child of <body> it is outside that subtree, and its `aria-live` cards keep it visible when a dialog opens
 * after it. Being a root-level fixed element also puts its `z-toast` layer in the root stacking context (§4.8).
 */
export function ToastViewport() {
  const { visible } = useSyncExternalStore(subscribeToasts, getToastSnapshot, getToastSnapshot);
  if (visible.length === 0) return null;
  return createPortal(
    <section
      aria-label="通知"
      className="pointer-events-none fixed right-5 top-20 z-toast flex flex-col gap-2"
    >
      {visible.map((item) => (
        <ToastCard key={item.id} item={item} />
      ))}
    </section>,
    document.body,
  );
}
