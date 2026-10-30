import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { Modal, ModalHost, ToastViewport, clearToasts } from '@dgos/ui';
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
  act(() => Modal.closeAll());
  act(() => clearToasts());
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

  it('shows pending migration and retries without requesting a key from the renderer', async () => {
    const provider = {
      id: 'provider-pending', name: 'Local fixture', provider_type: 'openai_compatible',
      base_url: 'https://example.invalid/v1', enabled: true, sort_order: 0,
      created_at: '2026-10-28T00:00:00Z', updated_at: '2026-10-28T00:00:00Z',
      user_id: 'default-user', credential_status: 'migration_required',
    };
    mockedInvoke.mockImplementation(async command => {
      if (command === 'list_providers') return { success: true, data: [provider] };
      if (command === 'migrate_provider_credential') return { success: true, data: { ...provider, credential_status: 'stored', credential_ref: 'fixture-reference' } };
      return { success: true, data: [] };
    });
    render(<ProviderManagement />);
    fireEvent.click(await screen.findByRole('button', { name: 'Retry credential migration for Local fixture' }));
    await waitFor(() => expect(screen.queryByRole('button', { name: 'Retry credential migration for Local fixture' })).toBeNull());
    expect(mockedInvoke).toHaveBeenCalledWith('migrate_provider_credential', { providerId: 'provider-pending' });
    expect(useProviderStore.getState().providers[0].credential_status).toBe('stored');
  });
});

const providerFixture = {
  id: 'provider-fixture', name: 'Saved fixture', provider_type: 'openai_compatible',
  base_url: 'https://example.invalid/v1', enabled: true, sort_order: 0,
  created_at: '2026-10-28T00:00:00Z', updated_at: '2026-10-28T00:00:00Z',
  user_id: 'default-user', credential_status: 'stored', config_revision: 0,
};
const modelFixture = {
  id: 'model-fixture', provider_id: providerFixture.id, model_id: 'fixture-model',
  model_name: 'Fixture model', model_type: 'chat', supports_streaming: true,
  supports_vision: false, cached_at: '2026-10-28T00:00:00Z',
};

describe('Provider form lifecycle', () => {
  it('retains draft edits when the saved model list finishes loading', async () => {
    let resolveModels!: (value: unknown) => void;
    const modelResponse = new Promise(resolve => { resolveModels = resolve; });
    mockedInvoke.mockImplementation(async command => {
      if (command === 'list_providers') return { success: true, data: [providerFixture] };
      if (command === 'list_models') return modelResponse;
      return { success: true, data: [] };
    });
    render(<ProviderManagement />);
    fireEvent.click(await screen.findByRole('heading', { name: 'Saved fixture', level: 3 }));
    fireEvent.change(screen.getByLabelText('Name *'), { target: { value: 'Unsaved draft' } });
    fireEvent.change(screen.getByLabelText('API Key *'), { target: { value: 'synthetic-draft-key' } });
    await act(async () => { resolveModels({ success: true, data: [modelFixture] }); });
    expect(await screen.findByRole('heading', { name: 'Fixture model' })).toBeTruthy();
    expect((screen.getByLabelText('Name *') as HTMLInputElement).value).toBe('Unsaved draft');
    expect((screen.getByLabelText('API Key *') as HTMLInputElement).value).toBe('synthetic-draft-key');
  });

  it('clears the old cache after endpoint save and keeps the new saved baseline', async () => {
    let saved = providerFixture;
    mockedInvoke.mockImplementation(async (command, args) => {
      if (command === 'list_providers') return { success: true, data: [saved] };
      if (command === 'list_models') return { success: true, data: saved.config_revision === 0 ? [modelFixture] : [] };
      if (command === 'update_provider') {
        const config = (args as { config: typeof saved }).config;
        saved = { ...saved, base_url: config.base_url, config_revision: 1 };
        return { success: true, data: saved };
      }
      return { success: true, data: [] };
    });
    render(<ProviderManagement />);
    fireEvent.click(await screen.findByRole('heading', { name: 'Saved fixture', level: 3 }));
    expect(await screen.findByRole('heading', { name: 'Fixture model' })).toBeTruthy();
    fireEvent.change(screen.getByLabelText('Base URL *'), { target: { value: 'https://example.invalid/new' } });
    fireEvent.change(screen.getByLabelText('API Key *'), { target: { value: 'synthetic-new-key' } });
    fireEvent.click(screen.getByRole('button', { name: /Save Changes/ }));
    await waitFor(() => expect(useProviderStore.getState().providers[0].config_revision).toBe(1));
    await waitFor(() => expect(screen.queryByRole('heading', { name: 'Fixture model' })).toBeNull());
    expect((screen.getByLabelText('Base URL *') as HTMLInputElement).value).toBe(saved.base_url);
    expect((screen.getByLabelText('API Key *') as HTMLInputElement).value).toBe('');
    expect((screen.getByRole('button', { name: /Refresh Models/ }) as HTMLButtonElement).disabled).toBe(false);
  });
});

