// Imperative Modal API (FR-005 §8.1.2, B05-30..32). `Modal.confirm/alert/error` push an entry here; `ModalHost`
// (components/Modal.tsx) renders every entry as a dialog. Entries are rendered in the order they were opened, and
// each dialog is portalled to <body> after the previous one, so a later dialog always paints above an earlier one.
// Radix then keeps Esc and focus trapping on the topmost dialog only (AC16).
import type { ReactNode } from 'react';
import type { ButtonVariant } from '../components/Button';

export type ModalKind = 'confirm' | 'alert' | 'form';
export type ModalIcon = 'success' | 'error' | 'warning' | 'info';

export interface ModalEntry {
  id: string;
  kind: ModalKind;
  title: string;
  content?: ReactNode;
  icon?: ModalIcon;
  confirmText?: string;
  cancelText?: string;
  confirmVariant?: ButtonVariant;
  onConfirm?: () => void | Promise<void>;
  onCancel?: () => void;
}

export interface ModalHandle {
  id: string;
  close: () => void;
}

export interface ConfirmOptions {
  title: string;
  content?: ReactNode;
  icon?: ModalIcon;
  confirmText?: string;
  cancelText?: string;
  confirmVariant?: ButtonVariant;
  onConfirm?: () => void | Promise<void>;
  onCancel?: () => void;
}

/** `Modal.alert` and `Modal.error` have one button. `okText`/`onOk` are its text and callback. */
export interface AlertOptions {
  title: string;
  content?: ReactNode;
  icon?: ModalIcon;
  okText?: string;
  onOk?: () => void | Promise<void>;
}

let sequence = 0;
let entries: readonly ModalEntry[] = [];
const listeners = new Set<() => void>();

function publish(next: readonly ModalEntry[]): void {
  entries = next;
  for (const listener of [...listeners]) listener();
}

export function subscribeModals(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function getModalEntries(): readonly ModalEntry[] {
  return entries;
}

export function closeModal(id: string): void {
  if (entries.some((entry) => entry.id === id)) publish(entries.filter((entry) => entry.id !== id));
}

function open(entry: Omit<ModalEntry, 'id'>): ModalHandle {
  const id = `modal-${++sequence}`;
  publish([...entries, { ...entry, id }]);
  return { id, close: () => closeModal(id) };
}

function openAlert(options: AlertOptions, icon: ModalIcon | undefined): ModalHandle {
  return open({
    kind: 'alert',
    title: options.title,
    content: options.content,
    icon: options.icon ?? icon,
    confirmText: options.okText,
    onConfirm: options.onOk,
  });
}

export const modalApi = {
  confirm: (options: ConfirmOptions): ModalHandle => open({ kind: 'confirm', ...options }),
  alert: (options: AlertOptions): ModalHandle => openAlert(options, undefined),
  /** An alert with the error tone, not a fourth kind (§13.2). */
  error: (options: AlertOptions): ModalHandle => openAlert(options, 'error'),
  /** Closes every dialog opened through this API. */
  closeAll: (): void => publish([]),
};
