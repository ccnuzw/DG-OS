// Shell-only glyphs. Deliberately not a general icon set: each one is here because a workspace
// component renders it. Strokes follow currentColor so tone comes from the surrounding text colour.

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
      <path d="M4 4h6v6H4zm10 0h6v6h-6zM4 14h6v6H4zm10 0h6v6h-6z" />
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
  return (
    <svg aria-hidden="true" focusable="false" viewBox="0 0 24 24" className={className} {...stroke}>
      <path d="M12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8zM12 3v2m0 14v2M3 12h2m14 0h2" />
    </svg>
  );
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
      <path d="M3 4h18v16H3zM9 4v16m5-12-3 4 3 4" />
    </svg>
  );
}
