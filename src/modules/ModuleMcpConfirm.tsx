import { useSyncExternalStore } from 'react';
import { Button, Modal } from '@dgos/ui';
import { moduleRuntimeHost, type ModuleRuntimeHost } from './moduleRuntimeHost';
import { useLocale } from '../i18n';

// Host-owned confirmation of a module's `mcp.call` (D093). Rendered by the host page at the application root,
// outside every module container; the pending list lives in the runtime host and is not part of the `host`
// object handed to modules. Shows exactly what the host will send: module, server, tool and the argument JSON.
// Approve and Reject (also Esc and ×) answer through `resolve_module_mcp_confirmation`. This is a guard against
// a module calling without the user knowing, not isolation: scripts in the same realm can call that command too
// (D050 residual risk).

function formatArguments(value: unknown): string {
  try {
    return JSON.stringify(value, null, 2) ?? 'null';
  } catch {
    return 'null';
  }
}

export default function ModuleMcpConfirm({ host = moduleRuntimeHost }: { host?: ModuleRuntimeHost }) {
  const { t } = useLocale();
  const pending = useSyncExternalStore(host.subscribeMcpConfirmations, host.getMcpConfirmations);
  const current = pending[0];
  if (current === undefined) return null;
  const answer = (approved: boolean) => {
    void host.resolveMcpConfirmation(current.confirmationId, approved);
  };
  return (
    <Modal
      key={current.confirmationId}
      open
      kind="confirm"
      icon="warning"
      title={t('module.mcpTitle')}
      description={
        <>
          {t('module.mcpDescription', { name: current.moduleName, server: current.serverId, tool: current.toolName })}
          {pending.length > 1 ? t('module.mcpQueued', { count: pending.length - 1 }) : null}
        </>
      }
      onOpenChange={(next) => {
        // Esc and × reject. Approve and Reject below answer first, so a second answer is a no-op.
        if (!next) answer(false);
      }}
      footer={
        <>
          <Button variant="secondary" onClick={() => answer(false)}>
            {t('module.mcpReject')}
          </Button>
          <Button onClick={() => answer(true)}>{t('module.mcpApprove')}</Button>
        </>
      }
    >
      <pre
        aria-label={t('module.mcpArgs')}
        className="max-h-60 overflow-auto whitespace-pre-wrap rounded-md border border-input bg-muted p-3 text-xs"
      >
        {formatArguments(current.arguments)}
      </pre>
    </Modal>
  );
}
