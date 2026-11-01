export default function createModule(host) {
  const React = host.React;
  // Exposed so a harness can drive every host capability from outside the component, one function per
  // capability, each answering `{ok, value}` or `{ok:false, code}`. A real module has no such hook; the
  // realm is shared (D042).
  const run = (task) =>
    Promise.resolve()
      .then(task)
      .then(
        (value) => ({ ok: true, value: value === undefined ? null : value }),
        (error) => ({ ok: false, code: error && error.code }),
      );
  (globalThis.__dgosCapabilityCallers ??= {})[host.moduleId] = {
    storageGet: (key) => run(() => host.storage.get(key)),
    storageSet: (key, value) => run(() => host.storage.set(key, value)),
    storageDelete: (key) => run(() => host.storage.delete(key)),
    storageList: () => run(() => host.storage.list()),
    // Writes `count` values of `size` bytes under `prefix-N` until the host refuses; reports the first refusal.
    storageFill: (prefix, size, count) =>
      run(async () => {
        const value = 'x'.repeat(size);
        for (let index = 0; index < count; index += 1) {
          try {
            await host.storage.set(prefix + '-' + index, value);
          } catch (error) {
            return { written: index, code: error && error.code };
          }
        }
        return { written: count, code: null };
      }),
    networkRequest: (request) => run(() => host.network.request(request)),
    mcpCallTool: (serverId, toolName, args) => run(() => host.mcp.callTool(serverId, toolName, args)),
    skillsPrepare: (skillId) => run(() => host.skills.prepare(skillId)),
  };
  function Component() {
    return React.createElement('div', { 'data-testid': 'capability-caller' }, 'capability-caller');
  }
  return { Component };
}
