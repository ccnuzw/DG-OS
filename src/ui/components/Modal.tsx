import { useEffect, useLayoutEffect, useRef, useState, useSyncExternalStore } from 'react';
import type { FocusEvent, ReactNode } from 'react';
import { Dialog } from 'radix-ui';
import {
  closeModal,
  getModalEntries,
  modalApi,
  subscribeModals,
  type ModalIcon,
  type ModalKind,
} from '../feedback/modalStore';
import { Button, type ButtonVariant } from './Button';
import { StatusIcon } from './StatusIcon';
import { t, useLocale } from '../../i18n';
import './patterns.css';

// Modal on the Radix Dialog primitive (D066). What the primitive provides and this file relies on (verified in
// jsdom by tests/ui-overlays.test.tsx): `role="dialog"` named by the title, siblings hidden from assistive
// technology while open, focus moved in and trapped, Esc handled by the topmost layer only. What this file adds:
//  - B05-30 focus returns to the element that had it before opening (works without a Dialog.Trigger);
//  - B05-31 stacking: every dialog renders its own scrim, so scrims add up and the page darkens with each layer;
//  - B05-32 a confirm callback runs once however fast the button is clicked, and the dialog cannot be dismissed
//    while it runs;
//  - a click on the scrim never dismisses (a stray click must not cancel a delete or lose a form).
// Real focus movement, scrim opacity and viewport-constrained geometry are measured by the WKWebView smoke.

const ICON_COLOR: Record<ModalIcon, string> = {
  success: 'text-status-success-text',
  error: 'text-status-error-text',
  warning: 'text-status-warning-text',
  info: 'text-status-info-text',
};

/** Shown when the confirm callback rejects. Fixed text: a rejection's own message is never rendered. */
export const CONFIRM_FAILED_TEXT = '操作未完成，请重试。';

export interface ModalProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  /** Descriptive body text, linked to the dialog with `aria-describedby`. */
  description?: ReactNode;
  /** `confirm`: Cancel + Confirm. `alert`: one OK button. `form`: Cancel + Confirm around `children`. */
  kind?: ModalKind;
  icon?: ModalIcon;
  /** Extra body, e.g. the fields of a `form` dialog. */
  children?: ReactNode;
  confirmText?: string;
  cancelText?: string;
  confirmVariant?: ButtonVariant;
  /** Runs once per confirm. A returned promise keeps the dialog busy; the dialog closes when it resolves and stays open if it rejects. */
  onConfirm?: () => void | Promise<void>;
  /** Runs when the dialog is dismissed without confirming (Cancel, ×, Esc). */
  onCancel?: () => void;
  /** Replaces the default buttons. */
  footer?: ReactNode;
}

// WebKit can scroll only the text caret into view on Tab, leaving the input boundary clipped.
// Correct the nearest modal scrollport after native focus scrolling, without moving the panel or page.
function revealFocusedControl(event: FocusEvent<HTMLDivElement>) {
  const region = event.currentTarget;
  const target = event.target;
  requestAnimationFrame(() => {
    if (!region.isConnected || document.activeElement !== target || !region.contains(target)) return;
    const bounds = region.getBoundingClientRect();
    const field = target.getBoundingClientRect();
    const padding = parseFloat(getComputedStyle(region).scrollPaddingTop) || 4;
    const top = bounds.top + region.clientTop + padding;
    const bottom = bounds.top + region.clientTop + region.clientHeight - padding;
    if (field.top < top || field.height > bottom - top) region.scrollTop += field.top - top;
    else if (field.bottom > bottom) region.scrollTop += field.bottom - bottom;
  });
}

