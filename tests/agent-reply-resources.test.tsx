import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import type { ReactNode } from 'react';
import { AgentNativeResources, AgentReplyCitation, AgentReplySourcePreview, AgentReplySourcesProvider } from '../src/components/development/AgentReplyResources';
import { agentReplySources, boundedResourceFields, decodeNativeImage, nativeResources, NATIVE_RESOURCE_LIMITS } from '../src/components/development/nativeReplyResourceData';
import type { AgentEvent } from '../src/stores/agentRunnerStore';
import type { AgentReferenceActions } from '../src/components/development/eventOutputReferences';
import { EventContent } from '../src/components/development/AgentReplyContent';

vi.mock('../src/i18n', () => ({ useLocale: () => ({ t: (key: string) => key }), t: (key: string) => key, formatDateTime: String }));
vi.mock('../src/components/chat/SourcePreview', () => ({ ConfirmedExternalLink: ({ url, children }: { url: string; children: ReactNode }) => <button type="button" data-external-url={url}>{children}</button> }));
afterEach(() => { cleanup(); vi.unstubAllGlobals(); vi.clearAllMocks(); });
const PNG = 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Wl6bXkAAAAASUVORK5CYII=';
const event = (data: AgentEvent['data'], kind = 'message', seq = 1, run = 'run'): AgentEvent => ({ seq, kind, data, text: null, run });

it('uses only linked native WebSearch object hits and never invents IDs or retrieval dates', () => {
  const use = event({ type: 'assistant', content: [{ type: 'tool_use', id: 'outer-search', name: 'WebSearch' }] });
  const result = event({ type: 'user', content: [{ type: 'tool_result', tool_use_id: 'outer-search', content: 'https://body.invalid' }], tool_use_result: { results: ['not evidence https://string.invalid', { tool_use_id: 'native-inner-id', content: [{ title: 'Actual hit', url: 'https://example.org/page', snippet: 'Received excerpt' }] }] } }, 'message', 2);
  expect(agentReplySources([use, result])).toEqual([{ url: 'https://example.org/page', title: 'Actual hit', excerpt: 'Received excerpt', kind: 'search_result', origin: 'webSearch', eventSeq: 2, truncated: false }]);
  expect(agentReplySources([result])).toEqual([]);
  expect(agentReplySources([use, { ...result, run: 'other' }])).toEqual([]);
  expect(agentReplySources([event({ type: 'mcpToolCall', result: { sources: [{ id: 'made-up', url: 'https://example.org' }] } }, 'mcpToolCall')])).toEqual([]);
  expect(agentReplySources([event({ type: 'assistant', content: [{ type: 'text', text: '[source](https://example.org)' }] })])).toEqual([]);
});

it('shows native ID-less hits in the source domain without fabricating citation mappings', () => {
  const search = event({ type: 'webSearch', results: [{ url: 'https://example.org', title: 'Search hit' }] }, 'webSearch');
  const view = render(<AgentReplySourcesProvider events={[search]}><AgentReplyCitation id="1">[1]</AgentReplyCitation></AgentReplySourcesProvider>);
  expect(screen.getByText('[1] (agentResources.citationUnavailable)')).toBeTruthy();
  view.rerender(<AgentReplySourcesProvider events={[event({ type: 'webSearch', results: [{ id: 'native-id', url: 'https://example.org', title: 'Search hit' }] }, 'webSearch')]}><AgentReplyCitation id="native-id">[native]</AgentReplyCitation></AgentReplySourcesProvider>);
  expect(screen.getByRole('button', { name: '[native]' })).toBeTruthy();
});

