import readline from 'node:readline';

const mode = process.argv[2] ?? 'normal';
let initialized = false;
let listRound = 0;
const waitingCalls = [];
let startedCalls = 0;
const cancellations = new Set();
const clientReplies = new Map();
let pendingInitialize;
let activePressure = 0;
let maxPressure = 0;
const reply = (id, result) => process.stdout.write(`${JSON.stringify({ jsonrpc: '2.0', id, result })}\n`);
const tool = (name) => ({
  name, description: 'Untrusted external tool',
  inputSchema: {
    type: 'object', required: ['text'], additionalProperties: false,
    properties: { text: { type: 'string', minLength: 1 }, delay: { type: 'integer', minimum: 0 }, fail: { type: 'boolean' } },
  },
});
readline.createInterface({ input: process.stdin }).on('line', (line) => {
  const request = JSON.parse(line);
  if (mode === 'exit') process.exit(3);
  if (mode === 'malformed') { process.stdout.write('not json\n'); return; }
  if (mode === 'silent') return;
  if (!request.method) {
    clientReplies.set(request.id, request);
    if (mode === 'server-requests' && clientReplies.size === 2) {
      if (JSON.stringify(clientReplies.get('server-ping')?.result) !== '{}'
        || clientReplies.get('server-unsupported')?.error?.code !== -32601) process.exit(5);
      reply(pendingInitialize, { protocolVersion: '2025-06-18', capabilities: { tools: {} }, serverInfo: { name: 'fixture', version: '1' } });
    }
    return;
  }
  if (request.method === 'notifications/initialized') { initialized = true; return; }
  if (request.method === 'notifications/cancelled') { cancellations.add(request.params.requestId); return; }
  if (request.method === 'initialize') {
    if (mode === 'server-requests') {
      pendingInitialize = request.id;
      process.stdout.write(`${JSON.stringify({ jsonrpc: '2.0', id: 'server-ping', method: 'ping' })}\n`);
      process.stdout.write(`${JSON.stringify({ jsonrpc: '2.0', id: 'server-unsupported', method: 'fixture/notSupported' })}\n`);
      return;
    }
    const version = mode.startsWith('version-') ? mode.slice(8) : (mode === 'incompatible' ? '1900-01-01' : '2025-03-26');
    const result = { protocolVersion: version, capabilities: mode === 'no-tools' ? {} : { tools: {} }, serverInfo: { name: 'fixture', version: '1' } };
    if (mode === 'missing-capabilities') delete result.capabilities;
    if (mode === 'invalid-tools-capability') result.capabilities.tools = true;
    if (mode === 'missing-server-info') delete result.serverInfo;
    reply(request.id, result);
  } else if (request.method === 'ping') {
    if (mode === 'ping-silent') return;
    if (mode === 'ping-error') {
      process.stdout.write(`${JSON.stringify({ jsonrpc: '2.0', id: request.id, error: { code: -32603, message: 'fixture ping failure' } })}\n`);
      return;
    }
    reply(request.id, {});
  } else if (request.method === 'tools/list') {
    if (!initialized) { process.exit(4); }
    if (mode === 'no-tools') process.exit(6);
    if (mode === 'bad-schema') { reply(request.id, { tools: [{ name: 'broken', inputSchema: { type: 'impossible' } }] }); return; }
    const second = Boolean(request.params.cursor);
    if (!second) listRound += 1;
    if (mode === 'refresh-failure' && listRound === 2 && second) {
      process.stdout.write(`${JSON.stringify({ jsonrpc: '2.0', id: request.id, error: { code: -32603, message: 'fixture discovery failure' } })}\n`);
      return;
    }
    const changed = listRound > 1 && (mode === 'refresh-change' || mode === 'refresh-failure');
    const definition = tool(second ? (changed ? 'echo_changed' : 'echo_second') : (changed && mode === 'refresh-failure' ? 'echo_replacement' : 'echo'));
    if (mode.startsWith('output-')) definition.outputSchema = {
      type: 'object', required: ['text'], properties: { text: { type: 'string' } }, additionalProperties: false,
      ...(mode === 'output-change' && listRound > 1 ? { description: 'output-only revision' } : {}),
    };
    if (mode === 'bad-output-schema' || (mode === 'output-invalid-refresh' && listRound > 1)) definition.outputSchema = { type: 'impossible' };
    reply(request.id, { tools: [definition], ...((!second || mode === 'pagination-cycle') ? { nextCursor: 'page2' } : {}) });
  } else if (request.method === 'tools/call') {
    if (mode === 'cancellation-observable' && request.params.arguments.text === 'fixture-counts') {
      reply(request.id, { content: [{ type: 'text', text: JSON.stringify({ started: startedCalls, cancellations: cancellations.size }) }], isError: false });
      return;
    }
    startedCalls += 1;
    if (mode === 'pressure') {
      activePressure += 1; maxPressure = Math.max(maxPressure, activePressure);
      setTimeout(() => {
        activePressure -= 1;
        reply(request.id, { content: [{ type: 'text', text: request.params.arguments.text }], structuredContent: { maxPressure }, isError: false });
      }, 5);
      return;
    }
    if (mode.startsWith('output-') || mode === 'malformed-result') {
      const result = { content: [{ type: 'text', text: request.params.arguments.text }], isError: false };
      if (mode !== 'output-missing') result.structuredContent = { text: mode === 'output-wrong' ? 123 : request.params.arguments.text };
      if (mode === 'output-error') { result.isError = true; delete result.structuredContent; }
      if (mode === 'malformed-result') result.isError = 'false';
      reply(request.id, result); return;
    }
    if (mode === 'five-barrier') {
      waitingCalls.push(request);
      if (waitingCalls.length === 5) {
        for (const call of waitingCalls.reverse()) reply(call.id, { content: [{ type: 'text', text: call.params.arguments.text }], isError: false });
      }
      return;
    }
    // Wrong IDs and notifications must not consume another caller's response.
    reply(request.id + 1000000, { content: 'wrong id' });
    process.stdout.write(`${JSON.stringify({ jsonrpc: '2.0', method: 'notifications/message', params: {} })}\n`);
    setTimeout(() => reply(request.id, { content: [{ type: 'text', text: request.params.arguments.text }], isError: Boolean(request.params.arguments.fail) }), request.params.arguments.delay ?? 0);
  }
});
if (mode === 'delayed-stdin') {
  process.stdin.pause();
  setTimeout(() => process.stdin.resume(), 250);
}
