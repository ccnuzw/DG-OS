// V2-FR-009 AC15/AC16 (UI half): the outer tab bar and the shell frame.
// jsdom does no layout; tab geometry and scrolling still require browser verification.
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { MemoryRouter, Link, useLocation } from 'react-router-dom';
import type { ReactNode } from 'react';
import { AppShell } from '../src/workspace/AppShell';
import { TabBar } from '../src/workspace/TabBar';
import type { WorkspaceSurface } from '../src/workspace/model';
import { useWorkspaceTabs } from '../src/workspace/useWorkspaceTabs';
import { resetLocaleForTesting, setLocaleForTesting } from '../src/i18n';
import { WorkspaceTools, Launcher, type NavigationAction } from '../src/workspace/WorkspaceTools';
import type { WorkspaceAppEntry } from '../src/modules/moduleApi';

// vitest runs without globals here, so Testing Library's auto-cleanup does not run by itself.
afterEach(() => {
  cleanup();
  resetLocaleForTesting();
});

const tabProps = {
  surfaces: [] as WorkspaceSurface[],
  activeSurfaceId: null as string | null,
  onActivate: () => {},
  onClose: () => {},
};

const aiSurface: WorkspaceSurface = { kind: 'ai', id: 'ai' };
const appSurface = (title: string): WorkspaceSurface => ({
  kind: 'app',
  id: title,
  moduleId: title,
  viewId: title,
  documentId: null,
  title,
});

describe('live command and launcher inputs', () => {
  it('keeps a command selected by identity and recovers when that command disappears', () => {
    const run = vi.fn();
    const action = (id: string): NavigationAction => ({ id, label: id, group: 'apps', icon: null, run: () => run(id) });
    const first = action('first'), second = action('second'), last = action('last');
    const { rerender } = render(<WorkspaceTools open actions={[first, second, last]} onClose={() => {}} />);
    fireEvent.mouseEnter(screen.getByRole('option', { name: 'second' }));
    rerender(<WorkspaceTools open actions={[second, last]} onClose={() => {}} />);
    fireEvent.keyDown(screen.getByRole('combobox'), { key: 'Enter' });
    expect(run).toHaveBeenLastCalledWith('second');
    rerender(<WorkspaceTools open actions={[last]} onClose={() => {}} />);
    fireEvent.keyDown(screen.getByRole('combobox'), { key: 'Enter' });
    expect(run).toHaveBeenLastCalledWith('last');
  });

  it('lets a pinned unavailable app be unpinned from its actions menu', async () => {
    const entry: WorkspaceAppEntry = { moduleId: 'revoked', displayName: 'Revoked app', iconResourceId: null, layout: 'document', viewCapability: 'single', enabled: true, permissionState: 'denied', runtimeState: 'ready', crashed: false };
    const onUnpin = vi.fn();
    render(<Launcher open query="" catalog={[entry]} loading={false} error={false} pinnedIds={[entry.moduleId]} activeDrag={null} notice="" onRetry={() => {}} onManage={() => {}} onQuery={() => {}} onClose={() => {}} onOpen={() => {}} onPin={() => {}} onUnpin={onUnpin} onMove={() => {}} onDragStart={() => {}} onDragEnd={() => {}} />);
    fireEvent.pointerDown(screen.getByRole('button', { name: 'Revoked app 的操作' }), { button: 0, ctrlKey: false });
    expect(await screen.findByRole('menuitem', { name: '取消固定' })).toBeTruthy();
    expect(screen.getByRole('menuitem', { name: '向前移动' }).getAttribute('aria-disabled')).toBe('true');
    expect(screen.getByRole('menuitem', { name: '向后移动' }).getAttribute('aria-disabled')).toBe('true');
    fireEvent.click(screen.getByRole('menuitem', { name: '取消固定' }));
    expect(onUnpin).toHaveBeenCalledWith('revoked');
  });

  it('moves from launcher search to apps without the removed pin binding', () => {
    const entry: WorkspaceAppEntry = { moduleId: 'ready', displayName: 'Ready app', iconResourceId: null, layout: 'document', viewCapability: 'single', enabled: true, permissionState: 'granted', runtimeState: 'ready', crashed: false };
    const onPin = vi.fn();
    const onOpen = vi.fn();
    render(<Launcher open query="" catalog={[entry]} loading={false} error={false} pinnedIds={[]} activeDrag={null} notice="" onRetry={() => {}} onManage={() => {}} onQuery={() => {}} onClose={() => {}} onOpen={onOpen} onPin={onPin} onUnpin={() => {}} onMove={() => {}} onDragStart={() => {}} onDragEnd={() => {}} />);
    const search = screen.getByRole('textbox', { name: '搜索应用' });
    const app = screen.getByRole('button', { name: 'Ready app' });
    fireEvent.keyDown(search, { key: 'ArrowDown' });
    expect(document.activeElement).toBe(app);
    fireEvent.keyDown(app, { key: 'p' });
    expect(onPin).not.toHaveBeenCalled();
    fireEvent.keyDown(search, { key: 'Enter', isComposing: true });
    expect(onOpen).not.toHaveBeenCalled();
    fireEvent.keyDown(search, { key: 'Enter' });
    expect(onOpen).toHaveBeenCalledWith(entry);
  });
});

