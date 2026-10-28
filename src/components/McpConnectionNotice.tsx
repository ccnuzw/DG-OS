import { useEffect } from 'react';
import { Link } from 'react-router-dom';
import { useMcpStore } from '../stores/mcpStore';

// Connection recovery must remain visible even outside the management page.
export default function McpConnectionNotice() {
  const servers = useMcpStore(state => state.servers);
  const fetch = useMcpStore(state => state.fetch);
  useEffect(() => {
    void fetch();
    const timer = window.setInterval(() => {
      if (!useMcpStore.getState().pending.length) void fetch();
    }, 10000);
    return () => window.clearInterval(timer);
  }, [fetch]);
  const unavailable = servers.filter(server => server.enabled && server.reconnect_exhausted && server.status === 'error');
  if (!unavailable.length) return null;
  return <aside role="alert" className="mcp-connection-notice" aria-label="MCP connection recovery stopped">
    <div><strong>MCP automatic recovery stopped</strong><span>{unavailable.map(server => server.name).join(', ')} cannot connect. Check the configuration, then connect manually. No tool calls are replayed.</span></div>
    <Link to="/mcp">Manage connections</Link>
  </aside>;
}
