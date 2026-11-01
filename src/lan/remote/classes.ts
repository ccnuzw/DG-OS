import { FOCUS_RING } from '@dgos/ui/lib/focus';

// Shared Tailwind class strings of the remote SPA. Control text is text-lg (16px at the default density) so
// mobile Safari does not zoom the page when an input is focused.

export const CONTROL =
  `w-full rounded-md border border-input bg-background px-3 text-lg text-foreground placeholder:text-text-tertiary disabled:cursor-not-allowed disabled:opacity-disabled ${FOCUS_RING}`;

export const SELECT = `${CONTROL} h-11`;
export const TEXT_INPUT = `${CONTROL} h-11`;
export const TEXTAREA = `${CONTROL} min-h-11 resize-none py-2`;

export const ICON_BUTTON = `rounded-md px-3 text-base text-foreground enabled:hover:bg-accent disabled:opacity-disabled ${FOCUS_RING}`;
