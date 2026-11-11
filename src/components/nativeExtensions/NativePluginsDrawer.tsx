import { useEffect, useRef } from 'react';
import { Button, StatusBadge } from '@dgos/ui';
import { BookOpen, ChevronRight, Puzzle, Server } from 'lucide-react';
import { t } from '../../i18n';
import { useDevelopment } from '../../stores/developmentStore';
import { nativeSelectionIdentity, useNativeExtensions } from '../../stores/nativeExtensions';
import type { NativeSelection } from '../../types/nativeExtensions';
import './native-manager.css';

export interface NativePluginsDrawerProps {
  selection: NativeSelection;
  sessionId?: string;
  onManage(kind: 'skills' | 'mcp', resourceId?: string): void;
}

/** File-backed summaries only: opening the drawer never starts or observes a CLI. */
export default function NativePluginsDrawer({ selection, sessionId, onManage }: NativePluginsDrawerProps) {
  const epoch = useDevelopment(state => state.epoch);
  const projects = useDevelopment(state => state.projects);
  const api = useNativeExtensions();
  const key = JSON.stringify([epoch, selection.target, selection.scope, selection.project_id, selection.task_id ?? null]);
  const rootIdentity = useDevelopment(() => JSON.stringify([key, nativeSelectionIdentity(selection)]));
  const mcp = api.mcp[key], skills = api.skills[`${key}:`];
  const skillsSupported = selection.scope !== 'ProjectLocal';
  const valid = selection.scope === 'OsUser' || projects.some(project => project.id === selection.project_id && !project.archived);
  const lastRefresh = useRef<{ root: string; mcp: boolean; skills: boolean } | null>(null);
  useEffect(() => {
    if (!valid) return;
    const previous = lastRefresh.current;
    lastRefresh.current = { root: rootIdentity, mcp: Boolean(mcp), skills: Boolean(skills) };
    if (previous?.root === rootIdentity && !(previous.mcp && !mcp) && !(skillsSupported && previous.skills && !skills)) return;
    const current = useNativeExtensions.getState();
    void current.discoverMcp(selection);
    if (skillsSupported) void current.discoverSkills({ ...selection, query: '' });
  }, [rootIdentity, valid, skillsSupported, Boolean(mcp), Boolean(skills)]);
  return <aside className="native-plugins-drawer" aria-label={t('nativeExtensions.plugins')}>
    <header><span className="native-resource-icon" aria-hidden="true"><Puzzle /></span><div><h2>{t('nativeExtensions.plugins')}</h2><p>{selection.target === 'Claude' ? 'Claude Code' : 'Codex'} · {t(`nativeExtensions.scope.${selection.scope}`)}</p></div></header>
    {!valid && <p role="alert">{t('nativeExtensions.error.project_not_found')}</p>}
    {(['mcp', 'skills'] as const).map(kind => {
      const state = kind === 'mcp' ? mcp : skills;
      const count = !valid ? undefined : kind === 'mcp' ? mcp?.value?.instances.length : skills?.value ? skills.value.instances.length + skills.value.readonly_entries.length : undefined;
      const currentCount = state?.value && !state.loading && !state.error && !state.stale && !state.value.incomplete ? count : undefined;
      const unsupported = kind === 'skills' && !skillsSupported;
      return <section key={kind} className="native-plugins-summary" aria-label={t(kind === 'mcp' ? 'nativeExtensions.configuredServers' : 'nativeExtensions.installedSkills')}>
        <header><h3>{t(kind === 'mcp' ? 'nativeExtensions.configuredServers' : 'nativeExtensions.installedSkills')} <span aria-label={t('nativeExtensions.count')}>{unsupported || currentCount === undefined ? '—' : currentCount}</span></h3><Button variant="ghost" disabled={!valid} onClick={() => onManage(kind)}>{t('nativeExtensions.manage')}</Button></header>
        {!valid ? null : unsupported ? <p>{t('nativeExtensions.localRulesOnly')}</p> : <>
          {(!state || state.loading) && valid && <p role="status">{t('nativeExtensions.loading')}</p>}
          {state?.loading && state.value && <p role="status">{t('nativeExtensions.refreshingCached')}</p>}
          {state?.error && <div role="alert"><p>{t(`nativeExtensions.error.${state.error}`)}</p><Button variant="secondary" disabled={state.loading} onClick={() => void (kind === 'mcp' ? api.discoverMcp(selection) : api.discoverSkills({ ...selection, query: '' }))}>{t('nativeExtensions.refresh')}</Button></div>}
          {state?.stale && state.value && !state.loading && <p role="status">{t('nativeExtensions.stale')}</p>}
          {state?.value?.incomplete && <p role="status">{t('nativeExtensions.incomplete')}</p>}
          {state?.value && count === 0 && !state.loading && !state.error && !state.stale && !state.value.incomplete && <p>{t(kind === 'mcp' ? 'nativeExtensions.noConfiguredServers' : 'nativeExtensions.noInstalledSkills')}</p>}
          {kind === 'mcp' ? <ul>{mcp?.value?.instances.slice(0, 4).map(server => <li key={server.native_key}><button type="button" className="native-plugins-item" onClick={() => onManage('mcp', server.native_key)}><Server aria-hidden="true" /><span><strong>{server.native_key}</strong><span>{server.transport} · {t('nativeExtensions.configEnabled')}: {t(server.enabled === null ? 'nativeExtensions.notReported' : server.enabled ? 'nativeExtensions.yes' : 'nativeExtensions.no')}</span>{server.owner !== 'Loose' && <StatusBadge status="neutral" label={t(`nativeExtensions.owner.${server.owner}`)}/>}</span><ChevronRight aria-hidden="true" /></button></li>)}</ul> : <ul>{skills?.value?.instances.slice(0, 4).map(skill => <li key={skill.instance.id}><button type="button" className="native-plugins-item" onClick={() => onManage('skills', skill.instance.id)}><BookOpen aria-hidden="true" /><span><strong>{skill.metadata.name}</strong><span>{skill.metadata.description}</span>{skill.owner !== 'Loose' && <StatusBadge status="neutral" label={t(`nativeExtensions.owner.${skill.owner}`)}/>}</span><ChevronRight aria-hidden="true" /></button></li>)}{skills?.value?.readonly_entries.slice(0, Math.max(0, 4 - skills.value.instances.length)).map(entry => <li key={entry.id}><strong>{entry.slug}</strong><span>{t('nativeExtensions.owner.Symlink')}</span></li>)}</ul>}
          {count !== undefined && count > 4 && <Button variant="ghost" onClick={() => onManage(kind)}>{t('nativeExtensions.moreResources', { count: count - 4 })}</Button>}
        </>}
      </section>;
    })}
    <footer className="native-plugins-footer"><p>{sessionId ? t('nativeExtensions.drawerSessionNotice') : t('nativeExtensions.noActiveSession')}</p><details><summary>{t('nativeExtensions.managerBoundary')}</summary><p>{t('nativeExtensions.drawerProviderNotice')}</p></details></footer>
  </aside>;
}