describe('TabBar (AC15)', () => {
  it('renders one tab per surface and marks the active one', () => {
    render(
      <TabBar {...tabProps} surfaces={[aiSurface, appSurface('无限画布')]} activeSurfaceId="无限画布" />,
    );
    const tabs = screen.getAllByRole('tab');
    expect(tabs).toHaveLength(2);
    expect(tabs[0].getAttribute('aria-selected')).toBe('false');
    expect(tabs[1].getAttribute('aria-selected')).toBe('true');
    // Only the active tab is currently in the tab order.
    expect(tabs[0].getAttribute('tabindex')).toBe('-1');
  });

  it('activates a tab on click', () => {
    const onActivate = vi.fn();
    render(<TabBar {...tabProps} surfaces={[aiSurface, appSurface('技能中心')]} activeSurfaceId="ai" onActivate={onActivate} />);
    fireEvent.click(screen.getByRole('tab', { name: /技能中心/ }));
    expect(onActivate).toHaveBeenCalledWith('技能中心');
  });

  it('exposes a named close control per tab', () => {
    const onClose = vi.fn();
    render(<TabBar {...tabProps} surfaces={[appSurface('无限画布')]} activeSurfaceId="无限画布" onClose={onClose} />);
    fireEvent.click(screen.getByRole('button', { name: '关闭 无限画布' }));
    expect(onClose).toHaveBeenCalledWith('无限画布');
  });

  it('shows a status dot only for running app surfaces', () => {
    render(
      <TabBar
        {...tabProps}
        surfaces={[aiSurface, appSurface('模型调试台')]}
        activeSurfaceId="ai"
        runningModuleIds={['模型调试台']}
      />,
    );
    const runningTab = screen.getByRole('tab', { name: /模型调试台/ });
    expect(within(runningTab).getByRole('img', { name: '运行中' })).toBeTruthy();
    expect(within(screen.getByRole('tab', { name: /AI 会话/ })).queryByRole('img', { name: '运行中' })).toBeNull();
  });

  it('toggles focus from the bar and reflects it through aria-pressed', () => {
    const onToggleFocus = vi.fn();
    const { rerender } = render(
      <TabBar {...tabProps} surfaces={[appSurface('无限画布')]} activeSurfaceId="无限画布" onToggleFocus={onToggleFocus} />,
    );
    const focus = screen.getByRole('button', { name: /专注/ });
    expect(focus.getAttribute('aria-pressed')).toBe('false');
    fireEvent.click(focus);
    expect(onToggleFocus).toHaveBeenCalledWith('无限画布');

    rerender(
      <TabBar
        {...tabProps}
        surfaces={[appSurface('无限画布')]}
        activeSurfaceId="无限画布"
        focusedSurfaceId="无限画布"
        onToggleFocus={onToggleFocus}
      />,
    );
    expect(screen.getByRole('button', { pressed: true })).toBeTruthy();
  });

  it('offers no focus control when nothing is open', () => {
    render(<TabBar {...tabProps} onToggleFocus={() => {}} />);
    expect(screen.queryByRole('button', { name: /专注/ })).toBeNull();
  });
});

describe('AppShell (AC15, AC17)', () => {
  function Router({ children }: { children: ReactNode }) {
    return <MemoryRouter initialEntries={['/workbench']} future={{ v7_startTransition: true, v7_relativeSplatPath: true }}>{children}</MemoryRouter>;
  }
  const baseProps = {
    navEntries: [
      { id: 'desktop', label: '桌面总览', to: '/', end: true, icon: <span /> },
      { id: 'ai', label: 'AI 会话', to: '/workbench', icon: <span /> },
    ],
    tabs: { ...tabProps, surfaces: [aiSurface], activeSurfaceId: 'ai' },
    children: <div>工作区内容</div>,
  };

  it('renders the rail, the tab bar and the content area', () => {
    render(<AppShell {...baseProps} />, { wrapper: Router });
    expect(screen.getByRole('complementary', { name: '主导航' })).toBeTruthy();
    expect(screen.getByRole('tab', { name: /AI 会话/ })).toBeTruthy();
    expect(screen.getByText('工作区内容')).toBeTruthy();
    expect(screen.getByRole('link', { name: 'AI 会话' }).getAttribute('aria-current')).toBe('page');
    expect(screen.getByRole('link', { name: '桌面总览' }).getAttribute('aria-current')).toBeNull();
  });

  it('collapses to icons and hides the labels', () => {
    render(<AppShell {...baseProps} collapsed onToggleCollapsed={() => {}} />, { wrapper: Router });
    expect(screen.queryByText('我的工作空间')).toBeNull();
    // The entry keeps its accessible name through the title attribute while the text is gone.
    expect(screen.getByRole('link', { name: '桌面总览' }).getAttribute('href')).toBe('/');
  });

  it('reports and toggles the collapse state', () => {
    const onToggle = vi.fn();
    const { rerender } = render(<AppShell {...baseProps} onToggleCollapsed={onToggle} />, { wrapper: Router });
    const collapse = screen.getByRole('button', { name: '收起导航' });
    expect(collapse.getAttribute('aria-expanded')).toBe('true');
    fireEvent.click(collapse);
    expect(onToggle).toHaveBeenCalled();

    rerender(<AppShell {...baseProps} collapsed onToggleCollapsed={onToggle} />);
    const expand = screen.getByRole('button', { name: '展开导航' });
    expect(expand.getAttribute('aria-expanded')).toBe('false');
  });

  it('hides the rail for full-bleed surfaces but keeps the tab bar', () => {
    render(<AppShell {...baseProps} railHidden />, { wrapper: Router });
    expect(screen.queryByRole('complementary', { name: '主导航' })).toBeNull();
    expect(screen.getByRole('tab', { name: /AI 会话/ })).toBeTruthy();
  });

  it('distinguishes the open launcher from the background route and restores selection', () => {
    const { rerender } = render(<AppShell {...baseProps} onOpenLauncher={() => {}} />, { wrapper: Router });
    const ai = screen.getByRole('link', { name: 'AI 会话' });
    expect(ai.classList.contains('is-active')).toBe(true);
    rerender(<AppShell {...baseProps} onOpenLauncher={() => {}} launcherOpen />);
    expect(ai.getAttribute('aria-current')).not.toBe('page');
    expect(ai.classList.contains('is-active')).toBe(false);
    expect(screen.getByRole('button', { name: '应用' }).getAttribute('aria-expanded')).toBe('true');
    rerender(<AppShell {...baseProps} onOpenLauncher={() => {}} />);
    expect(ai.getAttribute('aria-current')).toBe('page');
    expect(ai.classList.contains('is-active')).toBe(true);
  });

});

