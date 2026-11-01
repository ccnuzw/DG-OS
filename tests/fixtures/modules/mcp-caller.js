export default function createModule(host) {
  const React = host.React;
  // Exposed so a harness can inspect everything a module can reach and drive `mcp.callTool` from outside
  // the component. A real module has no such hook; the realm is shared (D042).
  (globalThis.__dgosMcpCallers ??= {})[host.moduleId] = {
    host,
    call: (serverId, toolName, args) => host.mcp.callTool(serverId, toolName, args),
  };
  function Component() {
    return React.createElement('div', { 'data-testid': 'mcp-caller' }, 'mcp-caller');
  }
  return { Component };
}
