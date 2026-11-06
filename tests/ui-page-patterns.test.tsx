import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import {
  EmptyState,
  FilterBar,
  FixedActionBar,
  Notice,
  ResourceCard,
  ResourceDetail,
  ResourceList,
  SearchField,
  SectionHeader,
  StatusBadge,
} from '@dgos/ui';
import { setLocaleForTesting } from '../src/i18n/runtime';

afterEach(cleanup);

describe('page patterns', () => {
  it('exposes visible status semantics without relying on color', () => {
    render(
      <>
        <StatusBadge status="success" label="Connected" />
        <Notice tone="error" title="Connection failed">Check the service settings.</Notice>
        <EmptyState title="No resources" description="Add a resource to get started." />
        <SectionHeader title="资源" description="支持较长的中英文说明。" />
      </>,
    );
    expect(screen.getByText('Connected')).toBeTruthy();
    expect(screen.getByRole('alert').textContent).toContain('Connection failed');
    expect(screen.getByText('No resources')).toBeTruthy();
    expect(screen.getByRole('heading', { name: '资源' })).toBeTruthy();
  });

  it('renders loading, confirmed empty, failure and read-only states', () => {
    setLocaleForTesting('en-US');
    const retry = vi.fn();
    const { rerender } = render(<ResourceList state={{ status: 'loading' }} loadingLabel="Loading resources" />);
    expect(screen.getByRole('status').textContent).toContain('Loading resources');

    rerender(<ResourceList state={{ status: 'idle' }} emptyContent={<EmptyState title="No resources" />} />);
    expect(screen.queryByText('No resources')).toBeNull();
    expect(screen.getByRole('status').textContent).toContain('Not checked yet');

    rerender(<ResourceList state={{ status: 'ready', empty: true, refresh: { status: 'current' } }} emptyContent={<EmptyState title="没有资源" />} />);
    expect(screen.getByText('没有资源')).toBeTruthy();

    rerender(<ResourceList state={{ status: 'error', error: 'Network unavailable' }} emptyContent={<EmptyState title="No resources" />} errorContent="Unavailable" retry={<button onClick={retry}>Retry</button>} />);
    expect(screen.queryByText('No resources')).toBeNull();
    expect(screen.getByRole('alert').textContent).toContain('Unavailable');
    fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
    expect(retry).toHaveBeenCalledOnce();

    rerender(<ResourceDetail state={{ status: 'ready', empty: false, refresh: { status: 'current' } }} readOnly><span>Read only detail</span></ResourceDetail>);
    expect(screen.getByText('Read only detail').closest('[data-read-only]')?.getAttribute('data-read-only')).toBe('true');
  });

  it('keeps detail inputs mounted through refresh, failure, retry and recovery', () => {
    setLocaleForTesting('en-US');
    const retry = vi.fn();
    const content = <input aria-label="Resource draft" defaultValue="Initial draft" />;
    const { rerender } = render(<ResourceDetail state={{ status: 'ready', empty: false, refresh: { status: 'current' } }}>{content}</ResourceDetail>);
    const input = screen.getByRole('textbox', { name: 'Resource draft' }) as HTMLInputElement;
    fireEvent.change(input, { target: { value: 'Unsaved work' } });
    input.focus();

    rerender(<ResourceDetail state={{ status: 'ready', empty: false, refresh: { status: 'refreshing' } }}>{content}</ResourceDetail>);
    expect(screen.getByRole('status').textContent).toContain('Refreshing');
    expect(screen.getByRole('textbox')).toBe(input);
    expect(document.activeElement).toBe(input);

    rerender(<ResourceDetail state={{ status: 'ready', empty: false, refresh: { status: 'error', error: 'Connection lost' } }} retry={<button onClick={retry}>Retry</button>}>{content}</ResourceDetail>);
    expect(screen.getByRole('alert').textContent).toContain('Connection lost');
    expect(screen.getByRole('alert').textContent).toContain('Showing the previous result');
    fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
    expect(retry).toHaveBeenCalledOnce();

    rerender(<ResourceDetail state={{ status: 'ready', empty: false, refresh: { status: 'refreshing', error: 'Connection lost' } }} retry={<button disabled>Retry</button>}>{content}</ResourceDetail>);
    expect(screen.getByRole('alert').textContent).toContain('Connection lost');
    expect(screen.getByRole('status').textContent).toContain('Refreshing');
    expect(screen.getByRole('button', { name: 'Retry' }).hasAttribute('disabled')).toBe(true);
    expect(screen.getByRole('textbox')).toBe(input);
    expect(input.value).toBe('Unsaved work');

    rerender(<ResourceDetail state={{ status: 'ready', empty: false, refresh: { status: 'current' } }}>{content}</ResourceDetail>);
    expect(screen.queryByRole('alert')).toBeNull();
    expect(screen.queryByText(/Refreshing/)).toBeNull();
    expect(screen.getByRole('textbox')).toBe(input);
    expect(input.value).toBe('Unsaved work');
  });

  it('retains confirmed empty content while refreshing, stale or failed', () => {
    setLocaleForTesting('en-US');
    const empty = <EmptyState title="Confirmed empty" />;
    const { rerender } = render(<ResourceList state={{ status: 'ready', empty: true, refresh: { status: 'current' } }} emptyContent={empty} />);
    const previous = screen.getByText('Confirmed empty');
    rerender(<ResourceList state={{ status: 'ready', empty: true, refresh: { status: 'refreshing' } }} emptyContent={empty} />);
    expect(screen.getByText('Confirmed empty')).toBe(previous);
    expect(screen.getByRole('status').textContent).toContain('Refreshing');
    rerender(<ResourceList state={{ status: 'ready', empty: true, refresh: { status: 'stale' } }} emptyContent={empty} />);
    expect(screen.getByText('Confirmed empty')).toBe(previous);
    expect(screen.getByRole('status').textContent).toContain('Showing the previous result');
    rerender(<ResourceList state={{ status: 'ready', empty: true, refresh: { status: 'error', error: 'Still offline' } }} emptyContent={empty} />);
    expect(screen.getByText('Confirmed empty')).toBe(previous);
    expect(screen.getByRole('alert').textContent).toContain('Still offline');
  });

  it('supports keyboard selection and disabled/read-only cards', () => {
    const onSelect = vi.fn();
    const { rerender } = render(<ResourceCard title="Keyboard resource" onSelect={onSelect} />);
    const card = screen.getByRole('button', { name: 'Keyboard resource' });
    card.focus();
    fireEvent.keyDown(card, { key: 'Enter' });
    fireEvent.keyDown(card, { key: ' ' });
    expect(onSelect).toHaveBeenCalledTimes(2);

    rerender(<ResourceCard title="Disabled resource" onSelect={onSelect} disabled />);
    expect(screen.getByRole('button', { name: 'Disabled resource' }).getAttribute('aria-disabled')).toBe('true');
    expect(screen.getByRole('button', { name: 'Disabled resource' }).getAttribute('tabindex')).toBeNull();

    rerender(<ResourceCard title="Read-only resource" onSelect={onSelect} readOnly />);
    const readOnly = screen.getByRole('button', { name: 'Read-only resource' });
    expect(readOnly.getAttribute('aria-disabled')).toBe('true');
    fireEvent.click(readOnly);
    expect(onSelect).toHaveBeenCalledTimes(2);

    const onAction = vi.fn();
    rerender(<ResourceCard title="Resource" onSelect={onSelect} actions={<button onClick={onAction}>Manage</button>} />);
    fireEvent.click(screen.getByRole('button', { name: 'Manage' }));
    expect(onAction).toHaveBeenCalledOnce();
    expect(onSelect).toHaveBeenCalledTimes(2);
  });

  it('clears a search field by keyboard-reachable button and wraps action areas', () => {
    const onChange = vi.fn();
    const onClear = vi.fn();
    render(
      <>
        <SearchField label="Search resources" value="long 中文 query" onChange={onChange} onClear={onClear} clearLabel="Clear search" />
        <FilterBar label="Resource filters" actions={<button type="submit">Apply filters</button>}>
          <span>Long filter label that can wrap</span>
        </FilterBar>
        <FixedActionBar label="Resource actions" status="Unsaved changes"><button>Save</button></FixedActionBar>
      </>,
    );
    fireEvent.click(screen.getByRole('button', { name: 'Clear search' }));
    expect(onClear).toHaveBeenCalledOnce();
    expect(screen.getByRole('group', { name: 'Resource filters' })).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Save' })).toBeTruthy();
  });
});
