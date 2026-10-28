// Opt-in external SDK compatibility server, not a hand-written MCP protocol fixture.
// Install dependencies in an isolated prefix and pass it as argv[2].
import http from 'node:http';
import path from 'node:path';
import { createRequire } from 'node:module';
import { randomUUID } from 'node:crypto';

const require = createRequire(path.join(process.argv[2], 'package.json'));
const { McpServer } = require('@modelcontextprotocol/sdk/server/mcp.js');
const { StreamableHTTPServerTransport } = require('@modelcontextprotocol/sdk/server/streamableHttp.js');
const { SSEServerTransport } = require('@modelcontextprotocol/sdk/server/sse.js');
const { z } = require('zod');
const legacy = process.argv[3] === 'legacy';
const mcp = new McpServer({ name: 'dgos-sdk-compatibility', version: '1.0.0' });
mcp.registerTool('echo', {
  description: 'Echo synthetic compatibility data',
  inputSchema: { text: z.string() },
  outputSchema: { text: z.string() },
}, async ({ text }) => ({
  content: [{ type: 'text', text }],
  structuredContent: { text },
}));
let transport;
if (!legacy) {
  transport = new StreamableHTTPServerTransport({ sessionIdGenerator: randomUUID });
  await mcp.connect(transport);
}
const server = http.createServer(async (request, response) => {
  try {
    if (!legacy && request.url === '/mcp') {
      await transport.handleRequest(request, response);
    } else if (legacy && request.url === '/sse' && request.method === 'GET') {
      transport = new SSEServerTransport('/messages', response);
      await mcp.connect(transport);
    } else if (legacy && request.url === '/sse' && request.method === 'POST') {
      request.resume();
      response.writeHead(405).end();
    } else if (legacy && request.url?.startsWith('/messages?') && request.method === 'POST' && transport) {
      await transport.handlePostMessage(request, response);
    } else {
      response.writeHead(404).end();
    }
  } catch {
    if (!response.headersSent) response.writeHead(500).end();
    else response.end();
  }
});
server.listen(0, '127.0.0.1', () => console.log(`http://127.0.0.1:${server.address().port}/${legacy ? 'sse' : 'mcp'}`));