it('projects typed MCP content without executing structured fields or exposing binary data', () => {
  const result = nativeResources({ content: [{ type: 'text', text: 'Actual returned text' }, { type: 'resource', resource: { uri: 'file:///task/result.json', mimeType: 'application/json', text: '{"ok":true}' } }, { type: 'document', source: { type: 'text', media_type: 'text/plain', data: 'Received document text' } }, { type: 'resource_link', uri: 'https://example.org/report.pdf', mimeType: 'application/pdf' }, { type: 'image', mimeType: 'image/png', bytes_len: 68, preview_resource_id: 'opaque', redacted: true }], structuredContent: { ok: true, command: 'do not execute', blob: PNG.repeat(1000) } });
  expect(result.resources.map(item => item.kind)).toEqual(['text', 'resource', 'document', 'resource_link', 'image', 'structured']);
  expect(result.resources[2].text).toBe('Received document text');
  expect(result.resources[2].base64).toBeUndefined();
  expect(result.resources[4]).toMatchObject({ previewId: 'opaque', byteLength: 68, mime: 'image/png' });
  const fields = boundedResourceFields(result.resources[5].fields!);
  expect(fields.truncated).toBe(true);
  expect(JSON.stringify(fields.value)).not.toContain(PNG);
});

it('renders native task resource links as artifacts and never upgrades them to search sources', () => {
  const data = { type: 'task_notification', resource_links: [{ uri: 'https://example.org/result.pdf', name: 'Result', description: 'Native task output', mimeType: 'application/pdf', size: 1200 }] };
  const result = nativeResources(data);
  expect(result.resources[0]).toMatchObject({ kind: 'resource_link', title: 'Result', uri: 'https://example.org/result.pdf', byteLength: 1200, text: 'Native task output' });
  expect(agentReplySources([event(data, 'task_notification')])).toEqual([]);
  expect(nativeResources({ type: 'resource_link', uri: `data:image/png;base64,${PNG}` }).resources[0]).toMatchObject({ omitted: 'unsupported_resource_location' });
  expect(nativeResources({ type: 'resource_link', uri: `data:image/png;base64,${PNG}` }).resources[0].uri).toBeUndefined();
});

it('enforces image MIME, magic, bytes and pixel bounds before browser decoding', () => {
  expect(decodeNativeImage(PNG, 'image/png')).toMatchObject({ width: 1, height: 1 });
  expect(() => decodeNativeImage(PNG, 'image/jpeg')).toThrow('mimeMismatch');
  expect(() => decodeNativeImage(PNG, 'image/svg+xml')).toThrow('unsupportedMime');
  expect(() => decodeNativeImage('!', 'image/png')).toThrow('invalidBase64');
  expect(() => decodeNativeImage('A'.repeat(Math.ceil(NATIVE_RESOURCE_LIMITS.imageBytes / 3) * 4 + 4), 'image/png')).toThrow('imageTooLarge');
  const bytes = Uint8Array.from(atob(PNG), char => char.charCodeAt(0));
  new DataView(bytes.buffer).setUint32(16, 9000);
  expect(() => decodeNativeImage(btoa(String.fromCharCode(...bytes)), 'image/png')).toThrow('invalidDimensions');
});

function imageEnvironment() {
  const pending: { naturalWidth: number; naturalHeight: number; onload: (() => void) | null; onerror: (() => void) | null }[] = [];
  class ReceivedImage {
    naturalWidth = 1; naturalHeight = 1;
    onload: (() => void) | null = null;
    onerror: (() => void) | null = null;
    set src(_value: string) { pending.push(this); }
    removeAttribute() {}
  }
  const NativeURL = URL;
  const createObjectURL = vi.fn(() => 'blob:received');
  const revokeObjectURL = vi.fn();
  vi.stubGlobal('Image', ReceivedImage);
  vi.stubGlobal('URL', class extends NativeURL { static createObjectURL = createObjectURL; static revokeObjectURL = revokeObjectURL; });
  return { pending, createObjectURL, revokeObjectURL };
}