describe('workspace route projection (D120, D124)', () => {
  function Workspace() {
    const tabs = useWorkspaceTabs();
    const { pathname } = useLocation();
    return (
      <>
        <Link to="/workbench">AI</Link>
        <Link to="/skills">Skills</Link>
        <Link to="/mcp">MCP</Link>
        <Link to="/">Desktop</Link>
        <Link to="/settings/modules">Settings</Link>
        <Link to="/account">Account</Link>
        <Link to="/unknown">Unknown</Link>
        <output aria-label="path">{pathname}</output>
        <output aria-label="AI identity">{tabs.surfaces.find(surface => surface.kind === 'ai')?.id}</output>
        <TabBar {...tabs} />
      </>
    );
  }

  it('retains singleton tabs and AI identity across routes, including special surfaces', () => {
    setLocaleForTesting('zh-CN');
    render(<MemoryRouter initialEntries={['/workbench']} future={{ v7_startTransition: true, v7_relativeSplatPath: true }}><Workspace /></MemoryRouter>);
    fireEvent.click(screen.getByRole('link', { name: 'Skills' }));
    expect(screen.getAllByRole('tab')).toHaveLength(2);
    expect(screen.getByRole('status', { name: 'AI identity' }).textContent).toBe('ai');
    fireEvent.click(screen.getByRole('link', { name: 'AI' }));
    expect(screen.getAllByRole('tab')).toHaveLength(2);

    for (const name of ['Desktop', 'Settings', 'Account', 'Unknown']) {
      fireEvent.click(screen.getByRole('link', { name }));
      expect(screen.getAllByRole('tab')).toHaveLength(2);
      expect(screen.getAllByRole('tab').every(tab => tab.getAttribute('aria-selected') === 'false')).toBe(true);
    }
    fireEvent.click(screen.getByRole('tab', { name: 'AI 会话' }));
    expect(screen.getByRole('status', { name: 'path' }).textContent).toBe('/workbench');
  });

  it('closes inactive tabs without navigating, selects a neighbour, and returns home after the last close', async () => {
    setLocaleForTesting('zh-CN');
    render(<MemoryRouter initialEntries={['/workbench']} future={{ v7_startTransition: true, v7_relativeSplatPath: true }}><Workspace /></MemoryRouter>);
    fireEvent.click(screen.getByRole('link', { name: 'Skills' }));
    fireEvent.click(screen.getByRole('link', { name: 'MCP' }));
    fireEvent.click(screen.getByRole('button', { name: '关闭 技能中心' }));
    expect(screen.getByRole('status', { name: 'path' }).textContent).toBe('/mcp');
    await waitFor(() => expect(screen.queryByRole('tab', { name: '技能中心' })).toBeNull());
    fireEvent.click(screen.getByRole('button', { name: '关闭 MCP 管理' }));
    await waitFor(() => expect(screen.getByRole('status', { name: 'path' }).textContent).toBe('/workbench'));
    fireEvent.click(screen.getByRole('button', { name: '关闭 AI 会话' }));
    await waitFor(() => expect(screen.getByRole('status', { name: 'path' }).textContent).toBe('/'));
    expect(screen.queryAllByRole('tab')).toEqual([]);
    fireEvent.click(screen.getByRole('link', { name: 'AI' }));
    expect(screen.getByRole('tab', { name: 'AI 会话' }).getAttribute('aria-selected')).toBe('true');
  });
});
