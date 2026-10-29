import { useEffect } from 'react';
import { moduleRuntimeHost } from './moduleRuntimeHost';

// Mounted once at the App root so the attachment survives SPA route changes (D044).
// Outside Tauri (browser dev, unit tests of other pages) there is nothing to attach to.
export default function ModuleHostBootstrap() {
  useEffect(() => {
    if (typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window) {
      void moduleRuntimeHost.ensureAttached();
    }
  }, []);
  return null;
}
