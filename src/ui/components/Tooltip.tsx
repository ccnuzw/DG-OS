import type { ReactElement } from 'react';
import { Tooltip as TooltipPrimitive } from 'radix-ui';

// Tooltip on the Radix primitive (D066, AC15). Radix opens it on hover and on keyboard focus, closes it on Esc,
// blur, pointer leave and click, and sets `aria-describedby` on the trigger while it is open. The content is
// text only: it is coerced to a string and rendered as a text node, so markup in it is shown, not parsed.
// Each Tooltip carries its own Provider, so no application-level provider is needed.

export interface TooltipProps {
  /** Plain text. Nothing (null, undefined, empty) renders the child alone. */
  content: string;
  /** Exactly one focusable element; it receives the ref and the event handlers. */
  children: ReactElement;
  /** Milliseconds before a hover opens the tooltip. Focus opens it at once. */
  delayDuration?: number;
}

export function Tooltip({ content, children, delayDuration = 400 }: TooltipProps) {
  const text = content == null ? '' : String(content);
  if (text === '') return children;
  return (
    <TooltipPrimitive.Provider delayDuration={delayDuration}>
      <TooltipPrimitive.Root>
        <TooltipPrimitive.Trigger asChild>{children}</TooltipPrimitive.Trigger>
        <TooltipPrimitive.Portal>
          <TooltipPrimitive.Content
            sideOffset={6}
            className="z-popover max-w-75 rounded-md border border-border bg-popover px-3 py-1.5 text-sm text-popover-foreground shadow-md wrap-break-word"
          >
            {text}
          </TooltipPrimitive.Content>
        </TooltipPrimitive.Portal>
      </TooltipPrimitive.Root>
    </TooltipPrimitive.Provider>
  );
}