function ModalView(props: ModalProps) {
  useLocale();
  const {
    open,
    onOpenChange,
    title,
    description,
    kind = 'confirm',
    icon,
    children,
    confirmText,
    cancelText = t('common.cancel'),
    confirmVariant = 'primary',
    onConfirm,
    onCancel,
    footer,
  } = props;
  const [pending, setPending] = useState(false);
  const [failed, setFailed] = useState(false);
  // A ref, not state: two clicks in the same tick both see the same render's state.
  const busy = useRef(false);
  // One round per open period. Closing, reopening and unmounting end the current round; a confirm that was started
  // in an ended round must not show its failure, close the dialog or clear the busy flag of a later round.
  const round = useRef(0);
  const returnFocusTo = useRef<HTMLElement | null>(null);

  useLayoutEffect(() => {
    busy.current = false;
    setPending(false);
    setFailed(false);
    return () => {
      round.current += 1;
    };
  }, [open]);

  const handleOpenChange = (next: boolean) => {
    if (next) return onOpenChange(true);
    if (busy.current) return;
    onCancel?.();
    onOpenChange(false);
  };

  const handleConfirm = async () => {
    if (busy.current) return;
    const mine = round.current;
    busy.current = true;
    setPending(true);
    setFailed(false);
    let succeeded = false;
    try {
      await onConfirm?.();
      succeeded = true;
    } catch {
      if (round.current === mine) setFailed(true);
    }
    if (round.current !== mine) return;
    busy.current = false;
    setPending(false);
    if (succeeded) onOpenChange(false);
  };

  const defaultFooter =
    kind === 'alert' ? (
      <Button loading={pending} onClick={handleConfirm}>
        {confirmText ?? t('system.confirm')}
      </Button>
    ) : (
      <>
        <Button variant="secondary" disabled={pending} onClick={() => handleOpenChange(false)}>
          {cancelText}
        </Button>
        <Button variant={confirmVariant} loading={pending} onClick={handleConfirm}>
          {confirmText ?? t('system.confirm')}
        </Button>
      </>
    );

  return (
    <Dialog.Root open={open} onOpenChange={handleOpenChange}>
      <Dialog.Portal>
        {/* The inset box owns the viewport bounds, including at enlarged density; no intrinsic minimum can exceed it. */}
        <div className="pointer-events-none fixed inset-4 z-modal flex items-center justify-center">
        <Dialog.Overlay className="pointer-events-auto fixed inset-0 z-modal bg-overlay-scrim opacity-overlay animate-overlay-in motion-reduce:animate-none" />
        <Dialog.Content
          data-modal-kind={kind}
          {...(description ? {} : { 'aria-describedby': undefined })}
          onInteractOutside={(event) => event.preventDefault()}
          onOpenAutoFocus={() => {
            // Runs before Radix moves focus, so this is still the element that opened the dialog.
            returnFocusTo.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
          }}
          onCloseAutoFocus={(event) => {
            event.preventDefault();
            const target = returnFocusTo.current;
            if (target && target !== document.body && target.isConnected) target.focus();
          }}
          className="ui-modal-panel pointer-events-auto relative z-modal flex max-h-full flex-col gap-4 rounded-lg border border-edge-control bg-surface-popover p-6 text-ink-primary shadow-xl wrap-anywhere animate-modal-in motion-reduce:animate-none"
        >
          <div className="ui-modal-heading flex shrink-0 items-start gap-3 pr-8">
            {icon ? <StatusIcon name={icon} className={ICON_COLOR[icon]} /> : null}
            <Dialog.Title className="min-w-0 text-lg font-semibold">{title}</Dialog.Title>
          </div>
          <div className="ui-modal-body flex min-h-0 min-w-0 flex-col gap-4 overflow-y-auto" onFocusCapture={revealFocusedControl}>
            {description ? (
              <Dialog.Description asChild>
                <div className="text-sm text-ink-secondary">{description}</div>
              </Dialog.Description>
            ) : null}
            {children}
            {failed ? (
              <p role="alert" className="text-sm text-status-error-text">
                {CONFIRM_FAILED_TEXT}
              </p>
            ) : null}
          </div>
          <div className="ui-modal-footer flex min-w-0 shrink-0 flex-wrap justify-end gap-2" onFocusCapture={revealFocusedControl}>{footer ?? defaultFooter}</div>
          {/* Last in DOM order so the first Tab stop is an action, not the ×, while it stays top-right. */}
          <Dialog.Close asChild>
            <Button
              variant="ghost"
              size="sm"
              iconOnly
              aria-label={t('common.close')}
              disabled={pending}
              className="absolute right-3 top-3"
            >
              <StatusIcon name="close" className="size-4" />
            </Button>
          </Dialog.Close>
        </Dialog.Content>
        </div>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

/**
 * Renders the dialogs opened with `Modal.confirm/alert/error`. Mount once at the application root.
 *
 * Dialogs are mounted one per commit, oldest first. Radix hides the rest of the page from assistive technology
 * when a dialog mounts; two dialogs mounting in the same commit would each hide the other, leaving the top one
 * unexposed. Mounting in sequence gives the same result as opening them one after another.
 */
export function ModalHost() {
  const entries = useSyncExternalStore(subscribeModals, getModalEntries, getModalEntries);
  const [mounted, setMounted] = useState<ReadonlySet<string>>(() => new Set());
  useEffect(() => {
    setMounted((previous) => {
      const next = new Set(entries.filter((entry) => previous.has(entry.id)).map((entry) => entry.id));
      const pending = entries.find((entry) => !previous.has(entry.id));
      if (pending) next.add(pending.id);
      return next.size === previous.size && [...next].every((id) => previous.has(id)) ? previous : next;
    });
  });
  return (
    <>
      {entries.filter((entry) => mounted.has(entry.id)).map((entry) => (
        <ModalView
          key={entry.id}
          open
          onOpenChange={(next) => {
            if (!next) closeModal(entry.id);
          }}
          title={entry.title}
          description={entry.content}
          kind={entry.kind}
          icon={entry.icon}
          confirmText={entry.confirmText}
          cancelText={entry.cancelText}
          confirmVariant={entry.confirmVariant}
          footer={entry.footer}
          onConfirm={entry.onConfirm}
          onCancel={entry.onCancel}
        />
      ))}
    </>
  );
}

export const Modal = Object.assign(ModalView, modalApi);
