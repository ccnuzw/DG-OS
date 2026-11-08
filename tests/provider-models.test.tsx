import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import ProviderModels from '../src/components/ProviderModels';
import type { ProviderModelsProps } from '../src/components/ProviderModels';
import { setLocaleForTesting } from '../src/i18n/runtime';
import { MODEL_KINDS } from '../src/stores/providerStore';
import type { Model } from '../src/stores/providerStore';

function model(id: string, overrides: Partial<Model> = {}): Model {
  return {
    id: `record-${id}`, provider_id: 'provider', model_id: id, model_name: id,
    model_type: 'text-generation', supports_streaming: false, supports_vision: false,
    cached_at: '2026-11-08T00:00:00Z', ...overrides,
  };
}

function props(overrides: Partial<ProviderModelsProps> = {}): ProviderModelsProps {
  return {
    models: [], editMode: true, busy: false, refreshing: false, mutationBusy: null,
    error: null, onRefresh: vi.fn(), onAdd: vi.fn(async () => true), onRemove: vi.fn(async () => true),
    ...overrides,
  };
}

async function chooseOption(label: string, option: string) {
  const trigger = screen.getByRole('combobox', { name: label });
  trigger.focus();
  fireEvent.keyDown(trigger, { key: 'Enter' });
  const listbox = await screen.findByRole('listbox');
  fireEvent.click(within(listbox).getByRole('option', { name: option, exact: true }));
  await waitFor(() => expect(screen.queryByRole('listbox')).toBeNull());
}

beforeEach(() => setLocaleForTesting('en-US'));
afterEach(() => cleanup());

