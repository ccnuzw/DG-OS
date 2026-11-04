import { useEffect } from 'react';
import { useNavigate } from 'react-router-dom';
import { Button, EmptyState, Icon } from '@dgos/ui';
import { useChatStore } from '../stores/chatStore';
import { useLocale, formatDateTime } from '../i18n';
import { appUsable } from '../workspace/WorkspaceTools';
import type { WorkspaceAppEntry } from '../modules/moduleApi';
import './Desktop.css';
import { useDesktopPreferences } from '../workspace/desktopPreferences';

export default function Desktop({ entries, onOpenApps, onOpenApp }: { entries: WorkspaceAppEntry[]; onOpenApps: () => void; onOpenApp: (entry: WorkspaceAppEntry) => void }) {
  const { t } = useLocale();
  const navigate = useNavigate();
  const sessions = useChatStore(state => state.sessions);
  const loading = useChatStore(state => state.sessionsLoading);
  const loaded = useChatStore(state => state.sessionsLoaded);
  const error = useChatStore(state => state.sessionsError);
  const generations = useChatStore(state => state.generations);
  const preferences = useDesktopPreferences();
  const { showEntries, showRecent } = preferences.values;
  useEffect(() => { if (!loaded && !loading) void useChatStore.getState().loadSessions(); }, [loaded, loading]);
  const shortcuts = [
    { path: '/workbench', icon: 'file', label: t('nav.workbench') },
    { path: '/tasks/single', icon: 'code', label: t('workbench.single') },
    { path: '/tasks/agent', icon: 'code', label: t('workbench.agent') },
    { path: '/skills', icon: 'file', label: t('nav.skills') },
    { path: '/mcp', icon: 'globe', label: t('nav.mcp') },
  ];
  return <main className="desktop-page">
    <header className="desktop-header"><div><h1>{t('home.title')}</h1><p>{t('home.subtitle')}</p></div><details><summary>{t('home.organize')}</summary><label><input type="checkbox" disabled={!preferences.loaded} checked={showEntries} onChange={e => preferences.patch({ showEntries: e.target.checked })} />{t('home.entries')}</label><label><input type="checkbox" disabled={!preferences.loaded} checked={showRecent} onChange={e => preferences.patch({ showRecent: e.target.checked })} />{t('home.recent')}</label></details></header>
    <div className="desktop-overview"><section className="desktop-start"><h2>{t('chat.start')}</h2><p>{t('home.startHelp')}</p><Button onClick={() => navigate('/workbench')}>{t('nav.workbench')}</Button></section><section className="desktop-status"><h2>{t('home.available')}</h2><p>{t('home.appsCount', { count: entries.filter(appUsable).length })}</p><Button variant="secondary" size="xs" onClick={() => navigate('/settings/providers')}>{t('nav.providers')}</Button><Button variant="secondary" size="xs" onClick={() => navigate('/mcp')}>{t('nav.mcp')}</Button><Button variant="secondary" size="xs" onClick={() => navigate('/skills')}>{t('nav.skills')}</Button></section></div>
    {showEntries && <section aria-label={t('home.entries')} className="desktop-shortcuts">{shortcuts.map(item => <button key={item.path} className="desktop-shortcut" onClick={() => navigate(item.path)}><span className="desktop-icon"><Icon name={item.icon} /></span><span>{item.label}</span></button>)}{entries.map(entry => <button key={entry.moduleId} className="desktop-shortcut" disabled={!appUsable(entry)} onClick={() => onOpenApp(entry)}><span className="desktop-icon"><Icon name="menu" /></span><span>{entry.displayName}</span></button>)}<button className="desktop-shortcut" onClick={onOpenApps}><span className="desktop-icon"><Icon name="menu" /></span><span>{t('shell.openApps')}</span></button></section>}
    {showRecent && <section className="desktop-recent"><h2>{t('home.recent')}</h2>{loading && <p role="status">{t('common.loading')}</p>}{error ? <div role="alert"><p>{error}</p><Button variant="secondary" onClick={() => void useChatStore.getState().loadSessions()}>{t('common.retry')}</Button></div> : !loading && !sessions.length ? <EmptyState title={t('home.noRecent')} /> : <div className="desktop-recent-list">{sessions.slice(0, 6).map(session => <button key={session.id} onClick={() => { navigate('/workbench'); void useChatStore.getState().selectSession(session); }}><Icon name="file" /><span>{session.title}</span>{generations[session.id] && <span role="status">{t('workbench.running')}</span>}<time>{formatDateTime(session.updated_at * 1000)}</time></button>)}</div>}</section>}
  </main>;
}
