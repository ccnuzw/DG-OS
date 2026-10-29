import type { ReactNode } from 'react';
import { cn } from '../lib/cn';
import { hasVisibleLabel, omitStyle, type DomProps } from '../lib/props';
import { Spinner } from './Spinner';

export type LoadingSize = 'sm' | 'md' | 'lg';

const SPINNER_SIZE: Record<LoadingSize, string> = {
  sm: 'size-4',
  md: 'size-6',
  lg: 'size-8',
};

// AC14: `role="status"` always. With visible text that text is the name; without it `aria-label` is required
// (D069: never both, so the name cannot disagree with what is shown).
export type LoadingProps = Omit<DomProps<'div'>, 'children' | 'aria-label' | 'aria-labelledby'> & {
  size?: LoadingSize;
} & ({ label: ReactNode; 'aria-label'?: undefined } | { label?: undefined; 'aria-label': string });

export function Loading(props: LoadingProps) {
  const { size = 'md', label, className, 'aria-label': ariaLabel, ...rest } = omitStyle(props);
  return (
    <div
      {...rest}
      aria-label={hasVisibleLabel(label) ? undefined : ariaLabel}
      role="status"
      className={cn('inline-flex items-center gap-2 text-sm text-muted-foreground', className)}
    >
      <Spinner className={cn('text-primary', SPINNER_SIZE[size])} />
      {hasVisibleLabel(label) ? <span>{label}</span> : null}
    </div>
  );
}
