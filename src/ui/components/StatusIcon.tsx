import { cn } from '../lib/cn';

// The five built-in status icons of D064 (FR-005 B05-20). Inline SVG, strokes follow `currentColor`, no icon package.
// They are decorative: meaning is carried by the surrounding text and role, never by the glyph alone.

export type StatusIconName = 'success' | 'error' | 'warning' | 'info' | 'close';

const PATHS: Record<StatusIconName, string[]> = {
  success: ['M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18Z', 'M8 12.5l2.7 2.7L16 9.8'],
  error: ['M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18Z', 'M9 9l6 6M15 9l-6 6'],
  warning: ['M12 4.2 21 19.5H3L12 4.2Z', 'M12 10v4.2M12 16.9v.1'],
  info: ['M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18Z', 'M12 11v5M12 8v.1'],
  close: ['M6 6l12 12M18 6 6 18'],
};

export interface StatusIconProps {
  name: StatusIconName;
  className?: string;
}

export function StatusIcon({ name, className }: StatusIconProps) {
  return (
    <svg
      aria-hidden="true"
      focusable="false"
      viewBox="0 0 24 24"
      fill="none"
      data-icon={name}
      className={cn('size-5 shrink-0', className)}
    >
      {PATHS[name].map((d) => (
        <path key={d} d={d} stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
      ))}
    </svg>
  );
}
