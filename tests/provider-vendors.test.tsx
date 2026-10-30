// V1-FR-001 AC16 (frontend part): the vendor table comes from the host, never from the renderer. The mock holds a
// made-up vendor ("Acme Cloud") that no source file knows, so a hard-coded list cannot satisfy these tests.
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { Modal, clearToasts } from '@dgos/ui';
import ProviderManagement from '../src/pages/ProviderManagement';
import { useProviderStore } from '../src/stores/providerStore';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));

const mockedInvoke = vi.mocked(invoke);
const initialState = useProviderStore.getState();

const VENDORS = [
  {
    vendor_id: 'openai', display_name: 'OpenAI',
    endpoints: [{ provider_type: 'openai_compatible', display_name: 'OpenAI compatible', default_base_url: 'https://api.openai.com/v1' }],
  },
  {
    vendor_id: 'anthropic', display_name: 'Anthropic',
    endpoints: [{ provider_type: 'anthropic', display_name: 'Anthropic native', default_base_url: 'https://api.anthropic.com' }],
  },
  {
    vendor_id: 'acme', display_name: 'Acme Cloud',
    endpoints: [{ provider_type: 'openai_compatible', display_name: 'OpenAI compatible', default_base_url: 'https://llm.acme.example/v1' }],
  },
  {
    vendor_id: 'custom', display_name: 'Custom',
    endpoints: [
      { provider_type: 'openai_compatible', display_name: 'OpenAI compatible', default_base_url: null },
      { provider_type: 'anthropic', display_name: 'Anthropic compatible', default_base_url: null },
    ],
  },
];

const savedBase = {
  provider_type: 'openai_compatible', enabled: true, sort_order: 0,
  created_at: '2026-10-30T00:00:00Z', updated_at: '2026-10-30T00:00:00Z',
  user_id: 'default-user', credential_status: 'stored', config_revision: 0,
};

function mockHost(providers: unknown[] = [], extra: (command: string, args: unknown) => unknown = () => undefined) {
  mockedInvoke.mockImplementation(async (command, args) => {
    const handled = extra(command, args);
    if (handled !== undefined) return handled;
    if (command === 'list_provider_vendors') return { success: true, data: VENDORS };
    if (command === 'list_providers') return { success: true, data: providers };
    if (command === 'create_provider') {
      return { success: true, data: { ...savedBase, id: 'created', name: 'x', base_url: 'https://x' } };
    }
    return { success: true, data: [] };
  });
}

beforeEach(() => {
  useProviderStore.setState(initialState, true);
});

afterEach(() => {
  act(() => Modal.closeAll());
  act(() => clearToasts());
  cleanup();
  vi.restoreAllMocks();
});

const input = (label: string) => screen.getByLabelText(label) as HTMLInputElement;

async function openNewForm() {
  render(<ProviderManagement />);
  fireEvent.click(await screen.findByRole('button', { name: '+ Create Provider' }));
  await screen.findByRole('heading', { name: 'New Provider' });
  await waitFor(() => expect(input('Base URL *').value).toBe('https://api.openai.com/v1'));
}

async function choose(selectName: string, optionName: string) {
  const trigger = screen.getByRole('combobox', { name: selectName });
  trigger.focus();
  fireEvent.keyDown(trigger, { key: 'Enter' });
  const listbox = await screen.findByRole('listbox');
  fireEvent.click(within(listbox).getByRole('option', { name: optionName }));
  await waitFor(() => expect(screen.queryByRole('listbox')).toBeNull());
}

const createButton = () => screen.getByRole('button', { name: /💾 Create/ }) as HTMLButtonElement;

function fillRequired() {
  fireEvent.change(input('Name *'), { target: { value: 'My service' } });
  fireEvent.change(input('API Key *'), { target: { value: 'synthetic-key' } });
}

