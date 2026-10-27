import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import ProviderManagement from '../src/pages/ProviderManagement';
import { useProviderStore } from '../src/stores/providerStore';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));

const mockedInvoke = vi.mocked(invoke);
const initialState = useProviderStore.getState();

beforeEach(() => {
  useProviderStore.setState(initialState, true);
  mockedInvoke.mockImplementation(async (command) => {
    if (command === 'list_providers') {
      return { success: true, data: [] };
    }
    if (command === 'list_models_for_config') {
      return {
        success: true,
        data: [{
          id: 'model-1',
          provider_id: '',
          model_id: 'gpt-test',
          model_name: 'gpt-test',
          model_type: 'chat',
          supports_streaming: true,
          supports_vision: false,
          cached_at: '2026-10-27T00:00:00Z',
        }],
      };
    }
    return { success: true, data: [] };
  });
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe('Provider creation entry points', () => {
  it('opens the new provider form from the empty state', async () => {
    render(<ProviderManagement />);

    expect(await screen.findByText('No Providers Yet')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: '+ Create Provider' }));

    expect(await screen.findByRole('heading', { name: 'New Provider' })).toBeTruthy();
    expect(screen.getByLabelText('Name *')).toBeTruthy();
  });

  it('loads models before the new provider is saved', async () => {
    render(<ProviderManagement />);

    fireEvent.click(await screen.findByRole('button', { name: '+ Create Provider' }));
    fireEvent.change(screen.getByLabelText('API Key *'), { target: { value: 'test-key' } });
    fireEvent.click(screen.getByRole('button', { name: /Refresh Models/ }));

    expect(await screen.findByRole('heading', { name: 'gpt-test' })).toBeTruthy();
    expect(mockedInvoke).toHaveBeenCalledWith('list_models_for_config', {
      config: expect.objectContaining({ api_key: 'test-key' }),
    });
  });
});
