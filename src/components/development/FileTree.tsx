import { useEffect, useRef, useState, type ReactNode } from 'react';
import { Button } from '@dgos/ui';
import { useLocale } from '../../i18n';
import { listDevelopmentFiles, type DevFileEntry } from '../../stores/developmentReview';
import { useDevelopment } from '../../stores/developmentStore';
import { contextGeneration } from '../../stores/developmentSelection';
import { useWorkbenchLocation } from './workbenchLocations';
import { errorText } from './shared';

interface Props {
  projectId: string; taskId: string | null; scope: string; entries: DevFileEntry[]; query: string; ignored: boolean;
  selected: string | null; refreshKey: number; collapseKey: number; onOpen: (path: string) => void; onDirectory: (path: string) => void;
  referenceActions?: (entry: DevFileEntry) => ReactNode;
}
export function FileTree(props: Props) {
  const { t } = useLocale();
  const roots = new Map<string, DevFileEntry>();
  for (const entry of props.entries) {
    if (!props.query || !entry.path.includes('/')) roots.set(entry.path, entry);
    else {
      const name = entry.path.split('/')[0];
      roots.set(name, { path: name, name, kind: 'directory', size: 0, ignored: false });
    }
  }
  return <div role="tree" aria-label={t('review.files')}>{sortEntries([...roots.values()]).map(entry => <FileNode key={`${props.scope}:${props.collapseKey}:${entry.path}`} entry={entry} {...props} />)}</div>;
}
function sortEntries(entries: DevFileEntry[]) {
  return [...entries].sort((a, b) => Number(b.kind === 'directory') - Number(a.kind === 'directory') || a.name.localeCompare(b.name));
}
function FileNode({ entry, ...props }: Props & { entry: DevFileEntry }) {
  const { t } = useLocale();
  const [expanded, setExpanded] = useWorkbenchLocation(`${props.scope}:expanded:${props.collapseKey}:${entry.path}`, false);
  const [children, setChildren] = useState<DevFileEntry[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [truncated, setTruncated] = useState(false);
  const [loading, setLoading] = useState(false);
  const request = useRef(0);
  const isDirectory = entry.kind === 'directory';
  const open = props.query ? true : expanded;
  useEffect(() => {
    if (!isDirectory || !open || props.query) return;
    const n = ++request.current, epoch = useDevelopment.getState().epoch, generation = contextGeneration();
    setLoading(true); setError(null); setChildren(null);
    void listDevelopmentFiles({ project_id: props.projectId, task_id: props.taskId, path: entry.path, query: '', include_ignored: props.ignored }).then(result => {
      if (n !== request.current || epoch !== useDevelopment.getState().epoch || generation !== contextGeneration()) return;
      setLoading(false);
      if (result.ok) { setChildren(result.value.entries); setTruncated(result.value.truncated); } else setError(result.code);
    });
    return () => { ++request.current; };
  }, [isDirectory, open, props.projectId, props.taskId, props.ignored, props.refreshKey, props.query, entry.path]);
  let displayed = children ?? [];
  if (props.query) {
    const prefix = `${entry.path}/`, descendants = new Map<string, DevFileEntry>();
    for (const item of props.entries) {
      if (!item.path.startsWith(prefix)) continue;
      const name = item.path.slice(prefix.length).split('/')[0], path = prefix + name;
      descendants.set(path, item.path === path ? item : { path, name, kind: 'directory', size: 0, ignored: false });
    }
    displayed = [...descendants.values()];
  }
  return <div role="treeitem" aria-expanded={isDirectory ? open : undefined} aria-selected={props.selected === entry.path}>
    <div className="dev-file-browser-row"><Button size="xs" variant={props.selected === entry.path ? 'secondary' : 'ghost'} className="dev-file-entry" data-testid={`dev-file-item-${entry.path}`} disabled={entry.kind === 'symlink' || entry.kind === 'blocked'} onClick={() => { if (isDirectory) { setExpanded(!expanded); props.onDirectory(entry.path); } else props.onOpen(entry.path); }}>
      <span aria-hidden="true" className="dev-tree-icon">{isDirectory ? open ? '▾' : '▸' : null}</span><svg aria-hidden="true" className="dev-tree-file-icon" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.2">{isDirectory ? <path d="M1.5 4h5l1.5 1.5h6.5v8h-13z M1.5 4V2.5h5L8 4" /> : <path d="M3 1.5h6l4 4v9H3z M9 1.5v4h4" />}</svg><span>{entry.name}</span><span className="text-ink-secondary">{entry.ignored ? t('review.ignored') : ''}{entry.kind === 'symlink' || entry.kind === 'blocked' ? t(`review.kind.${entry.kind}`) : ''}</span>
    </Button>{props.referenceActions?.(entry)}</div>
    {isDirectory && open ? <div role="group" className="dev-file-tree-children">{loading ? <p role="status" className="text-xs">{t('review.loading')}</p> : null}{error ? <p role="alert" className="text-xs">{errorText(error)}</p> : null}{truncated ? <p role="status" className="text-xs">{t('review.filesTruncated')}</p> : null}{sortEntries(displayed).map(child => <FileNode key={child.path} entry={child} {...props} />)}</div> : null}
  </div>;
}