describe('Vendor selection (AC16)', () => {
  it('lists exactly the vendors the host returned, including one the renderer has never heard of', async () => {
    mockHost();
    await openNewForm();

    const trigger = screen.getByRole('combobox', { name: 'Vendor' });
    expect(mockedInvoke).toHaveBeenCalledWith('list_provider_vendors', {});
    expect(trigger.textContent).toContain('OpenAI');
    trigger.focus();
    fireEvent.keyDown(trigger, { key: 'Enter' });
    const listbox = await screen.findByRole('listbox');
    expect(within(listbox).getAllByRole('option').map((option) => option.textContent)).toEqual([
      'OpenAI', 'Anthropic', 'Acme Cloud', 'Custom',
    ]);
  });

  it('implies the endpoint of a single-endpoint vendor and shows no adapter select', async () => {
    mockHost();
    await openNewForm();
    expect(screen.queryByRole('combobox', { name: 'Provider Type' })).toBeNull();

    await choose('Vendor', 'Acme Cloud');
    expect(screen.queryByRole('combobox', { name: 'Provider Type' })).toBeNull();
    expect(input('Base URL *').value).toBe('https://llm.acme.example/v1');
  });

  it('pre-fills the base URL per vendor and keeps it editable', async () => {
    mockHost();
    await openNewForm();
    await choose('Vendor', 'Anthropic');
    expect(input('Base URL *').value).toBe('https://api.anthropic.com');

    fireEvent.change(input('Base URL *'), { target: { value: 'https://gateway.example/anthropic' } });
    expect(input('Base URL *').value).toBe('https://gateway.example/anthropic');
  });

  it('asks for the adapter of a multi-endpoint vendor and requires a base URL before Create', async () => {
    mockHost();
    await openNewForm();
    await choose('Vendor', 'Custom');

    expect(screen.getByRole('combobox', { name: 'Provider Type' }).textContent).toContain('OpenAI compatible');
    expect(input('Base URL *').value).toBe('');
    fillRequired();
    expect(createButton().disabled).toBe(true);
    expect((screen.getByRole('button', { name: /Test Connection/ }) as HTMLButtonElement).disabled).toBe(true);

    fireEvent.change(input('Base URL *'), { target: { value: 'https://my-gateway.example/v1' } });
    expect(createButton().disabled).toBe(false);
    expect((screen.getByRole('button', { name: /Test Connection/ }) as HTMLButtonElement).disabled).toBe(false);

    await choose('Provider Type', 'Anthropic compatible');
    expect(input('Base URL *').value).toBe('https://my-gateway.example/v1');
    fireEvent.click(createButton());

    await waitFor(() => expect(mockedInvoke).toHaveBeenCalledWith('create_provider', {
      config: expect.objectContaining({
        name: 'My service',
        provider_type: 'anthropic',
        base_url: 'https://my-gateway.example/v1',
        api_key: 'synthetic-key',
        config: { vendor_id: 'custom' },
      }),
    }));
  });

  it('drops a model preview and the typed test state when the vendor changes', async () => {
    mockHost([], (command) => command === 'list_models_for_config' ? {
      success: true,
      data: [{
        id: 'm1', provider_id: '', model_id: 'preview-model', model_name: 'preview-model', model_type: 'text-generation',
        supports_streaming: true, supports_vision: false, cached_at: '2026-10-30T00:00:00Z',
      }],
    } : undefined);
    await openNewForm();
    fireEvent.change(input('API Key *'), { target: { value: 'synthetic-key' } });
    fireEvent.click(screen.getByRole('button', { name: /Refresh Models/ }));
    expect(await screen.findByRole('heading', { name: 'preview-model' })).toBeTruthy();

    await choose('Vendor', 'Acme Cloud');
    expect(screen.queryByRole('heading', { name: 'preview-model' })).toBeNull();
  });
});