it('loads an authorized received image only after explicit preview and revokes its object URL', async () => {
  const images = imageEnvironment();
  const onPreviewResource = vi.fn(async () => ({ base64: PNG, mime: 'image/png' }));
  const view = render(<AgentNativeResources value={{ type: 'image', mimeType: 'image/png', preview_resource_id: 'opaque', bytes_len: 68 }} onPreviewResource={onPreviewResource} />);
  expect(onPreviewResource).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole('button', { name: 'agentResources.previewImage' }));
  await waitFor(() => expect(images.createObjectURL).toHaveBeenCalledTimes(1));
  expect(onPreviewResource).toHaveBeenCalledWith('opaque');
  act(() => images.pending[0].onload?.());
  expect(screen.getByAltText('agentResources.imageAlt').getAttribute('src')).toBe('blob:received');
  const replacementLoader = vi.fn(async () => ({ base64: PNG, mime: 'image/png' }));
  view.rerender(<AgentNativeResources value={{ type: 'image', mimeType: 'image/png', preview_resource_id: 'opaque', bytes_len: 68 }} onPreviewResource={replacementLoader} />);
  expect(screen.getByAltText('agentResources.imageAlt').getAttribute('src')).toBe('blob:received');
  expect(replacementLoader).not.toHaveBeenCalled();
  expect(images.createObjectURL).toHaveBeenCalledTimes(1);
  expect(images.revokeObjectURL).not.toHaveBeenCalled();
  view.unmount();
  expect(images.revokeObjectURL).toHaveBeenCalledWith('blob:received');
});

it('ignores late image responses after switching content and makes expired previews explicit', async () => {
  const images = imageEnvironment();
  const { promise, resolve: finish } = Promise.withResolvers<{ base64: string; mime: string }>();
  const onPreviewResource = vi.fn(() => promise);
  const view = render(<AgentNativeResources value={{ type: 'image', mimeType: 'image/png', preview_resource_id: 'old' }} onPreviewResource={onPreviewResource} />);
  fireEvent.click(screen.getByRole('button', { name: 'agentResources.previewImage' }));
  view.rerender(<AgentNativeResources value={{ type: 'image', mimeType: 'image/png', preview_resource_id: 'new' }} onPreviewResource={onPreviewResource} />);
  await act(async () => finish({ base64: PNG, mime: 'image/png' }));
  expect(images.createObjectURL).not.toHaveBeenCalled();
  const expired = vi.fn(async () => { throw new Error('expired'); });
  view.rerender(<AgentNativeResources value={{ type: 'image', mimeType: 'image/png', preview_resource_id: 'new' }} onPreviewResource={expired} />);
  fireEvent.click(screen.getByRole('button', { name: 'agentResources.previewImage' }));
  expect(await screen.findByRole('alert')).toHaveProperty('textContent', 'agentResources.previewExpired');
});

it('shows history and unsupported binary boundaries instead of empty previews', () => {
  render(<AgentNativeResources value={[{ type: 'image', mimeType: 'image/png', preview_resource_id: 'expired-history', unavailable_reason: 'image_payload_omitted', bytes_len: 68 }, { type: 'document', mimeType: 'application/pdf', bytes_len: 100, source: { type: 'base64', data: 'JVBERg==' } }, { type: 'text', text: PNG.repeat(1000) }]} />);
  expect(screen.queryByRole('button', { name: 'agentResources.previewImage' })).toBeNull();
  expect(screen.getByText('agentResources.previewUnavailable')).toBeTruthy();
  expect(screen.getByText('agentResources.binaryBoundary')).toBeTruthy();
  expect(screen.getByText('agentResources.binaryOmitted')).toBeTruthy();
  expect(screen.queryByText('JVBERg==')).toBeNull();
});

it('rejects invalid received image metadata before creating a browser object URL', async () => {
  const images = imageEnvironment();
  render(<AgentNativeResources value={{ type: 'image', mimeType: 'image/png', preview_resource_id: 'invalid' }} onPreviewResource={async () => ({ base64: PNG, mime: 'image/jpeg' })} />);
  fireEvent.click(screen.getByRole('button', { name: 'agentResources.previewImage' }));
  expect(await screen.findByRole('alert')).toHaveProperty('textContent', 'agentResources.mimeMismatch');
  expect(images.createObjectURL).not.toHaveBeenCalled();
});

