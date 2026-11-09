/** Geometry follows density's rem root; text budgets follow the independent UI font. */
export function layoutMetrics(root: HTMLElement) {
  const documentStyle = getComputedStyle(root.ownerDocument.documentElement);
  const geometry = Number.parseFloat(documentStyle.fontSize) || 14;
  const ui = Number.parseFloat(getComputedStyle(root).getPropertyValue('--font-size-ui') || documentStyle.getPropertyValue('--font-size-ui')) || 12;
  return { geometry, ui };
}

/** Budget the drawer even when hidden, so toggling tools cannot feed back into geometry. */
export function workbenchMinimumWidth(root: HTMLElement): number {
  const { geometry, ui } = layoutMetrics(root);
  const drawer = Number.parseFloat(root.dataset.drawerWidth ?? '') || 24;
  return (3.5 + drawer + .5) * geometry + Math.max(26 * geometry, 28 * ui);
}

/** Always budget an expanded rail, so auto-collapse cannot feed back into its own measurement. */
export function shellMinimumWidth(root: HTMLElement): number {
  const { geometry, ui } = layoutMetrics(root);
  const expandedRail = Math.max(80, Math.min(112, 88 * geometry / 14), ui * 4 + 32);
  // The application shell keeps its existing navigation budget; workbench drawers are local.
  return expandedRail + Math.max(16 * geometry, 16 * ui) + Math.max(32 * geometry, 32 * ui) + Math.max(20 * geometry, 22 * ui) + .5 * geometry + 2;
}
