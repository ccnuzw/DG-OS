// MUST mirror `is_settings_route` in src-tauri/src/host.rs (D037): the UI must never show the
// Settings page for a path the Rust gate would deny. The check is on the raw (undecoded) pathname.
export function isHostSettingsPath(pathname: string): boolean {
  return pathname === '/settings' || pathname.startsWith('/settings/');
}
