import { useEffect } from 'react';
import ModuleMcpConfirm from './ModuleMcpConfirm';
import { moduleRuntimeHost } from './moduleRuntimeHost';

// Mounted once at the App root so the attachment survives SPA route changes (D044).
// Outside Tauri (browser dev, unit tests of other pages) there is nothing to attach to.
// The MCP confirmation dialog (D093) is host UI, so it lives here at the root, never inside a module container.
export default function ModuleHostBootstrap() {
  useEffect(() => {
    if (typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window) {
      void moduleRuntimeHost.ensureAttached();
    }
  }, []);
  return <ModuleMcpConfirm />;
}