const HOST_MESSAGE = 'raw host detail that must stay off screen';

function renderWithToasts() {
  return render(<><ProviderManagement /><ToastViewport /><ModalHost /></>);
}

async function openSavedProvider() {
  renderWithToasts();
  fireEvent.click(await screen.findByRole('heading', { name: 'Saved fixture', level: 3 }));
  await screen.findByLabelText('Name *');
}

describe('Provider failure reporting', () => {
  it('shows registry text in a toast when the connection test fails with a known code', async () => {
    mockedInvoke.mockImplementation(async command => {
      if (command === 'list_providers') return { success: true, data: [providerFixture] };
      if (command === 'test_provider_connection') return { success: false, error: { code: 1001002, message: HOST_MESSAGE } };
      return { success: true, data: [] };
    });
    await openSavedProvider();
    fireEvent.change(screen.getByLabelText('API Key *'), { target: { value: 'synthetic-key' } });
    fireEvent.click(screen.getByRole('button', { name: /Test Connection/ }));

    expect(await screen.findByText('API Key 无效')).toBeTruthy();
    expect(document.body.textContent).toContain(HOST_MESSAGE); // inline result is kept
    expect(document.querySelector('[data-toast-type]')?.textContent).not.toContain(HOST_MESSAGE);
    expect(document.querySelectorAll('[data-toast-type]')).toHaveLength(1);
  });

  it('reports a rejected model refresh once with the generic text', async () => {
    mockedInvoke.mockImplementation(async command => {
      if (command === 'list_providers') return { success: true, data: [providerFixture] };
      if (command === 'refresh_models') throw new Error(HOST_MESSAGE);
      return { success: true, data: [] };
    });
    const logged = vi.spyOn(console, 'error').mockImplementation(() => {});
    await openSavedProvider();
    fireEvent.click(screen.getByRole('button', { name: /Refresh Models/ }));

    await waitFor(() => expect(document.querySelectorAll('[data-toast-type="error"]')).toHaveLength(1));
    expect(document.querySelector('[data-toast-type="error"]')?.textContent).toContain('操作失败');
    expect(document.querySelector('[data-toast-type="error"]')?.textContent).not.toContain(HOST_MESSAGE);
    expect(logged).toHaveBeenCalledWith('[dgos] error', { code: null, where: 'ProviderManagement' });
  });

  it('blocks with a dialog for a severe code and never echoes the host message', async () => {
    mockedInvoke.mockImplementation(async command => {
      if (command === 'list_providers') return { success: true, data: [providerFixture] };
      if (command === 'refresh_models') return { success: false, error: { code: 1001015, message: HOST_MESSAGE } };
      return { success: true, data: [] };
    });
    await openSavedProvider();
    fireEvent.click(screen.getByRole('button', { name: /Refresh Models/ }));

    const dialog = await screen.findByRole('dialog', { name: '系统凭据库不可用' });
    expect(within(dialog).queryByText(HOST_MESSAGE)).toBeNull();
  });
});

