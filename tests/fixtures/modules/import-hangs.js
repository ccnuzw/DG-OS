const probe = (globalThis.__dgosModuleProbe ??= { evaluated: 0, events: [] }); probe.evaluated += 1;
// Top-level await that never settles: import() hangs, so the page-side hook timer never starts and
// only the host's own start deadline can end the load. Real WebView smoke only: jsdom cannot run it.
await new Promise(() => {});
export default function createModule() {
  return { Component() { return null; } };
}
