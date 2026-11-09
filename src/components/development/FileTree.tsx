import { useEffect, useRef, useState } from 'react';
import { Button, ContextMenu, DropdownMenu, type MenuItem } from '@dgos/ui';
import { ChevronDown, ChevronRight, Ellipsis, File, FileCode2, FileJson2, FileText, Folder, GitBranch } from 'lucide-react';
import { useLocale } from '../../i18n';
import { listDevelopmentFiles, type DevFileEntry } from '../../stores/developmentReview';
import { useDevelopment } from '../../stores/developmentStore';
import { contextGeneration } from '../../stores/developmentSelection';
import { useWorkbenchLocation } from './workbenchLocations';
import { errorText } from './shared';

interface Props {
  projectId: string; taskId: string | null; scope: string; entries: DevFileEntry[]; query: string; ignored: boolean;
  selected: string | null; refreshKey: number; collapseKey: number; onOpen: (path: string) => void; onDirectory: (path: string) => void;
  onSelect: (path: string) => void;
  menuItems: (entry: DevFileEntry) => MenuItem[];
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
  const FileIcon = isDirectory ? Folder : /\.(json|ya?ml)$/.test(entry.name) ? FileJson2 : /^\.git/.test(entry.name) ? GitBranch : /\.md$/.test(entry.name) ? FileText : /\.(tsx?|jsx?|rs|go|py|css|html)$/.test(entry.name) ? FileCode2 : File;
  return <div role="treeitem" aria-expanded={isDirectory ? open : undefined} aria-selected={props.selected === entry.path}>
    <ContextMenu items={props.menuItems(entry)} onOpenChange={open => { if (open) props.onSelect(entry.path); }}><div className="dev-file-browser-row" data-file-path={entry.path} data-file-kind={entry.kind} onFocus={() => props.onSelect(entry.path)}>
      <Button size="xs" variant="ghost" className="dev-file-entry" data-testid={`dev-file-item-${entry.path}`} disabled={entry.kind === 'symlink' || entry.kind === 'blocked'} onKeyDown={event => {
        if (event.key === 'ContextMenu' || event.shiftKey && event.key === 'F10') { event.preventDefault(); const bounds = event.currentTarget.getBoundingClientRect(); event.currentTarget.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, clientX: bounds.left, clientY: bounds.bottom })); }
        if (isDirectory && ['ArrowLeft', 'ArrowRight'].includes(event.key)) { event.preventDefault(); setExpanded(event.key === 'ArrowRight'); }
        if (['ArrowUp', 'ArrowDown', 'Home', 'End'].includes(event.key)) { event.preventDefault(); const tree = event.currentTarget.closest('[role=tree]'); const rows = [...(tree?.querySelectorAll<HTMLButtonElement>('.dev-file-entry:not(:disabled)') ?? [])]; const index = rows.indexOf(event.currentTarget); rows[event.key === 'Home' ? 0 : event.key === 'End' ? rows.length - 1 : Math.max(0, Math.min(rows.length - 1, index + (event.key === 'ArrowDown' ? 1 : -1)))]?.focus(); }
      }} onClick={() => { if (isDirectory) { setExpanded(!expanded); props.onDirectory(entry.path); } else props.onOpen(entry.path); }}>
        <span aria-hidden="true" className="dev-tree-icon">{isDirectory ? open ? <ChevronDown size={12} /> : <ChevronRight size={12} /> : null}</span><FileIcon aria-hidden="true" size={16} className={`dev-tree-file-icon${FileIcon === FileJson2 ? ' dev-tree-file-config' : FileIcon === FileText ? ' dev-tree-file-document' : FileIcon === GitBranch ? ' dev-tree-file-git' : ''}`} /><span className="dev-file-entry-name" title={entry.path}>{entry.name}</span>{entry.kind === 'symlink' || entry.kind === 'blocked' ? <span className="text-ink-secondary">{t(`review.kind.${entry.kind}`)}</span> : null}
      </Button><DropdownMenu label={t('workbenchFiles.actions', { path: entry.path })} trigger={<Button size="xs" variant="ghost" className="dev-file-more" aria-label={t('workbenchFiles.actions', { path: entry.path })}><Ellipsis size={14} aria-hidden="true" /></Button>} items={props.menuItems(entry)} />
    </div></ContextMenu>
    {isDirectory && open ? <div role="group" className="dev-file-tree-children">{loading ? <p role="status" className="text-xs">{t('review.loading')}</p> : null}{error ? <p role="alert" className="text-xs">{errorText(error)}</p> : null}{truncated ? <p role="status" className="text-xs">{t('review.filesTruncated')}</p> : null}{sortEntries(displayed).map(child => <FileNode key={child.path} entry={child} {...props} />)}</div> : null}
  </div>;
}
