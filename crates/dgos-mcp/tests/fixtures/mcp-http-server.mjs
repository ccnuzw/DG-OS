import http from 'node:http';
const legacy = process.argv[2] === 'legacy';
const protocolVersion = legacy ? '2024-11-05' : '2025-03-26';
let startedCalls = 0;
let cancellations = 0;
let stream;
const clientReplies = new Map();
const server = http.createServer(async (request, response) => {
  if (!request.headers.authorization?.startsWith('Bearer test-only-token-')) {
    response.writeHead(401).end(); return;
  }
  if (!legacy && request.method === 'DELETE' && request.url === '/mcp') {
    const valid = request.headers['mcp-session-id'] === 'fixture-session'
      && request.headers['mcp-protocol-version'] === protocolVersion;
    response.writeHead(valid ? 204 : 400).end(); return;
  }
  if (legacy && request.url === '/sse') {
    if (request.method === 'POST') { request.resume(); response.writeHead(405).end(); return; }
    if (request.method !== 'GET') { response.writeHead(405).end(); return; }
    stream = response;
    response.writeHead(200, { 'content-type': 'text/event-stream' });
    response.write(': fixture heartbeat\r\n\r\nevent: endpoint\r\ndata: /messages?sessionId=fixture\r\n\r\n');
    request.on('close', () => { if (stream === response) stream = undefined; });
    return;
  }
  if (legacy && (request.method !== 'POST' || request.url !== '/messages?sessionId=fixture' || !stream)) {
    response.writeHead(404).end(); return;
  }
  const chunks = [];
  for await (const chunk of request) chunks.push(chunk);
  const message = JSON.parse(Buffer.concat(chunks).toString());
  if (message.method !== 'initialize' && request.headers['mcp-protocol-version'] !== protocolVersion) {
    response.writeHead(400).end(); return;
  }
  if (!message.method) { clientReplies.set(message.id, message); response.writeHead(202).end(); return; }
  if (message.method === 'notifications/initialized') { response.writeHead(202).end(); return; }
  if (!legacy && message.method !== 'initialize' && request.headers['mcp-session-id'] !== 'fixture-session') {
    response.writeHead(400).end(); return;
  }
  if (message.method === 'notifications/cancelled') { cancellations += 1; response.writeHead(202).end(); return; }
  let result;
  switch (message.method) {
    case 'initialize': result = { protocolVersion, capabilities: { tools: {} }, serverInfo: { name: 'http-fixture', version: '1' } }; break;
    case 'tools/list':
      if (legacy) {
        for (const [id, method] of [['legacy-ping', 'ping'], ['legacy-unsupported', 'fixture/notSupported']]) {
          stream.write(`event: message\ndata: ${JSON.stringify({ jsonrpc: '2.0', id, method })}\n\n`);
        }
      }
      result = { tools: [{ name: 'echo', inputSchema: { type: 'object', required: ['text'], properties: { text: { type: 'string' }, delay: { type: 'integer', minimum: 0 } }, additionalProperties: false } }] }; break;
    case 'tools/call': {
      if (message.params.arguments.text === 'fixture-counts') {
        result = { content: [{ type: 'text', text: JSON.stringify({ started: startedCalls, cancellations, pingReplied: Boolean(clientReplies.get('legacy-ping')?.result), unsupportedCode: clientReplies.get('legacy-unsupported')?.error?.code }) }], isError: false };
        break;
      }
      startedCalls += 1;
      await new Promise(resolve => setTimeout(resolve, message.params.arguments.delay ?? 0));
      result = { content: [{ type: 'text', text: message.params.arguments.text }], isError: false }; break;
    }
    default: result = {};
  }
  if (legacy) {
    response.writeHead(202).end();
    stream?.write(`event: message\ndata: ${JSON.stringify({ jsonrpc: '2.0', id: message.id, result })}\n\n`);
    return;
  }
  response.writeHead(200, { 'content-type': 'application/json', 'mcp-session-id': 'fixture-session' });
  response.end(JSON.stringify({ jsonrpc: '2.0', id: message.id, result }));
});
server.listen(0, '127.0.0.1', () => console.log(`http://127.0.0.1:${server.address().port}/${legacy ? 'sse' : 'mcp'}`));