describe('Advanced options (AC16)', () => {
  it('are collapsed by default and the plain-HTTP risk note shows only while the box is checked', async () => {
    mockHost();
    await openNewForm();
    const toggle = screen.getByRole('button', { name: 'Advanced options' });
    expect(toggle.getAttribute('aria-expanded')).toBe('false');
    expect(screen.queryByRole('checkbox', { name: 'Allow plain HTTP' })).toBeNull();
    expect(screen.queryByRole('combobox', { name: 'Auth header style' })).toBeNull();

    fireEvent.click(toggle);
    expect(toggle.getAttribute('aria-expanded')).toBe('true');
    expect(screen.getByRole('combobox', { name: 'Auth header style' })).toBeTruthy();
    expect(screen.queryByText(/prompts unencrypted/)).toBeNull();

    fireEvent.click(screen.getByRole('checkbox', { name: 'Allow plain HTTP' }));
    expect(screen.getByText(/sends your API key and prompts unencrypted/)).toBeTruthy();

    fireEvent.click(screen.getByRole('checkbox', { name: 'Allow plain HTTP' }));
    expect(screen.queryByText(/prompts unencrypted/)).toBeNull();
  });

  it('saves vendor_id, provider_type, auth_style and allow_insecure_http in the payload', async () => {
    mockHost();
    await openNewForm();
    fillRequired();
    fireEvent.change(input('Base URL *'), { target: { value: 'http://gateway.example/v1' } });
    fireEvent.click(screen.getByRole('button', { name: 'Advanced options' }));
    await choose('Auth header style', 'x-api-key');
    fireEvent.click(screen.getByRole('checkbox', { name: 'Allow plain HTTP' }));
    fireEvent.click(createButton());

    await waitFor(() => expect(mockedInvoke).toHaveBeenCalledWith('create_provider', {
      config: expect.objectContaining({
        provider_type: 'openai_compatible',
        base_url: 'http://gateway.example/v1',
        config: { vendor_id: 'openai', auth_style: 'x-api-key', allow_insecure_http: true },
      }),
    }));
  });

  it('omits auth_style and allow_insecure_http when they are left at their defaults', async () => {
    mockHost();
    await openNewForm();
    fillRequired();
    fireEvent.click(screen.getByRole('button', { name: 'Advanced options' }));
    await choose('Auth header style', 'Both');
    await choose('Auth header style', 'Default');
    fireEvent.click(screen.getByRole('checkbox', { name: 'Allow plain HTTP' }));
    fireEvent.click(screen.getByRole('checkbox', { name: 'Allow plain HTTP' }));
    fireEvent.click(createButton());

    await waitFor(() => expect(mockedInvoke).toHaveBeenCalledWith('create_provider', {
      config: expect.objectContaining({ config: { vendor_id: 'openai' } }),
    }));
  });

  it('notes cleartext to a loopback or private address without blocking, and not for a public host', async () => {
    mockHost();
    await openNewForm();
    const note = /http:\/\/ on this machine or a private network/;
    for (const local of ['http://localhost:11434/v1', 'http://127.0.0.1:8080', 'http://[::1]:8080', 'http://10.1.2.3/v1', 'http://172.20.0.5/v1', 'http://192.168.1.9/v1']) {
      fireEvent.change(input('Base URL *'), { target: { value: local } });
      expect(screen.getByText(note), local).toBeTruthy();
    }
    for (const other of ['http://172.32.0.5/v1', 'http://example.com/v1', 'https://localhost/v1']) {
      fireEvent.change(input('Base URL *'), { target: { value: other } });
      expect(screen.queryByText(note), other).toBeNull();
    }
    fireEvent.change(input('Base URL *'), { target: { value: 'http://localhost:11434/v1' } });
    fillRequired();
    expect(createButton().disabled).toBe(false);
  });
});

