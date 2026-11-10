import { describe, expect, it } from 'vitest';
import { selectableResourceText, selectedResourceRange } from '../src/components/chat/resourcePreview';

const bytes = (text: string) => new TextEncoder().encode(text);

describe('immutable resource preview byte ranges', () => {
  it('maps normalized textarea newlines back to the original UTF-8 bytes, including a BOM', () => {
    const source = '\uFEFF甲\r\n😀e\u0301\r末\n';
    const preview = selectableResourceText(source);
    expect(preview.text).toBe('\uFEFF甲\n😀e\u0301\n末\n');
    expect(selectedResourceRange(preview, 0, preview.text.length)).toEqual({ start: 0, max_bytes: bytes(source).length });
    const range = selectedResourceRange(preview, 2, 9)!;
    expect(new TextDecoder().decode(bytes(source).slice(range.start, range.start + range.max_bytes))).toBe('\r\n😀e\u0301\r末');
  });

  it.each([[1, 2], [2, 3], [1, 3]])('keeps a selected emoji on Unicode scalar boundaries at %s–%s', (start, end) => {
    // Textarea positions are UTF-16 offsets; a partial surrogate selection must include the scalar.
    const preview = selectableResourceText('A😀中');
    const range = selectedResourceRange(preview, start, end)!;
    expect(range).toEqual({ start: 1, max_bytes: 4 });
    expect(new TextDecoder('utf-8', { fatal: true }).decode(bytes('A😀中').slice(range.start, range.start + range.max_bytes))).toBe('😀');
  });

  it.each([[-1, 1], [0, 0], [2, 1], [0, 5]])('does not create a reference from an empty or out-of-bounds selection at %s–%s', (start, end) => {
    expect(selectedResourceRange(selectableResourceText('A😀中'), start, end)).toBeNull();
  });
});