it('keeps remote resources external and routes workspace paths only through the supplied host action', async () => {
  const onOpenReference = vi.fn();
  render(<AgentNativeResources value={[{ type: 'resource_link', uri: 'https://example.org/report.pdf', mimeType: 'application/pdf' }, { type: 'resource', resource: { uri: 'file:///task/result.txt', text: 'Returned snapshot' } }, { type: 'resource_link', uri: 'javascript:alert(1)' }]} onOpenReference={onOpenReference} />);
  expect(onOpenReference).not.toHaveBeenCalled();
  expect(screen.queryByRole('img')).toBeNull();
  expect(screen.getByText('agentResources.remoteBoundary')).toBeTruthy();
  expect(screen.getByText('agentResources.unsupportedLocation')).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: 'agentResources.openFile' }));
  await waitFor(() => expect(onOpenReference).toHaveBeenCalledWith({ path: '/task/result.txt', source: 'tool', diff: false }, 'file', expect.any(Function)));
});

it('closes a source preview when the citation now identifies another native source event', () => {
  const source = { id: 'native-id', url: 'https://example.org/first', title: 'First source', excerpt: 'Received excerpt', kind: 'search_result' as const, origin: 'webSearch' as const, eventSeq: 1, truncated: false };
  const view = render(<AgentReplySourcePreview source={source}>[source]</AgentReplySourcePreview>);
  fireEvent.click(screen.getByRole('button', { name: '[source]' }));
  expect(screen.getByRole('dialog', { name: 'First source' })).toBeTruthy();
  view.rerender(<AgentReplySourcePreview source={{ ...source, url: 'https://example.org/second', title: 'Second source', eventSeq: 2 }}>[source]</AgentReplySourcePreview>);
  expect(screen.queryByRole('dialog')).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: '[source]' }));
  expect(screen.getByRole('dialog', { name: 'Second source' })).toBeTruthy();
});

it('requires fresh image preview consent after switching away and back at the same resource path', async () => {
  const images = imageEnvironment();
  const response = Promise.withResolvers<{ base64: string; mime: string }>();
  const onPreviewResource = vi.fn(() => response.promise);
  const resource = { type: 'image', mimeType: 'image/png', path: '/task/image.png', preview_resource_id: 'first' };
  const view = render(<AgentNativeResources value={resource} onPreviewResource={onPreviewResource} />);
  fireEvent.click(screen.getByRole('button', { name: 'agentResources.previewImage' }));
  expect(onPreviewResource).toHaveBeenCalledExactlyOnceWith('first');
  view.rerender(<AgentNativeResources value={{ ...resource, preview_resource_id: 'second' }} onPreviewResource={onPreviewResource} />);
  view.rerender(<AgentNativeResources value={resource} onPreviewResource={onPreviewResource} />);
  expect(screen.getByRole('button', { name: 'agentResources.previewImage' })).toBeTruthy();
  expect(onPreviewResource).toHaveBeenCalledTimes(1);
  await act(async () => response.resolve({ base64: PNG, mime: 'image/png' }));
  expect(images.createObjectURL).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole('button', { name: 'agentResources.previewImage' }));
  await waitFor(() => expect(onPreviewResource).toHaveBeenCalledTimes(2));
});

it('does not reload a changed inline payload without fresh image preview consent', async () => {
  const images = imageEnvironment();
  const resource = { type: 'image', mimeType: 'image/png', path: '/task/image.png', preview_resource_id: 'same', data: PNG };
  const onPreviewResource = vi.fn(async () => ({ base64: PNG, mime: 'image/png' }));
  const view = render(<AgentNativeResources value={resource} onPreviewResource={onPreviewResource} />);
  fireEvent.click(screen.getByRole('button', { name: 'agentResources.previewImage' }));
  await waitFor(() => expect(images.createObjectURL).toHaveBeenCalledTimes(1));
  view.rerender(<AgentNativeResources value={{ ...resource, data: 'changed' }} onPreviewResource={onPreviewResource} />);
  expect(screen.getByRole('button', { name: 'agentResources.previewImage' })).toBeTruthy();
  expect(onPreviewResource).toHaveBeenCalledTimes(1);
  expect(images.revokeObjectURL).toHaveBeenCalledWith('blob:received');
});