describe('Provider store failures reach the unified error handling', () => {
  const failWith = (command: string, error: unknown) => {
    mockedInvoke.mockImplementation(async name => {
      if (name === 'list_providers') return { success: true, data: [providerFixture] };
      if (name === command) return typeof error === 'object' && error !== null && 'code' in error ? { success: false, error } : (() => { throw error; })();
      return { success: true, data: [] };
    });
  };
  const toasts = () => Array.from(document.querySelectorAll('[data-toast-type="error"]'));

  it('toggle: a known code shows its registry text once and keeps the inline alert', async () => {
    failWith('toggle_provider', { code: 1001003, message: HOST_MESSAGE });
    renderWithToasts();
    fireEvent.click(await screen.findByRole('switch', { name: 'Disable Saved fixture' }));

    expect(await screen.findByText('Provider 不可用')).toBeTruthy();
    expect(toasts()).toHaveLength(1);
    expect(toasts()[0].textContent).not.toContain(HOST_MESSAGE);
    const inline = screen.getAllByRole('alert').filter(node => !node.closest('[data-toast-type]'));
    expect(inline).toHaveLength(1);
    expect(inline[0].textContent).toContain(HOST_MESSAGE);
  });

  it('toggle: the error action opens the Provider list only when a router and the route exist', async () => {
    failWith('toggle_provider', { code: 1001003, message: HOST_MESSAGE });
    renderWithToasts();
    fireEvent.click(await screen.findByRole('switch', { name: 'Disable Saved fixture' }));
    await screen.findByText('Provider 不可用');
    // Rendered without a Router: no navigation button, even though 1001003 has an action in the registry.
    expect(screen.queryByRole('button', { name: '查看 Provider 列表' })).toBeNull();
  });

  it('delete: a rejected command is reported once with the generic text and no code', async () => {
    const logged = vi.spyOn(console, 'error').mockImplementation(() => {});
    failWith('delete_provider', new Error(HOST_MESSAGE));
    renderWithToasts();
    fireEvent.click(await screen.findByRole('button', { name: 'Delete Saved fixture' }));
    fireEvent.click(within(await screen.findByRole('dialog', { name: 'Delete Provider?' })).getByRole('button', { name: 'Delete' }));

    await waitFor(() => expect(toasts()).toHaveLength(1));
    expect(toasts()[0].textContent).toContain('操作失败');
    expect(toasts()[0].textContent).not.toContain(HOST_MESSAGE);
    expect(logged).toHaveBeenCalledWith('[dgos] error', { code: null, where: 'ProviderManagement' });
  });

  it('update: a coded failure shows the registry text and the form keeps its draft', async () => {
    failWith('update_provider', { code: 1001002, message: HOST_MESSAGE });
    await openSavedProvider();
    fireEvent.change(screen.getByLabelText('Name *'), { target: { value: 'Renamed' } });
    fireEvent.click(screen.getByRole('button', { name: /Save Changes/ }));

    expect(await screen.findByText('API Key 无效')).toBeTruthy();
    expect((screen.getByLabelText('Name *') as HTMLInputElement).value).toBe('Renamed');
  });

  it('credential migration: the blocking dialog appears for the severe code', async () => {
    const pending = { ...providerFixture, credential_status: 'migration_required' };
    mockedInvoke.mockImplementation(async name => {
      if (name === 'list_providers') return { success: true, data: [pending] };
      if (name === 'migrate_provider_credential') return { success: false, error: { code: 1001015, message: HOST_MESSAGE } };
      return { success: true, data: [] };
    });
    renderWithToasts();
    fireEvent.click(await screen.findByRole('button', { name: 'Retry credential migration for Saved fixture' }));

    const dialog = await screen.findByRole('dialog', { name: '系统凭据库不可用' });
    expect(within(dialog).queryByText(HOST_MESSAGE)).toBeNull();
  });

  it('a later success does not report a stale failure', async () => {
    let fail = true;
    mockedInvoke.mockImplementation(async (name, args) => {
      if (name === 'list_providers') return { success: true, data: [providerFixture] };
      if (name === 'toggle_provider') {
        return fail
          ? { success: false, error: { code: 1001003, message: HOST_MESSAGE } }
          : { success: true, data: { ...providerFixture, enabled: (args as { enabled: boolean }).enabled } };
      }
      return { success: true, data: [] };
    });
    renderWithToasts();
    fireEvent.click(await screen.findByRole('switch', { name: 'Disable Saved fixture' }));
    await screen.findByText('Provider 不可用');
    act(() => clearToasts());

    fail = false;
    fireEvent.click(screen.getByRole('switch', { name: 'Disable Saved fixture' }));
    await screen.findByRole('switch', { name: 'Enable Saved fixture' });
    expect(toasts()).toHaveLength(0);
  });

  it('loading the list: a coded failure shows its registry text once', async () => {
    mockedInvoke.mockImplementation(async name => {
      if (name === 'list_providers') return { success: false, error: { code: 1001015, message: HOST_MESSAGE } };
      return { success: true, data: [] };
    });
    renderWithToasts();

    const dialog = await screen.findByRole('dialog', { name: '系统凭据库不可用' });
    expect(within(dialog).queryByText(HOST_MESSAGE)).toBeNull();
  });

  it('loading models for a selected provider: a rejected call is reported once with the generic text', async () => {
    const logged = vi.spyOn(console, 'error').mockImplementation(() => {});
    mockedInvoke.mockImplementation(async name => {
      if (name === 'list_providers') return { success: true, data: [providerFixture] };
      if (name === 'list_models') throw new Error(HOST_MESSAGE);
      return { success: true, data: [] };
    });
    renderWithToasts();
    fireEvent.click(await screen.findByRole('heading', { name: 'Saved fixture', level: 3 }));

    await waitFor(() => expect(toasts()).toHaveLength(1));
    expect(toasts()[0].textContent).toContain('操作失败');
    expect(toasts()[0].textContent).not.toContain(HOST_MESSAGE);
    expect(logged).toHaveBeenCalledWith('[dgos] error', { code: null, where: 'ProviderManagement' });
  });

  it('a model load that was superseded by another selection does not report', async () => {
    const other = { ...providerFixture, id: 'provider-other', name: 'Other fixture' };
    let rejectFirst!: (error: Error) => void;
    mockedInvoke.mockImplementation(async (name, args) => {
      if (name === 'list_providers') return { success: true, data: [providerFixture, other] };
      if (name === 'list_models') {
        if ((args as { providerId: string }).providerId === providerFixture.id) {
          return new Promise((_resolve, reject) => { rejectFirst = reject; });
        }
        return { success: true, data: [] };
      }
      return { success: true, data: [] };
    });
    renderWithToasts();
    fireEvent.click(await screen.findByRole('heading', { name: 'Saved fixture', level: 3 }));
    await waitFor(() => expect(rejectFirst).toBeTypeOf('function'));
    fireEvent.click(screen.getByRole('heading', { name: 'Other fixture', level: 3 }));
    await waitFor(() => expect(mockedInvoke).toHaveBeenCalledWith('list_models', { providerId: 'provider-other' }));

    await act(async () => { rejectFirst(new Error(HOST_MESSAGE)); });
    expect(toasts()).toHaveLength(0);
  });
});

