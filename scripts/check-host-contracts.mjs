import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

// Static contract checks supplement compilation; they do not prove WebView isolation.
const files = ['provider_commands', 'skills', 'mcp', 'workbench'];
let commands = 0;
for (const file of files) {
  const source = readFileSync(new URL(`../src-tauri/src/commands/${file}.rs`, import.meta.url), 'utf8');
  const handlers = [...source.matchAll(/#\[tauri::command\]\s+pub async fn (\w+)\(([\s\S]*?)\) -> [\s\S]*?\{([\s\S]*?)(?=\n#\[|\npub struct |\n#\[cfg|$)/g)];
  assert.ok(handlers.length, `${file}: no command handlers checked`);
  for (const [, name, args, body] of handlers) {
    assert.match(args, /window:\s*WebviewWindow/, `${name}: host window context is required`);
    assert.doesNotMatch(args, /\b(?:user_id|module_id|caller)\s*:/, `${name}: identity belongs to the host`);
    assert.match(body, /host::context\(\s*&window,/, `${name}: missing host entry gate`);
    commands += 1;
  }
}
const registered = readFileSync(new URL('../src-tauri/src/lib.rs', import.meta.url), 'utf8')
  .match(/commands::\w+,/g) ?? [];
assert.equal(commands, registered.length, 'all registered IPC handlers must be inspected');
console.log(`Checked ${commands} host IPC contracts (static only).`);