it('opens a file once per explicit action and invalidates the host guard when its location changes', async () => {
  const response = Promise.withResolvers<void>();
  const onOpenReference = vi.fn<NonNullable<AgentReferenceActions['onOpenReference']>>(() => response.promise);
  const view = render(<AgentNativeResources value={{ type: 'resource_link', uri: 'file:///task/first.txt' }} onOpenReference={onOpenReference} />);
  const open = screen.getByRole('button', { name: 'agentResources.openFile' });
  act(() => {
    open.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    open.dispatchEvent(new MouseEvent('click', { bubbles: true }));
  });
  expect(onOpenReference).toHaveBeenCalledTimes(1);
  const current = onOpenReference.mock.calls[0][2] as () => boolean;
  expect(current()).toBe(true);
  view.rerender(<AgentNativeResources value={{ type: 'resource_link', uri: 'file:///task/second.txt' }} onOpenReference={onOpenReference} />);
  expect(current()).toBe(false);
  await act(async () => response.reject(new Error('old request failed')));
  expect(screen.queryByRole('alert')).toBeNull();
  expect(screen.getByRole('button', { name: 'agentResources.openFile' })).toHaveProperty('disabled', false);
});

it('previews a host-redacted nested imageGeneration result only on explicit request and revokes the URL', async () => {
  const images = imageEnvironment();
  const onPreviewResource = vi.fn(async () => ({ base64: PNG, mime: 'image/png' }));
  const value = event({ type: 'imageGeneration', status: 'completed', result: { type: 'image', mimeType: 'image/png', preview_resource_id: 'generated-preview', bytes_len: 68 } }, 'imageGeneration');
  const view = render(<EventContent event={value} engine="codex" onPreviewResource={onPreviewResource} />);
  expect(onPreviewResource).not.toHaveBeenCalled();
  fireEvent.click(screen.getByText('agentReply.imageOutput'));
  expect(await screen.findByText('image · image/png')).toBeTruthy();
  expect(screen.getByText('agentResources.bytes')).toBeTruthy();
  expect(images.createObjectURL).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole('button', { name: 'agentResources.previewImage' }));
  await waitFor(() => expect(images.createObjectURL).toHaveBeenCalledTimes(1));
  expect(onPreviewResource).toHaveBeenCalledExactlyOnceWith('generated-preview');
  act(() => images.pending[0].onload?.());
  expect(screen.getByAltText('agentResources.imageAlt').getAttribute('src')).toBe('blob:received');
  view.unmount();
  expect(images.revokeObjectURL).toHaveBeenCalledExactlyOnceWith('blob:received');
});

it('keeps generated image previews behind the existing MIME and binary safety boundary', async () => {
  const images = imageEnvironment();
  const onPreviewResource = vi.fn(async () => ({ base64: PNG, mime: 'image/jpeg' }));
  const value = { type: 'imageGeneration', result: { type: 'image', mimeType: 'image/svg+xml', preview_resource_id: 'generated-preview', uri: 'data:image/svg+xml;base64,PHN2Zy8+' } };
  const view = render(<AgentNativeResources value={value} onPreviewResource={onPreviewResource} />);
  expect(screen.queryByRole('button', { name: 'agentResources.previewImage' })).toBeNull();
  expect(screen.getByText('agentResources.unsupportedMime')).toBeTruthy();
  expect(view.container.querySelector('img, [data-external-url]')).toBeNull();
  expect(onPreviewResource).not.toHaveBeenCalled();
  view.rerender(<AgentNativeResources value={{ type: 'imageGeneration', result: { type: 'image', mimeType: 'image/png', preview_resource_id: 'generated-preview' } }} onPreviewResource={onPreviewResource} />);
  fireEvent.click(screen.getByRole('button', { name: 'agentResources.previewImage' }));
  expect(await screen.findByRole('alert')).toHaveProperty('textContent', 'agentResources.mimeMismatch');
  expect(images.createObjectURL).not.toHaveBeenCalled();
});
