// `host.ui` (D067, FR-005 §13.3 B05-34): the design-system surface handed to dynamic modules.
//
// Modules are plain JS without type checking, so the guarantees the types give host code are re-applied at
// runtime here. This is misuse prevention, not a sandbox: a module shares the realm and the document (D042),
// and the security boundary stays the D041 admission.
//  - Components: every one goes through `guard`, which drops `style` before the props reach the component.
//    `className` is kept (D067). Sub-components (`Card.Header`, `Tabs.List`, ...) are guarded one by one.
//  - `toast`: text only. Only the four type methods exist; `action` callbacks, `dismiss` and toast ids are not
//    exposed, so a module can neither run code from the host's toast UI nor remove another module's toast.
//  - `Modal`: only `confirm`, `alert` and `error`. Neither the declarative component, `ModalHost`, `closeAll`
//    nor the store entry ids are reachable. `ModalHost` stays mounted by the host in `main.tsx`.
// The result is deep-frozen and shared by all modules: it holds no per-module state.
import { createElement, type ComponentType } from 'react';
import {
  Button,
  Card,
  Checkbox,
  Divider,
  Input,
  Loading,
  Modal,
  RadioGroup,
  Select,
  Stack,
  Switch,
  Tabs,
  Tooltip,
  toast,
} from '@dgos/ui';
import type { AlertOptions, ConfirmOptions, ToastType } from '@dgos/ui';
import { omitStyle } from '@dgos/ui/lib/props';

// Frozen, so one module cannot change a property (`displayName`, a sub-component) that every other module shares.
// Statics are attached before the freeze.
function guardWith<P extends object, S extends object>(Component: ComponentType<P>, statics: S): ComponentType<P> & Readonly<S> {
  const guarded = (props: P) => createElement(Component, omitStyle(props));
  return Object.freeze(Object.assign(guarded, statics));
}

function guard<P extends object>(Component: ComponentType<P>): ComponentType<P> {
  return guardWith(Component, {});
}

const guardedCard = guardWith(Card, {
  Header: guard(Card.Header),
  Body: guard(Card.Body),
  Footer: guard(Card.Footer),
});

const guardedTabs = guardWith(Tabs, {
  List: guard(Tabs.List),
  Trigger: guard(Tabs.Trigger),
  Content: guard(Tabs.Content),
});

function asText(value: unknown): string {
  return typeof value === 'string' ? value : value == null ? '' : String(value);
}

function toastMethod(type: ToastType) {
  return (message: unknown, options?: { description?: unknown; duration?: unknown }): void => {
    const picked: { description?: string; duration?: number } = {};
    if (options && typeof options === 'object') {
      if (options.description !== undefined) picked.description = asText(options.description);
      if (typeof options.duration === 'number') picked.duration = options.duration;
    }
    toast[type](asText(message), picked);
  };
}

const moduleToast = Object.freeze({
  success: toastMethod('success'),
  error: toastMethod('error'),
  warning: toastMethod('warning'),
  info: toastMethod('info'),
});

function fn<T extends (...args: never[]) => unknown>(value: unknown): T | undefined {
  return typeof value === 'function' ? (value as T) : undefined;
}

function record(value: unknown): Record<string, unknown> {
  return typeof value === 'object' && value !== null ? (value as Record<string, unknown>) : {};
}

// Options are rebuilt field by field: spreading the module's object would let it set entry fields (`kind`, `id`)
// that the Modal API decides itself.
function confirmOptions(raw: unknown): ConfirmOptions {
  const o = record(raw);
  return {
    title: o.title as ConfirmOptions['title'],
    content: o.content as ConfirmOptions['content'],
    icon: o.icon as ConfirmOptions['icon'],
    confirmText: o.confirmText as ConfirmOptions['confirmText'],
    cancelText: o.cancelText as ConfirmOptions['cancelText'],
    confirmVariant: o.confirmVariant as ConfirmOptions['confirmVariant'],
    onConfirm: fn<NonNullable<ConfirmOptions['onConfirm']>>(o.onConfirm),
    onCancel: fn<NonNullable<ConfirmOptions['onCancel']>>(o.onCancel),
  };
}

function alertOptions(raw: unknown): AlertOptions {
  const o = record(raw);
  return {
    title: o.title as AlertOptions['title'],
    content: o.content as AlertOptions['content'],
    icon: o.icon as AlertOptions['icon'],
    okText: o.okText as AlertOptions['okText'],
    onOk: fn<NonNullable<AlertOptions['onOk']>>(o.onOk),
  };
}

// The handle carries `close` only: the entry id is not something a module needs.
function narrow(handle: { close: () => void }): Readonly<{ close: () => void }> {
  return Object.freeze({ close: () => handle.close() });
}

const moduleModal = Object.freeze({
  confirm: (options: ConfirmOptions) => narrow(Modal.confirm(confirmOptions(options))),
  alert: (options: AlertOptions) => narrow(Modal.alert(alertOptions(options))),
  error: (options: AlertOptions) => narrow(Modal.error(alertOptions(options))),
});

export const moduleHostUi = Object.freeze({
  Button: guard(Button),
  Input: guard(Input),
  Select: guard(Select),
  Checkbox: guard(Checkbox),
  // FR-005 §13.3 lists `Radio`; the component behind it is `RadioGroup` (§7.2.4).
  Radio: guard(RadioGroup),
  Switch: guard(Switch),
  Card: guardedCard,
  Tooltip: guard(Tooltip),
  Loading: guard(Loading),
  Divider: guard(Divider),
  Tabs: guardedTabs,
  Stack: guard(Stack),
  Modal: moduleModal,
  toast: moduleToast,
});

export type ModuleHostUi = typeof moduleHostUi;
