import { cn } from '../lib/cn';
import { omitStyle, type DomProps } from '../lib/props';

export type DividerOrientation = 'horizontal' | 'vertical';

export type DividerProps = Omit<DomProps<'div'>, 'children' | 'role' | 'aria-orientation'> & {
  orientation?: DividerOrientation;
};

/**
 * AC14: `role="separator"`; `aria-orientation="vertical"` only when vertical (horizontal is the implicit value
 * of a separator). The line is a token-coloured box, so it follows theme and density.
 */
export function Divider(props: DividerProps) {
  const { orientation = 'horizontal', className, ...rest } = omitStyle(props);
  const vertical = orientation === 'vertical';
  return (
    <div
      {...rest}
      role="separator"
      aria-orientation={vertical ? 'vertical' : undefined}
      className={cn('shrink-0 bg-divider', vertical ? 'h-full w-px self-stretch' : 'h-px w-full', className)}
    />
  );
}
