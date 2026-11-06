import { cva, type VariantProps } from 'class-variance-authority';
import type { MouseEvent, ReactNode } from 'react';
import { cn } from '../lib/cn';
import { FOCUS_RING } from '../lib/focus';
import { omitStyle, type DomProps } from '../lib/props';
import { Spinner } from './Spinner';

// Density scales geometry; control-text-fit separately preserves the current text line and border allowance.
const buttonVariants = cva(
  [
    'control-text-fit inline-flex shrink-0 appearance-none items-center justify-center gap-2 whitespace-nowrap rounded-md border-0 border-solid font-sans font-medium',
    'transition-colors motion-reduce:transition-none disabled:cursor-not-allowed disabled:opacity-disabled aria-busy:cursor-progress',
    FOCUS_RING,
  ],
  {
    variants: {
      variant: {
        primary: 'bg-accent-default text-accent-on enabled:hover:bg-accent-hover enabled:active:bg-accent-active',
        secondary: 'border border-edge-control bg-surface-inset text-ink-primary enabled:hover:bg-surface-control-hover',
        danger: 'bg-status-error text-status-error-on enabled:hover:bg-status-error-text',
        ghost: 'bg-transparent text-ink-primary enabled:hover:bg-surface-hover',
        link: 'bg-transparent text-accent-default underline-offset-4 enabled:hover:underline',
      },
      // xs is the shared 28px compact control; md retains the published 36px default-height role.
      size: {
        xxs: 'h-6 px-2 text-xs',
        xs: 'h-(--control-height-compact) px-3 text-sm',
        sm: 'h-8 px-3 text-sm',
        md: 'h-(--control-height-default) px-4 text-sm',
        lg: 'h-10 px-5 text-base',
      },
      iconOnly: {
        true: 'px-0',
        false: '',
      },
    },
    compoundVariants: [
      { iconOnly: true, size: 'xxs', className: 'w-6' },
      { iconOnly: true, size: 'xs', className: 'w-(--control-height-compact)' },
      { iconOnly: true, size: 'sm', className: 'w-8' },
      { iconOnly: true, size: 'md', className: 'w-(--control-height-default)' },
      { iconOnly: true, size: 'lg', className: 'w-10' },
      // A link reads as text: no fixed height, no padding. Compound classes come last, so they beat the size's.
      { variant: 'link', className: 'h-auto px-0' },
    ],
    defaultVariants: { variant: 'primary', size: 'xs', iconOnly: false },
  },
);

export type ButtonVariant = NonNullable<VariantProps<typeof buttonVariants>['variant']>;
export type ButtonSize = NonNullable<VariantProps<typeof buttonVariants>['size']>;

interface ButtonBase extends Omit<DomProps<'button'>, 'children'> {
  variant?: ButtonVariant;
  size?: ButtonSize;
  /** Shows a spinner, sets `aria-busy` and swallows clicks. The button stays focusable, unlike `disabled`. */
  loading?: boolean;
}

// AC05: visible text needs no aria-label; an icon-only button cannot be built without one.
export type ButtonProps =
  | (ButtonBase & { iconOnly?: false; children: ReactNode })
  | (ButtonBase & { iconOnly: true; 'aria-label': string; children: ReactNode });

export function Button(props: ButtonProps) {
  const {
    variant,
    size,
    iconOnly = false,
    loading = false,
    className,
    type = 'button',
    onClick,
    children,
    ...rest
  } = omitStyle(props);

  const handleClick = (event: MouseEvent<HTMLButtonElement>) => {
    if (loading) {
      // Also stops a `type="submit"` button from submitting while the work it started is still running.
      event.preventDefault();
      return;
    }
    onClick?.(event);
  };

  return (
    <button
      {...rest}
      type={type}
      aria-busy={loading || undefined}
      onClick={handleClick}
      className={cn(buttonVariants({ variant, size, iconOnly }), className)}
    >
      {loading ? <Spinner className="size-4" /> : null}
      {iconOnly && loading ? null : children}
    </button>
  );
}
