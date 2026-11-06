import { createContext, useContext, useMemo, type ReactElement, type ReactNode } from 'react';
import { Popover as Primitive } from 'radix-ui';

// Internal ownership context: never exposed through host.ui.
export const OverlayContainerContext = createContext<HTMLElement | undefined>(undefined);
export interface PopoverProps {
  trigger?: ReactElement;
  anchorPoint?: { x: number; y: number };
  content: ReactNode;
  label: string;
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
  side?: 'top' | 'right' | 'bottom' | 'left';
  align?: 'start' | 'center' | 'end';
}
export function Popover({ trigger, anchorPoint, content, label, open, onOpenChange, side = 'bottom', align = 'center' }: PopoverProps) {
  const container = useContext(OverlayContainerContext);
  const anchor = useMemo(() => ({ current: { getBoundingClientRect: () => new DOMRect(anchorPoint?.x ?? 0, anchorPoint?.y ?? 0, 0, 0) } }), [anchorPoint?.x, anchorPoint?.y]);
  if (typeof label !== 'string' || !label.trim() || (!trigger && (!anchorPoint || !Number.isFinite(anchorPoint.x) || !Number.isFinite(anchorPoint.y)))) return null;
  return <Primitive.Root open={open} onOpenChange={onOpenChange}>
    {trigger ? <Primitive.Trigger asChild>{trigger}</Primitive.Trigger> : <Primitive.Anchor virtualRef={anchor} />}
    <Primitive.Portal container={container}>
      <Primitive.Content aria-label={label} side={side} align={align} sideOffset={4} className="z-popover max-h-(--radix-popover-content-available-height) max-w-(--radix-popover-content-available-width) overflow-auto rounded-md border border-solid border-border-subtle bg-surface-popover p-3 font-sans text-sm text-ink-primary shadow-md">
        {content}
      </Primitive.Content>
    </Primitive.Portal>
  </Primitive.Root>;
}
