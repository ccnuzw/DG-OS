// AC24: every interactive component shows a `focus-visible` ring drawn from --color-focus-ring (`ring` in the
// Shadcn mapping, FR-005 §4.7). The outline is never removed, so there is no `outline-none` to pair (design:check R5).
// The class names are complete literals so Tailwind's scanner emits them.

/** Ring drawn outside the control. */
export const FOCUS_RING =
  'focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-focus-ring';

/** Ring drawn inside the box, for items that sit flush in a list (Select options). */
export const FOCUS_RING_INSET =
  'focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-focus-ring';
