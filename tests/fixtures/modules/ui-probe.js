const probe = (globalThis.__dgosModuleProbe ??= { evaluated: 0, events: [] }); probe.evaluated += 1;
export default function createModule(host) {
  const h = host.React.createElement;
  // Exposed so the harness can inspect the exact object a module receives. A real module has no such hook.
  (globalThis.__dgosUiHosts ??= {})[host.moduleId] = host;
  function Component() {
    // `style` is passed on purpose: module code is untyped JS, the wrapper has to drop it at runtime.
    return h(
      host.ui.Stack,
      { gap: 2, 'data-testid': 'ui-probe-stack', style: { color: 'rgb(11, 22, 33)' } },
      h(host.ui.Button, { 'data-testid': 'ui-probe-button', className: 'probe-class', style: { color: 'rgb(11, 22, 33)' } }, 'Go'),
    );
  }
  return { Component };
}
