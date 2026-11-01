import { useSyncExternalStore } from 'react';
import { Button, Modal } from '@dgos/ui';
import { moduleRuntimeHost, type ModuleRuntimeHost } from './moduleRuntimeHost';

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
      title="模块请求调用 MCP 工具"
      description={
        <>
          模块「{current.moduleName}」请求调用 MCP 服务 {current.serverId} 的工具 {current.toolName}。批准后将使用下面的参数调用一次。
          {pending.length > 1 ? `（另有 ${pending.length - 1} 个请求等待确认）` : null}
        </>
      }
      onOpenChange={(next) => {
        // Esc and × reject. Approve and Reject below answer first, so a second answer is a no-op.
        if (!next) answer(false);
      }}
      footer={
        <>
          <Button variant="secondary" onClick={() => answer(false)}>
            拒绝
          </Button>
          <Button onClick={() => answer(true)}>批准</Button>
        </>
      }
    >
      <pre
        aria-label="调用参数"
        className="max-h-60 overflow-auto whitespace-pre-wrap rounded-md border border-input bg-muted p-3 text-xs"
      >
        {formatArguments(current.arguments)}
      </pre>
    </Modal>
  );
}
