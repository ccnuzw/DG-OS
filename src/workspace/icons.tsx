import { Settings } from 'lucide-react';

// Shell-only glyphs use semantic colors so they follow the active theme.

interface GlyphProps {
  className?: string;
}

const stroke = {
  fill: 'none',
  stroke: 'currentColor',
  strokeWidth: 1.6,
  strokeLinecap: 'round' as const,
  strokeLinejoin: 'round' as const,
};

export function DesktopGlyph({ className }: GlyphProps) {
  return (
    <svg aria-hidden="true" focusable="false" viewBox="0 0 24 24" className={className} {...stroke}>
      <path d="M4 4h6v6H4z" fill="currentColor" stroke="none" className="text-status-info-text" />
      <path d="M14 4h6v6h-6z" fill="currentColor" stroke="none" className="text-status-success-text" />
      <path d="M4 14h6v6H4z" fill="currentColor" stroke="none" className="text-status-warning-text" />
      <path d="M14 14h6v6h-6z" fill="currentColor" stroke="none" className="text-accent-default" />
    </svg>
  );
}

export function ChatGlyph({ className }: GlyphProps) {
  return (
    <svg aria-hidden="true" focusable="false" viewBox="0 0 24 24" className={className} {...stroke}>
      <path d="M4 4h16v12H9l-5 4z" />
    </svg>
  );
}

export function SkillGlyph({ className }: GlyphProps) {
  return (
    <svg aria-hidden="true" focusable="false" viewBox="0 0 24 24" className={className} {...stroke}>
      <path d="M5 4h14v16H5zm4 4h6M9 12h6M9 16h3" />
    </svg>
  );
}

export function SettingsGlyph({ className }: GlyphProps) {
  return <Settings aria-hidden="true" focusable="false" className={`${className ?? ''} workspace-settings-glyph`} strokeWidth={1.6} />;
}

export function CloseGlyph({ className }: GlyphProps) {
  return (
    <svg aria-hidden="true" focusable="false" viewBox="0 0 24 24" className={className} {...stroke}>
      <path d="m6 6 12 12M6 18 18 6" />
    </svg>
  );
}

export function PlusGlyph({ className }: GlyphProps) {
  return (
    <svg aria-hidden="true" focusable="false" viewBox="0 0 24 24" className={className} {...stroke}>
      <path d="M12 5v14M5 12h14" />
    </svg>
  );
}

export function FocusGlyph({ className }: GlyphProps) {
  return (
    <svg aria-hidden="true" focusable="false" viewBox="0 0 24 24" className={className} {...stroke}>
      <path d="M9 4H4v5m11-5h5v5M9 20H4v-5m11 5h5v-5" />
    </svg>
  );
}

export function CollapseGlyph({ className }: GlyphProps) {
  return (
    <svg aria-hidden="true" focusable="false" viewBox="0 0 24 24" className={className} {...stroke}>
      <path d="m13 7-5 5 5 5m5-10-5 5 5 5" />
    </svg>
  );
}