describe('Provider list controls', () => {
  it('toggles enablement through the switch without selecting the provider', async () => {
    mockedInvoke.mockImplementation(async (command, args) => {
      if (command === 'list_providers') return { success: true, data: [providerFixture] };
      if (command === 'toggle_provider') return { success: true, data: { ...providerFixture, enabled: (args as { enabled: boolean }).enabled } };
      return { success: true, data: [] };
    });
    renderWithToasts();
    fireEvent.click(await screen.findByRole('switch', { name: 'Disable Saved fixture' }));

    await waitFor(() => expect(mockedInvoke).toHaveBeenCalledWith('toggle_provider', { id: providerFixture.id, enabled: false }));
    expect(await screen.findByRole('switch', { name: 'Enable Saved fixture' })).toBeTruthy();
    expect(screen.queryByLabelText('Name *')).toBeNull();
  });

  it('deletes only after the dialog is confirmed', async () => {
    mockedInvoke.mockImplementation(async command => {
      if (command === 'list_providers') return { success: true, data: [providerFixture] };
      if (command === 'delete_provider') return { success: true };
      return { success: true, data: [] };
    });
    renderWithToasts();
    fireEvent.click(await screen.findByRole('button', { name: 'Delete Saved fixture' }));
    const dialog = await screen.findByRole('dialog', { name: 'Delete Provider?' });
    fireEvent.click(within(dialog).getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(mockedInvoke).not.toHaveBeenCalledWith('delete_provider', expect.anything());

    fireEvent.click(screen.getByRole('button', { name: 'Delete Saved fixture' }));
    fireEvent.click(within(await screen.findByRole('dialog', { name: 'Delete Provider?' })).getByRole('button', { name: 'Delete' }));
    await waitFor(() => expect(mockedInvoke).toHaveBeenCalledWith('delete_provider', { id: providerFixture.id }));
    await waitFor(() => expect(screen.queryByRole('button', { name: 'Delete Saved fixture' })).toBeNull());
    expect(screen.queryByRole('dialog')).toBeNull();
  });
});

async function chooseOption(selectName: string, optionName: string) {
  const trigger = screen.getByRole('combobox', { name: selectName });
  trigger.focus();
  fireEvent.keyDown(trigger, { key: 'Enter' });
  const listbox = await screen.findByRole('listbox');
  fireEvent.click(within(listbox).getByRole('option', { name: optionName }));
  await waitFor(() => expect(screen.queryByRole('listbox')).toBeNull());
}

describe('Connection test diagnostics (AC37)', () => {
  it.each([
    [1001002, 'Invalid API key'],
    [1001019, 'The key lacks permission, or the region is restricted'],
    [1001025, 'The response was not the expected JSON. Check that the base URL is not missing /v1 and is not a web page address, and note redirects are not followed - enter the final address'],
    [1001023, 'Plain HTTP is not allowed for this address. Use https://, or enable Allow plain HTTP under Advanced options'],
  ])('shows an actionable hint for %i next to the host message', async (code, hint) => {
    mockedInvoke.mockImplementation(async command => {
      if (command === 'list_providers') return { success: true, data: [providerFixture] };
      if (command === 'test_provider_connection') return { success: false, error: { code, message: HOST_MESSAGE } };
      return { success: true, data: [] };
    });
    await openSavedProvider();
    fireEvent.change(screen.getByLabelText('API Key *'), { target: { value: 'synthetic-key' } });
    fireEvent.click(screen.getByRole('button', { name: /Test Connection/ }));

    expect(await screen.findByText(hint)).toBeTruthy();
    expect(screen.getByText(HOST_MESSAGE)).toBeTruthy();
  });

  it('shows only the host message for a code without a hint', async () => {
    mockedInvoke.mockImplementation(async command => {
      if (command === 'list_providers') return { success: true, data: [providerFixture] };
      if (command === 'test_provider_connection') return { success: false, error: { code: 1001014, message: HOST_MESSAGE } };
      return { success: true, data: [] };
    });
    await openSavedProvider();
    fireEvent.change(screen.getByLabelText('API Key *'), { target: { value: 'synthetic-key' } });
    fireEvent.click(screen.getByRole('button', { name: /Test Connection/ }));

    expect(await screen.findByText(HOST_MESSAGE)).toBeTruthy();
    expect(screen.queryByText('Invalid API key')).toBeNull();
  });
});

describe('Services without a model list endpoint (AC17)', () => {
  const NO_LIST = /This service has no model list endpoint/;

  async function openNewFilledForm() {
    renderWithToasts();
    fireEvent.click(await screen.findByRole('button', { name: '+ Create Provider' }));
    fireEvent.change(await screen.findByLabelText('Name *'), { target: { value: 'No list' } });
    fireEvent.change(screen.getByLabelText('Base URL *'), { target: { value: 'https://nolist.example/v1' } });
    fireEvent.change(screen.getByLabelText('API Key *'), { target: { value: 'synthetic-key' } });
  }

  const callsOf = (name: string) => mockedInvoke.mock.calls.filter(([command]) => command === name);

  it('asks for a model name on 1001020 and then tests with probeModel', async () => {
    mockedInvoke.mockImplementation(async (command, args) => {
      if (command === 'test_provider_connection') {
        return args && 'probeModel' in args
          ? { success: true, data: true }
          : { success: false, error: { code: 1001020, message: HOST_MESSAGE } };
      }
      return { success: true, data: [] };
    });
    await openNewFilledForm();
    expect(screen.queryByLabelText('Model name')).toBeNull();

    fireEvent.click(screen.getByRole('button', { name: /Test Connection/ }));
    expect(await screen.findByText(NO_LIST)).toBeTruthy();
    expect(screen.getByText(/consumes a few tokens/)).toBeTruthy();
    expect(callsOf('test_provider_connection')[0][1]).not.toHaveProperty('probeModel');

    fireEvent.change(screen.getByLabelText('Model name'), { target: { value: '  probe-model  ' } });
    fireEvent.click(screen.getByRole('button', { name: /Test Connection/ }));
    expect(await screen.findByText('Connection successful')).toBeTruthy();
    expect(mockedInvoke).toHaveBeenLastCalledWith('test_provider_connection', {
      config: expect.objectContaining({ base_url: 'https://nolist.example/v1' }),
      probeModel: 'probe-model',
    });
  });

  it('shows the host message instead of looping when 1001020 returns although a model name was entered', async () => {
    mockedInvoke.mockImplementation(async command => {
      if (command === 'test_provider_connection') return { success: false, error: { code: 1001020, message: HOST_MESSAGE } };
      return { success: true, data: [] };
    });
    await openNewFilledForm();
    fireEvent.click(screen.getByRole('button', { name: /Test Connection/ }));
    await screen.findByText(NO_LIST);

    fireEvent.change(screen.getByLabelText('Model name'), { target: { value: 'probe-model' } });
    fireEvent.click(screen.getByRole('button', { name: /Test Connection/ }));
    expect(await screen.findByText(HOST_MESSAGE)).toBeTruthy();
    expect(screen.queryByText(NO_LIST)).toBeNull();
  });

  it('clears the model name when the base URL changes', async () => {
    mockedInvoke.mockImplementation(async command => {
      if (command === 'test_provider_connection') return { success: false, error: { code: 1001020, message: HOST_MESSAGE } };
      return { success: true, data: [] };
    });
    await openNewFilledForm();
    fireEvent.click(screen.getByRole('button', { name: /Test Connection/ }));
    fireEvent.change(await screen.findByLabelText('Model name'), { target: { value: 'probe-model' } });

    fireEvent.change(screen.getByLabelText('Base URL *'), { target: { value: 'https://other.example/v1' } });
    expect(screen.queryByLabelText('Model name')).toBeNull();
    expect(screen.queryByText(NO_LIST)).toBeNull();
  });

  it('passes probeModel on create after a create attempt reports 1001020', async () => {
    mockedInvoke.mockImplementation(async (command, args) => {
      if (command === 'create_provider') {
        return args && 'probeModel' in args
          ? { success: true, data: { ...providerFixture, id: 'created', name: 'No list' } }
          : { success: false, error: { code: 1001020, message: HOST_MESSAGE } };
      }
      return { success: true, data: [] };
    });
    await openNewFilledForm();
    fireEvent.click(screen.getByRole('button', { name: /Create/ }));
    expect(await screen.findByText(NO_LIST)).toBeTruthy();
    expect(callsOf('create_provider')[0][1]).not.toHaveProperty('probeModel');

    fireEvent.change(screen.getByLabelText('Model name'), { target: { value: 'probe-model' } });
    fireEvent.click(screen.getByRole('button', { name: /Create/ }));
    await waitFor(() => expect(mockedInvoke).toHaveBeenLastCalledWith('create_provider', {
      config: expect.objectContaining({ name: 'No list', base_url: 'https://nolist.example/v1' }),
      probeModel: 'probe-model',
    }));
  });

  it('tests a saved provider by id with probeModel once one is entered', async () => {
    mockedInvoke.mockImplementation(async (command, args) => {
      if (command === 'list_providers') return { success: true, data: [providerFixture] };
      if (command === 'test_provider_by_id') {
        return args && 'probeModel' in args
          ? { success: true, data: true }
          : { success: false, error: { code: 1001020, message: HOST_MESSAGE } };
      }
      return { success: true, data: [] };
    });
    await openSavedProvider();
    fireEvent.click(screen.getByRole('button', { name: /Test Connection/ }));
    fireEvent.change(await screen.findByLabelText('Model name'), { target: { value: 'probe-model' } });
    fireEvent.click(screen.getByRole('button', { name: /Test Connection/ }));

    expect(await screen.findByText('Connection successful')).toBeTruthy();
    expect(mockedInvoke).toHaveBeenLastCalledWith('test_provider_by_id', { providerId: providerFixture.id, probeModel: 'probe-model' });
  });
});

describe('Manual models (AC17)', () => {
  const remoteModel = { ...modelFixture, model_id: 'legacy-chat', model_name: 'legacy-chat', model_type: 'chat', source: 'remote' };
  const manualRow = (modelId: string, kind: string) => ({
    ...modelFixture, id: `row-${modelId}`, model_id: modelId, model_name: modelId, model_type: kind,
    source: 'manual', kind_source: 'manual',
  });

  function mockManualHost(initial: unknown[]) {
    mockedInvoke.mockImplementation(async (command, args) => {
      if (command === 'list_providers') return { success: true, data: [providerFixture] };
      if (command === 'list_models') return { success: true, data: initial };
      if (command === 'add_manual_model') {
        const { modelId, kind } = args as { modelId: string; kind?: string };
        return { success: true, data: manualRow(modelId, kind ?? 'text-generation') };
      }
      if (command === 'remove_manual_model') return { success: true, data: null };
      return { success: true, data: [] };
    });
  }

  it('shows each model kind (legacy chat as text-generation) and a manual badge only on manual rows', async () => {
    mockManualHost([remoteModel, manualRow('my-embed', 'embedding')]);
    await openSavedProvider();

    const remoteCard = (await screen.findByRole('heading', { name: 'legacy-chat' })).parentElement!;
    expect(remoteCard.textContent).toContain('text-generation');
    expect(remoteCard.textContent).not.toContain('manual');
    expect(within(remoteCard).queryByRole('button', { name: /Remove/ })).toBeNull();

    const manualCard = screen.getByRole('heading', { name: 'my-embed' }).parentElement!;
    expect(manualCard.textContent).toContain('embedding');
    expect(within(manualCard).getByText('manual')).toBeTruthy();
  });

  it('adds a manual model with and without a kind and shows both rows', async () => {
    mockManualHost([]);
    await openSavedProvider();

    fireEvent.change(await screen.findByLabelText('Model ID'), { target: { value: ' model-a ' } });
    fireEvent.click(screen.getByRole('button', { name: 'Add model' }));
    expect(await screen.findByRole('heading', { name: 'model-a' })).toBeTruthy();
    expect(mockedInvoke).toHaveBeenCalledWith('add_manual_model', { providerId: providerFixture.id, modelId: 'model-a' });
    expect((screen.getByLabelText('Model ID') as HTMLInputElement).value).toBe('');

    fireEvent.change(screen.getByLabelText('Model ID'), { target: { value: 'model-b' } });
    await chooseOption('Kind', 'embedding');
    fireEvent.click(screen.getByRole('button', { name: 'Add model' }));
    expect(await screen.findByRole('heading', { name: 'model-b' })).toBeTruthy();
    expect(mockedInvoke).toHaveBeenCalledWith('add_manual_model', { providerId: providerFixture.id, modelId: 'model-b', kind: 'embedding' });
    expect(screen.getByRole('heading', { name: 'model-a' })).toBeTruthy();
    expect(screen.getByRole('heading', { name: 'model-b' }).parentElement!.textContent).toContain('embedding');
  });

  it('offers every fixed model kind plus Auto', async () => {
    mockManualHost([]);
    await openSavedProvider();
    const trigger = await screen.findByRole('combobox', { name: 'Kind' });
    trigger.focus();
    fireEvent.keyDown(trigger, { key: 'Enter' });
    const listbox = await screen.findByRole('listbox');
    expect(within(listbox).getAllByRole('option').map(option => option.textContent)).toEqual([
      'Auto', 'text-generation', 'embedding', 'rerank', 'image-generation', 'video-generation',
      'speech-synthesis', 'speech-transcription', 'moderation',
    ]);
  });

  it('removes a manual model by provider and model id', async () => {
    mockManualHost([manualRow('my-embed', 'embedding')]);
    await openSavedProvider();
    fireEvent.click(await screen.findByRole('button', { name: 'Remove my-embed' }));

    await waitFor(() => expect(screen.queryByRole('heading', { name: 'my-embed' })).toBeNull());
    expect(mockedInvoke).toHaveBeenCalledWith('remove_manual_model', { providerId: providerFixture.id, modelId: 'my-embed' });
  });

  it('reports a rejected add through the unified error path and keeps the typed id', async () => {
    mockedInvoke.mockImplementation(async command => {
      if (command === 'list_providers') return { success: true, data: [providerFixture] };
      if (command === 'add_manual_model') return { success: false, error: { code: 1001020, message: HOST_MESSAGE } };
      return { success: true, data: [] };
    });
    await openSavedProvider();
    fireEvent.change(await screen.findByLabelText('Model ID'), { target: { value: 'bad id' } });
    fireEvent.click(screen.getByRole('button', { name: 'Add model' }));

    expect(await screen.findByText('请求不受支持')).toBeTruthy();
    expect(document.body.textContent).not.toContain(HOST_MESSAGE);
    expect((screen.getByLabelText('Model ID') as HTMLInputElement).value).toBe('bad id');
  });
});

describe('Provider confirm dialogs (WKWebView has no window.confirm)', () => {
  const otherFixture = { ...providerFixture, id: 'provider-other', name: 'Other fixture' };
  const dirtyName = () => fireEvent.change(screen.getByLabelText('Name *'), { target: { value: 'Unsaved draft' } });
  const modelCalls = (id: string) => mockedInvoke.mock.calls.filter(([name, args]) => name === 'list_models' && (args as { providerId?: string }).providerId === id);
  const deleteCalls = () => mockedInvoke.mock.calls.filter(([name]) => name === 'delete_provider');

  function mockTwoProviders(extra: (command: string) => unknown = () => undefined) {
    mockedInvoke.mockImplementation(async command => {
      const handled = extra(command);
      if (handled !== undefined) return handled;
      if (command === 'list_providers') return { success: true, data: [providerFixture, otherFixture] };
      return { success: true, data: [] };
    });
  }

  it('stays on the draft when switching providers is cancelled, and switches once when confirmed', async () => {
    const confirm = vi.spyOn(window, 'confirm');
    mockTwoProviders();
    await openSavedProvider();
    dirtyName();

    const target = screen.getByRole('heading', { name: 'Other fixture', level: 3 });
    fireEvent.click(target); fireEvent.click(target);
    let dialog = await screen.findByRole('dialog', { name: 'Discard unsaved changes?' });
    expect(screen.getAllByRole('dialog')).toHaveLength(1);
    fireEvent.click(within(dialog).getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect((screen.getByLabelText('Name *') as HTMLInputElement).value).toBe('Unsaved draft');
    expect(modelCalls(otherFixture.id)).toHaveLength(0);

    fireEvent.click(screen.getByRole('heading', { name: 'Other fixture', level: 3 }));
    dialog = await screen.findByRole('dialog', { name: 'Discard unsaved changes?' });
    const discard = within(dialog).getByRole('button', { name: 'Discard' });
    fireEvent.click(discard); fireEvent.click(discard);
    await waitFor(() => expect((screen.getByLabelText('Name *') as HTMLInputElement).value).toBe('Other fixture'));
    expect(screen.queryByRole('dialog')).toBeNull();
    await waitFor(() => expect(modelCalls(otherFixture.id)).toHaveLength(1));
    expect(screen.queryByRole('img', { name: 'Unsaved changes' })).toBeNull();
    expect(confirm).not.toHaveBeenCalled();
  });

  it('Esc leaves the draft in place, and a selection without unsaved changes needs no dialog', async () => {
    mockTwoProviders();
    await openSavedProvider();
    fireEvent.click(screen.getByRole('heading', { name: 'Other fixture', level: 3 }));
    await waitFor(() => expect((screen.getByLabelText('Name *') as HTMLInputElement).value).toBe('Other fixture'));
    expect(screen.queryByRole('dialog')).toBeNull();

    dirtyName();
    fireEvent.click(screen.getByRole('heading', { name: 'Saved fixture', level: 3 }));
    fireEvent.keyDown(await screen.findByRole('dialog', { name: 'Discard unsaved changes?' }), { key: 'Escape' });
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect((screen.getByLabelText('Name *') as HTMLInputElement).value).toBe('Unsaved draft');
  });

  it('guards the new-provider entry point the same way', async () => {
    mockTwoProviders();
    await openSavedProvider();
    dirtyName();

    fireEvent.click(screen.getByRole('button', { name: '+ New Provider' }));
    fireEvent.click(within(await screen.findByRole('dialog', { name: 'Discard unsaved changes?' })).getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(screen.queryByRole('heading', { name: 'New Provider' })).toBeNull();
    expect((screen.getByLabelText('Name *') as HTMLInputElement).value).toBe('Unsaved draft');

    fireEvent.click(screen.getByRole('button', { name: '+ New Provider' }));
    fireEvent.click(within(await screen.findByRole('dialog', { name: 'Discard unsaved changes?' })).getByRole('button', { name: 'Discard' }));
    expect(await screen.findByRole('heading', { name: 'New Provider' })).toBeTruthy();
    expect((screen.getByLabelText('Name *') as HTMLInputElement).value).toBe('');
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('deletes from the form only after the dialog is confirmed, once, while the dialog stays busy', async () => {
    let finish!: (value: unknown) => void;
    mockTwoProviders(command => command === 'delete_provider' ? new Promise(resolve => { finish = resolve; }) : undefined);
    await openSavedProvider();

    fireEvent.click(screen.getByRole('button', { name: /🗑️ Delete/ }));
    let dialog = await screen.findByRole('dialog', { name: 'Delete Provider?' });
    expect(within(dialog).getByText(/"Saved fixture"/)).toBeTruthy();
    fireEvent.click(within(dialog).getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(deleteCalls()).toHaveLength(0);

    fireEvent.click(screen.getByRole('button', { name: /🗑️ Delete/ }));
    dialog = await screen.findByRole('dialog', { name: 'Delete Provider?' });
    const confirmButton = within(dialog).getByRole('button', { name: 'Delete' });
    fireEvent.click(confirmButton); fireEvent.click(confirmButton);
    await waitFor(() => expect(deleteCalls()).toHaveLength(1));
    expect(deleteCalls()[0][1]).toEqual({ id: providerFixture.id });
    expect(screen.getByRole('dialog', { name: 'Delete Provider?' })).toBeTruthy();
    expect(within(dialog).getByRole('button', { name: 'Cancel' }).hasAttribute('disabled')).toBe(true);
    await act(async () => { finish({ success: true }); });
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(screen.queryByLabelText('Name *')).toBeNull();
    expect(screen.queryByRole('heading', { name: 'Saved fixture', level: 3 })).toBeNull();
    expect(screen.getByRole('heading', { name: 'Other fixture', level: 3 })).toBeTruthy();
    expect(deleteCalls()).toHaveLength(1);
  });

  it('reports a failed form delete once, closes the dialog and keeps the provider selected', async () => {
    vi.spyOn(console, 'error').mockImplementation(() => {});
    mockTwoProviders(command => command === 'delete_provider' ? { success: false, error: { code: 1000000, message: HOST_MESSAGE } } : undefined);
    await openSavedProvider();
    fireEvent.click(screen.getByRole('button', { name: /🗑️ Delete/ }));
    fireEvent.click(within(await screen.findByRole('dialog', { name: 'Delete Provider?' })).getByRole('button', { name: 'Delete' }));

    await waitFor(() => expect(document.querySelectorAll('[data-toast-type="error"]')).toHaveLength(1));
    expect(screen.queryByRole('dialog', { name: 'Delete Provider?' })).toBeNull();
    expect((screen.getByLabelText('Name *') as HTMLInputElement).value).toBe('Saved fixture');
    expect(deleteCalls()).toHaveLength(1);
  });

  it('closes an open dialog when the page unmounts and never acts afterwards', async () => {
    mockTwoProviders();
    const view = render(<><ProviderManagement /><ModalHost /></>);
    fireEvent.click(await screen.findByRole('heading', { name: 'Saved fixture', level: 3 }));
    await screen.findByLabelText('Name *');
    fireEvent.click(screen.getByRole('button', { name: /🗑️ Delete/ }));
    await screen.findByRole('dialog', { name: 'Delete Provider?' });
    view.unmount();
    expect(document.querySelector('[role="dialog"]')).toBeNull();
    expect(deleteCalls()).toHaveLength(0);
  });
});
