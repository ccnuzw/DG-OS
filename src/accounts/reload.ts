// A full page load is how a session change reaches every page and the module host (V1-FR-008): each of them
// re-reads the current user's data on start. Kept in its own module so tests can replace it.
export function reloadApp(): void {
  window.location.reload();
}