describe('Provider model directory', () => {
  it('searches both name and ID without case sensitivity and never repeats identical name/ID', () => {
    render(<ProviderModels {...props({ models: [
      model('vendor/ALPHA-v2', { model_name: 'Friendly Alpha' }),
      model('same-identity'),
    ] })} />);
    expect(screen.getAllByText('same-identity')).toHaveLength(1);
    fireEvent.change(screen.getByRole('searchbox', { name: 'Search name or ID' }), { target: { value: 'FRIENDLY' } });
    expect(screen.getByRole('button', { name: /Friendly Alpha/ })).toBeTruthy();
    expect(screen.queryByText('same-identity')).toBeNull();
    fireEvent.change(screen.getByRole('searchbox', { name: 'Search name or ID' }), { target: { value: 'alpha-V2' } });
    expect(screen.getByRole('button', { name: /Friendly Alpha/ })).toBeTruthy();
    fireEvent.change(screen.getByRole('searchbox', { name: 'Search name or ID' }), { target: { value: 'missing' } });
    expect(screen.getByRole('heading', { name: 'No matching models' })).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Clear filters' }));
    expect(screen.getByText('same-identity')).toBeTruthy();
  });

  it('normalizes legacy chat and offers only present types with their counts', async () => {
    render(<ProviderModels {...props({ models: [
      model('legacy', { model_type: 'chat' }), model('text'),
      model('vector', { model_type: 'embedding' }),
    ] })} />);
    const trigger = screen.getByRole('combobox', { name: 'Model type' });
    trigger.focus();
    fireEvent.keyDown(trigger, { key: 'Enter' });
    const listbox = await screen.findByRole('listbox');
    expect(within(listbox).getAllByRole('option').map((item) => item.textContent)).toEqual([
      'All types (3)', 'Text generation (2)', 'Embedding (1)',
    ]);
    fireEvent.click(within(listbox).getByRole('option', { name: 'Embedding (1)' }));
    await waitFor(() => expect(screen.queryByRole('listbox')).toBeNull());
    expect(screen.getByRole('button', { name: /vector/ })).toBeTruthy();
    expect(screen.queryByText('legacy')).toBeNull();
    await chooseOption('Model type', 'Text generation (2)');
    expect(screen.getByText('legacy')).toBeTruthy();
    expect(screen.getByText('text')).toBeTruthy();
  });

  it('localizes every supported type and the manual kind choices', async () => {
    setLocaleForTesting('zh-CN');
    render(<ProviderModels {...props({ models: MODEL_KINDS.map((kind) => model(kind, { model_type: kind })) })} />);
    expect(screen.getByText('文本生成')).toBeTruthy();
    expect(screen.getByText('文本嵌入')).toBeTruthy();
    expect(screen.getByText('重排序')).toBeTruthy();
    expect(screen.getByText('图像生成')).toBeTruthy();
    expect(screen.getByText('视频生成')).toBeTruthy();
    expect(screen.getByText('语音合成')).toBeTruthy();
    expect(screen.getByText('语音转写')).toBeTruthy();
    expect(screen.getByText('内容审核')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: '手动添加模型' }));
    await chooseOption('类型（可选）', '内容审核');
    expect(screen.getByRole('combobox', { name: '类型（可选）' }).textContent).toContain('内容审核');
  });

  it('keeps a clear exit when the selected category becomes empty', async () => {
    const options = props({ models: [model('chat'), model('vector', { model_type: 'embedding' })] });
    const { rerender } = render(<ProviderModels {...options} />);
    await chooseOption('Model type', 'Embedding (1)');
    rerender(<ProviderModels {...options} models={[model('chat')]} />);
    expect(screen.getByRole('combobox', { name: 'Model type' }).textContent).toContain('Embedding (0)');
    expect(screen.getByRole('heading', { name: 'No matching models' })).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Clear filters' }));
    expect(screen.getByText('chat')).toBeTruthy();
  });

  it('pages 20 rows, resets on searches/types, clamps on removal, and preserves row identity', async () => {
    const models = Array.from({ length: 41 }, (_, index) => model(`item-${index}`, {
      model_name: `Model ${index}`, source: 'remote', model_type: index === 40 ? 'embedding' : 'text-generation',
    }));
    const options = props({ models });
    const { rerender } = render(<ProviderModels {...options} />);
    expect(screen.getAllByRole('listitem')).toHaveLength(20);
    fireEvent.click(screen.getByRole('button', { name: /^Model 0 / }));
    expect(screen.getByRole('button', { name: /^Model 0 / }).getAttribute('aria-expanded')).toBe('true');
    fireEvent.click(screen.getByRole('button', { name: 'Next' }));
    expect(screen.getByText('Page 2 of 3')).toBeTruthy();
    expect(screen.getByRole('button', { name: /^Model 20 / }).getAttribute('aria-expanded')).toBe('false');
    fireEvent.change(screen.getByRole('searchbox'), { target: { value: 'Model' } });
    expect(screen.getByText('Page 1 of 3')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Next' }));
    await chooseOption('Model type', 'Text generation (40)');
    expect(screen.getByText('Page 1 of 2')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Clear filters' }));
    fireEvent.click(screen.getByRole('button', { name: 'Next' }));
    fireEvent.click(screen.getByRole('button', { name: 'Next' }));
    expect(screen.getAllByRole('listitem')).toHaveLength(1);
    rerender(<ProviderModels {...options} models={models.slice(0, 21)} />);
    expect(screen.getByText('Page 2 of 2')).toBeTruthy();
    expect(screen.getByText('Model 20')).toBeTruthy();
    rerender(<ProviderModels {...options} models={models.slice(0, 5)} />);
    expect(screen.queryByRole('navigation', { name: 'Model pages' })).toBeNull();
    expect(screen.getAllByRole('listitem')).toHaveLength(5);
    rerender(<ProviderModels {...options} />);
    expect(screen.getByText('Page 1 of 3')).toBeTruthy();
  });

  it('expands only one model and distinguishes unknown, unsupported, and pending classification', () => {
    render(<ProviderModels {...props({ models: [
      model('legacy', { kind_source: 'default', supports_streaming: true }),
      model('explicit', { source: 'remote', kind_source: 'protocol', context_window: 32000, max_output_tokens: 4000,
        capabilities: { streaming: 'supported', vision: 'unsupported', tools: 'unknown', reasoning: 'unknown', structured_output: 'unknown' } }),
    ] })} />);
    expect(screen.getByText('Pending classification confirmation')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: /^legacy / }));
    const legacy = screen.getByRole('button', { name: /^legacy / }).closest('li')!;
    expect(within(legacy).getByText('Default type — pending confirmation')).toBeTruthy();
    expect(within(legacy).queryByText('Not supported')).toBeNull();
    const vision = within(legacy).getByText('Vision').closest('div')!;
    expect(within(vision).getByText('Unknown')).toBeTruthy();
    expect(within(legacy).getAllByText('Unknown').length).toBeGreaterThanOrEqual(6);
    fireEvent.click(screen.getByRole('button', { name: /^explicit / }));
    expect(screen.getByRole('button', { name: /^legacy / }).getAttribute('aria-expanded')).toBe('false');
    const explicit = screen.getByRole('button', { name: /^explicit / }).closest('li')!;
    expect(within(explicit).getByText('Not supported')).toBeTruthy();
    expect(within(explicit).getByText('Provider model list')).toBeTruthy();
    expect(within(explicit).getByText('Reported by provider')).toBeTruthy();
    expect(within(explicit).getByText('32,000 tokens')).toBeTruthy();
    expect(within(explicit).getByText('4,000 tokens')).toBeTruthy();
  });

  it('retains a failed manual add, then clears and closes only after success', async () => {
    const onAdd = vi.fn().mockResolvedValueOnce(false).mockResolvedValueOnce(true);
    render(<ProviderModels {...props({ onAdd })} />);
    fireEvent.click(screen.getByRole('button', { name: 'Add model manually' }));
    fireEvent.change(screen.getByLabelText('Model ID'), { target: { value: '  vendor/custom  ' } });
    await chooseOption('Type (optional)', 'Embedding');
    fireEvent.click(screen.getByRole('button', { name: 'Add model', exact: true }));
    await waitFor(() => expect(onAdd).toHaveBeenCalledWith('vendor/custom', 'embedding'));
    await waitFor(() => expect((screen.getByRole('button', { name: 'Add model', exact: true }) as HTMLButtonElement).disabled).toBe(false));
    expect((screen.getByLabelText('Model ID') as HTMLInputElement).value).toBe('  vendor/custom  ');
    expect(screen.getByRole('combobox', { name: 'Type (optional)' }).textContent).toContain('Embedding');
    fireEvent.click(screen.getByRole('button', { name: 'Add model', exact: true }));
    await waitFor(() => expect(screen.queryByLabelText('Model ID')).toBeNull());
    fireEvent.click(screen.getByRole('button', { name: 'Add model manually' }));
    expect((screen.getByLabelText('Model ID') as HTMLInputElement).value).toBe('');
    expect(screen.getByRole('combobox', { name: 'Type (optional)' }).textContent).toContain('Detect automatically');
  });

  it('sends an omitted kind for automatic detection and locks mutations while adding', async () => {
    const onAdd = vi.fn(async () => true);
    render(<ProviderModels {...props({ onAdd })} />);
    fireEvent.click(screen.getByRole('button', { name: 'Add model manually' }));
    expect((screen.getByRole('button', { name: 'Add model', exact: true }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.change(screen.getByLabelText('Model ID'), { target: { value: 'manual-id' } });
    fireEvent.click(screen.getByRole('button', { name: 'Add model', exact: true }));
    expect(onAdd).toHaveBeenCalledWith('manual-id', undefined);
    expect((screen.getByLabelText('Model ID') as HTMLInputElement).disabled).toBe(true);
    expect((screen.getByRole('button', { name: 'Cancel' }) as HTMLButtonElement).disabled).toBe(true);
    await waitFor(() => expect(screen.queryByLabelText('Model ID')).toBeNull());
  });

  it('allows removal only for an expanded manual row and retains details on failure', async () => {
    const manual = model('manual', { source: 'manual', kind_source: 'manual' });
    const onRemove = vi.fn().mockResolvedValueOnce(false).mockResolvedValueOnce(true);
    render(<ProviderModels {...props({ models: [manual, model('remote', { source: 'remote' })], onRemove })} />);
    expect(screen.queryByRole('button', { name: /Remove manual model/ })).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: /^remote / }));
    expect(screen.queryByRole('button', { name: /Remove manual model/ })).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: /^manual / }));
    fireEvent.click(screen.getByRole('button', { name: 'Remove manual model manual' }));
    await waitFor(() => expect(onRemove).toHaveBeenCalledWith(manual));
    await waitFor(() => expect((screen.getByRole('button', { name: 'Remove manual model manual' }) as HTMLButtonElement).disabled).toBe(false));
    fireEvent.click(screen.getByRole('button', { name: 'Remove manual model manual' }));
    await waitFor(() => expect(screen.queryByRole('button', { name: /Remove manual model/ })).toBeNull());
  });

  it('always renders the directory with preview, blocked, and error states', () => {
    const options = props({ editMode: false, blockedReason: 'Enter a credential first.', error: 'Fetch failed.' });
    const { rerender } = render(<ProviderModels {...options} />);
    expect(screen.getByRole('region', { name: 'Models' })).toBeTruthy();
    expect(screen.getByRole('heading', { name: 'No models yet' })).toBeTruthy();
    expect(screen.getByText('Enter a credential first.')).toBeTruthy();
    expect(screen.getByRole('alert').textContent).toBe('Fetch failed.');
    expect(screen.queryByRole('button', { name: 'Add model manually' })).toBeNull();
    expect((screen.getByRole('button', { name: 'Fetch model preview' }) as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByRole('searchbox') as HTMLInputElement).disabled).toBe(true);
    expect((screen.getByRole('combobox', { name: 'Model type' }) as HTMLButtonElement).disabled).toBe(true);
    rerender(<ProviderModels {...options} blockedReason={undefined} error={null} />);
    fireEvent.click(screen.getByRole('button', { name: 'Fetch model preview' }));
    expect(options.onRefresh).toHaveBeenCalledTimes(1);
    rerender(<ProviderModels {...options} editMode busy blockedReason={undefined} error={null} />);
    for (const button of screen.getAllByRole('button')) expect((button as HTMLButtonElement).disabled).toBe(true);
  });
});
