import { useEffect } from 'react';
import { Link } from 'react-router-dom';
import { cn } from '@dgos/ui';
import { FOCUS_RING } from '@dgos/ui/lib/focus';
import { useMcpStore } from '../stores/mcpStore';
import { t, useLocale } from '../i18n';

// Connection recovery must remain visible even outside the management page.
export default function McpConnectionNotice() {
  useLocale();
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
  return <aside
    role="alert"
    aria-label={t('mcp.recoveryAria')}
    className="flex flex-wrap items-center justify-between gap-4 border-b border-status-warning bg-status-warning-surface px-6 py-3 text-sm text-status-warning-text"
  >
     <div className="grid min-w-0 gap-1 wrap-anywhere"><strong>{t('mcp.recoveryStopped')}</strong><span>{t('mcp.recoveryHelp', { name: unavailable.map(server => server.name).join(', ') })}</span></div>
    <Link
      to="/mcp"
      className={cn('shrink-0 font-bold underline', FOCUS_RING)}
    >{t('mcp.manageConnections')}</Link>
  </aside>;
}
