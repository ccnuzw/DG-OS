/** Geometry follows density's rem root; text budgets follow the independent UI font. */
export function layoutMetrics(root: HTMLElement) {
  const documentStyle = getComputedStyle(root.ownerDocument.documentElement);
  const geometry = Number.parseFloat(documentStyle.fontSize) || 14;
  const ui = Number.parseFloat(getComputedStyle(root).getPropertyValue('--font-size-ui') || documentStyle.getPropertyValue('--font-size-ui')) || 12;
  return { geometry, ui };
}

/** Full navigation, code and conversation fit regardless of the saved split preference. */
export function workbenchMinimumWidth(root: HTMLElement): number {
  const { geometry, ui } = layoutMetrics(root);
  return Math.max(16 * geometry, 16 * ui)
    + Math.max(32 * geometry, 32 * ui)
    + Math.max(20 * geometry, 22 * ui)
    + .5 * geometry + 2;
}

/** Always budget an expanded rail, so auto-collapse cannot feed back into its own measurement. */
export function shellMinimumWidth(root: HTMLElement): number {
  const { geometry } = layoutMetrics(root);
  const expandedRail = Math.max(176, Math.min(216, 188 * geometry / 14));
  return expandedRail + workbenchMinimumWidth(root);
}
