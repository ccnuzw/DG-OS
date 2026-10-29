const probe = (globalThis.__dgosModuleProbe ??= { evaluated: 0, events: [] }); probe.evaluated += 1;
export default function createModule(host) {
  const React = host.React;
  // Exposed so a harness can drive calls from outside the component and keep a closure of an old
  // instance after it was unloaded. A real module has no such hook; the realm is shared (D042).
  const call = (providerId, prompt) =>
    host.providers
      .call(providerId, 'fixture-chat', { messages: [{ role: 'user', content: prompt }], max_tokens: 16, stream: false })
      .then(
        (response) => ({ ok: true, content: response.content }),
        (error) => ({ ok: false, code: error && error.code }),
      );
  (globalThis.__dgosCallers ??= {})[host.moduleId] = call;
  function Component() {
    return React.createElement('div', { 'data-testid': 'provider-caller' }, 'provider-caller');
  }
  return {
    Component,
    async onLoad() {
      probe.events.push('load:' + host.moduleId);
      // No route exists before the runtime is ready, so a call during onLoad must be refused.
      const early = await call('none', 'early');
      probe.events.push('early:' + (early.ok ? 'allowed' : early.code));
    },
    onUnload() { probe.events.push('unload:' + host.moduleId); },
  };
}
