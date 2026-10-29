// Internal control glyphs (check, minus, chevron). Not the 5 public status icons of D064, which arrive in
// Phase 5; no icon package is used. All strokes follow `currentColor`.

interface GlyphProps {
  className?: string;
}

export function CheckGlyph({ className }: GlyphProps) {
  return (
    <svg aria-hidden="true" focusable="false" viewBox="0 0 16 16" fill="none" className={className ?? 'size-3'}>
      <path d="M3.5 8.5l3 3 6-7" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

export function MinusGlyph({ className }: GlyphProps) {
  return (
    <svg aria-hidden="true" focusable="false" viewBox="0 0 16 16" fill="none" className={className ?? 'size-3'}>
      <path d="M4 8h8" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
    </svg>
  );
}

export function ChevronGlyph({ className }: GlyphProps) {
  return (
    <svg aria-hidden="true" focusable="false" viewBox="0 0 16 16" fill="none" className={className ?? 'size-4'}>
      <path d="M4 6l4 4 4-4" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}
