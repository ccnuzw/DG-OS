import type { ReactNode } from 'react';

interface Props {
  /** A prop named `style` in a type is not an inline style. */
  children?: ReactNode;
}

const tone = { variant: 'red', size: 'sm' };
const items = ['a', 'b'];

export function Clean({ children }: Props) {
  return (
    <div className="bg-background p-2 text-sm data-[state=open]:bg-accent aria-[sort=asc]:font-bold group-data-[open=true]:p-2 w-(--radix-select-trigger-width) hover:text-foreground">
      <p className={`gap-2 ${tone.variant}`}>{items[0]}</p>
      <svg fill="currentColor" stroke="transparent" />
      {children}
    </div>
  );
}

export const mix = 'color-mix(in srgb, var(--color-primary) 50%, transparent)';
export const note = 'see &#123; and #section and items[0].name';

// R5: removing the outline is fine when a focus-visible utility sits in the same string; outline-hidden is allowed.
export const ring = 'outline-none focus-visible:outline-2 focus-visible:outline-focus-ring';
export const hiddenRing = 'outline-hidden focus-visible:outline-2';
