import { cn } from '../lib/cn';
import { omitStyle, type DomProps } from '../lib/props';

// Layout primitive (D067), not one of the 13 components. `gap` is a closed enum of spacing-token steps. Every
// class below is a complete literal: Tailwind generates CSS only for class names it can read in the source
// (D066), so `gap-${n}` would silently produce nothing.
const GAP_CLASS = {
  0: 'gap-0',
  1: 'gap-1',
  2: 'gap-2',
  3: 'gap-3',
  4: 'gap-4',
  5: 'gap-5',
  6: 'gap-6',
  8: 'gap-8',
  10: 'gap-10',
  12: 'gap-12',
} as const;

export type StackGap = keyof typeof GAP_CLASS;
export type StackDirection = 'horizontal' | 'vertical';

export type StackProps = Omit<DomProps<'div'>, 'role'> & {
  /** Spacing token step (`--spacing-N`). Required, so spacing is always a deliberate choice. */
  gap: StackGap;
  direction?: StackDirection;
};

/** Own keys only: `GAP_CLASS['constructor']` would otherwise hand a function to `cn`. Unknown steps get no class. */
function gapClass(gap: unknown): string | undefined {
  return typeof gap === 'number' && Object.prototype.hasOwnProperty.call(GAP_CLASS, gap)
    ? GAP_CLASS[gap as StackGap]
    : undefined;
}

export function Stack(props: StackProps) {
  const { gap, direction = 'vertical', className, ...rest } = omitStyle(props);
  return (
    <div
      {...rest}
      className={cn('flex', direction === 'horizontal' ? 'flex-row' : 'flex-col', gapClass(gap), className)}
    />
  );
}