describe('Saved providers and the list (AC16)', () => {
  const acme = { ...savedBase, id: 'p-acme', name: 'Acme saved', base_url: 'https://llm.acme.example/v1', config: { vendor_id: 'acme', retry: { max_attempts: 3 }, auth_style: 'both' } };
  const legacyOfficial = { ...savedBase, id: 'p-legacy', name: 'Legacy official', base_url: 'https://api.openai.com/v1' };
  const legacyThirdParty = { ...savedBase, id: 'p-third', name: 'Legacy third party', base_url: 'https://proxy.thirdparty.example/v1' };
  const lookalike = { ...savedBase, id: 'p-evil', name: 'Lookalike host', base_url: 'https://api.openai.com.evil.example/v1' };
  const unknownVendor = { ...savedBase, id: 'p-ghost', name: 'Ghost vendor', base_url: 'https://ghost.example/v1', config: { vendor_id: 'ghost' } };

  const card = (name: string) => screen.getByRole('heading', { name, level: 3 }).closest('.cursor-pointer') as HTMLElement;

  it('shows the vendor display name, with the protocol as secondary text, and infers it for legacy providers', async () => {
    mockHost([acme, legacyOfficial, legacyThirdParty, lookalike, unknownVendor]);
    render(<ProviderManagement />);

    await waitFor(() => expect(within(card('Acme saved')).getByText('Acme Cloud')).toBeTruthy());
    expect(within(card('Acme saved')).getByText('OpenAI compatible')).toBeTruthy();
    expect(within(card('Legacy official')).getByText('OpenAI')).toBeTruthy();
    expect(within(card('Legacy third party')).getByText('Custom')).toBeTruthy();
    expect(within(card('Lookalike host')).getByText('Custom')).toBeTruthy();
    expect(within(card('Ghost vendor')).getByText('ghost')).toBeTruthy();
  });

  it('shows the saved vendor in the edit form and keeps unrelated config keys on save', async () => {
    mockHost([acme], (command) => command === 'update_provider'
      ? { success: true, data: acme }
      : undefined);
    render(<ProviderManagement />);
    fireEvent.click(await screen.findByRole('heading', { name: 'Acme saved', level: 3 }));

    await waitFor(() => expect(screen.getByRole('combobox', { name: 'Vendor' }).textContent).toContain('Acme Cloud'));
    fireEvent.change(input('Name *'), { target: { value: 'Acme renamed' } });
    fireEvent.click(screen.getByRole('button', { name: /Save Changes/ }));

    await waitFor(() => expect(mockedInvoke).toHaveBeenCalledWith('update_provider', {
      id: 'p-acme',
      config: expect.objectContaining({
        name: 'Acme renamed',
        config: { vendor_id: 'acme', retry: { max_attempts: 3 }, auth_style: 'both' },
      }),
    }));
  });

  it('infers the vendor of a legacy provider in the edit form without writing it back', async () => {
    mockHost([legacyOfficial, legacyThirdParty], (command) => command === 'update_provider'
      ? { success: true, data: legacyThirdParty }
      : undefined);
    render(<ProviderManagement />);
    fireEvent.click(await screen.findByRole('heading', { name: 'Legacy official', level: 3 }));
    await waitFor(() => expect(screen.getByRole('combobox', { name: 'Vendor' }).textContent).toContain('OpenAI'));

    fireEvent.click(screen.getByRole('heading', { name: 'Legacy third party', level: 3 }));
    await waitFor(() => expect(screen.getByRole('combobox', { name: 'Vendor' }).textContent).toContain('Custom'));
    fireEvent.change(input('Name *'), { target: { value: 'Renamed' } });
    fireEvent.click(screen.getByRole('button', { name: /Save Changes/ }));
    await waitFor(() => expect(mockedInvoke).toHaveBeenCalledWith('update_provider', {
      id: 'p-third',
      config: expect.objectContaining({ name: 'Renamed', config: undefined }),
    }));
  });

  it('requires a new API key once the vendor is changed on a saved provider', async () => {
    mockHost([acme]);
    render(<ProviderManagement />);
    fireEvent.click(await screen.findByRole('heading', { name: 'Acme saved', level: 3 }));
    await waitFor(() => expect(screen.getByRole('combobox', { name: 'Vendor' }).textContent).toContain('Acme Cloud'));

    await choose('Vendor', 'OpenAI');
    expect(screen.getByText(/Enter a new API key before testing or saving/)).toBeTruthy();
    expect((screen.getByRole('button', { name: /Save Changes/ }) as HTMLButtonElement).disabled).toBe(true);
    // the saved URL stays: only a new-provider form pre-fills it
    expect(input('Base URL *').value).toBe('https://llm.acme.example/v1');
  });
});
