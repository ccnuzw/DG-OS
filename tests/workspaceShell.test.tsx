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
    expect(screen.getByRole('button', { name: /专注/ }).getAttribute('aria-pressed')).toBe('true');
  });

  it('offers no focus control when nothing is open', () => {
    render(<TabBar {...tabProps} onToggleFocus={() => {}} />);
    expect(screen.queryByRole('button', { name: /专注/ })).toBeNull();
  });
});

describe('AppShell (AC15, AC17)', () => {
  function Router({ children }: { children: ReactNode }) {
    return <MemoryRouter initialEntries={['/workbench']}>{children}</MemoryRouter>;
  }
  const baseProps = {
    navEntries: [
      { id: 'desktop', label: '桌面总览', to: '/', end: true, icon: <span /> },
      { id: 'ai', label: 'AI 会话', to: '/workbench', icon: <span /> },
    ],
    accountLabel: '本地用户',
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

  it('renders pinned apps in their own group', () => {
    render(<AppShell {...baseProps} pinnedEntries={[{ id: 'canvas', label: '无限画布', to: '/settings/modules/canvas', icon: <span /> }]} />, { wrapper: Router });
    expect(screen.getByText('应用')).toBeTruthy();
    expect(screen.getByRole('link', { name: '无限画布' }).getAttribute('href')).toBe('/settings/modules/canvas');
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
    render(<MemoryRouter initialEntries={['/workbench']}><Workspace /></MemoryRouter>);
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
    render(<MemoryRouter initialEntries={['/workbench']}><Workspace /></MemoryRouter>);
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
