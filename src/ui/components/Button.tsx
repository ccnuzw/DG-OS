import { cva, type VariantProps } from 'class-variance-authority';
import type { MouseEvent, ReactNode } from 'react';
import { cn } from '../lib/cn';
import { FOCUS_RING } from '../lib/focus';
import { omitStyle, type DomProps } from '../lib/props';
import { Spinner } from './Spinner';

// Heights are the spec's 32/36/40px at 100% density: h-8/h-9/h-10 are 8/9/10 x 4rem/14 (FR-005 §7.1.1).
const buttonVariants = cva(
  [
    'inline-flex shrink-0 items-center justify-center gap-2 whitespace-nowrap rounded-md font-medium',
    'transition-colors disabled:cursor-not-allowed disabled:opacity-disabled aria-busy:cursor-progress',
    FOCUS_RING,
  ],
  {
    variants: {
      variant: {
        primary: 'bg-primary text-primary-foreground enabled:hover:bg-primary-hover enabled:active:bg-primary-active',
        secondary: 'border border-input bg-secondary text-secondary-foreground enabled:hover:bg-border',
        danger: 'bg-destructive text-destructive-foreground enabled:hover:bg-error-text',
        ghost: 'bg-transparent text-foreground enabled:hover:bg-accent',
        link: 'bg-transparent text-primary underline-offset-4 enabled:hover:underline',
      },
      size: {
        sm: 'h-8 px-3 text-sm',
        md: 'h-9 px-4 text-sm',
        lg: 'h-10 px-5 text-base',
      },
      iconOnly: {
        true: 'px-0',
        false: '',
      },
    },
    compoundVariants: [
      { iconOnly: true, size: 'sm', className: 'w-8' },
      { iconOnly: true, size: 'md', className: 'w-9' },
      { iconOnly: true, size: 'lg', className: 'w-10' },
      // A link reads as text: no fixed height, no padding. Compound classes come last, so they beat the size's.
      { variant: 'link', className: 'h-auto px-0' },
    ],
    defaultVariants: { variant: 'primary', size: 'md', iconOnly: false },
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
