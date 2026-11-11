import { Marked, type Tokens, type TokenizerExtension } from 'marked';

export interface MathToken extends Tokens.Generic { type: 'richMath'; text: string; display: boolean }

function escaped(source: string, index: number): boolean {
  let count = 0;
  while (index > 0 && source[--index] === '\\') count++;
  return count % 2 === 1;
}

const blockMath: TokenizerExtension = {
  name: 'richMath', level: 'block',
  start: source => { const index = source.search(/^(?: {0,3})(?:\$\$|\\\[)/m); return index < 0 ? undefined : index; },
  tokenizer(source) {
    const start = /^( {0,3})(\$\$|\\\[)/.exec(source);
    if (!start) return undefined;
    const delimiter = start[2] === '$$' ? '$$' : '\\]';
    const offset = start[0].length;
    let end = source.indexOf(delimiter, offset);
    while (end !== -1 && escaped(source, end)) end = source.indexOf(delimiter, end + delimiter.length);
    if (end === -1) return undefined;
    const tail = source.slice(end + delimiter.length).match(/^[ \t]*(?:\n|$)/);
    if (!tail) return undefined;
    return { type: 'richMath', raw: source.slice(0, end + delimiter.length + tail[0].length), text: source.slice(offset, end).trim(), display: true };
  },
};

const inlineMath: TokenizerExtension = {
  name: 'richMath', level: 'inline',
  start: source => { const index = source.search(/\$|\\\(/); return index < 0 ? undefined : index; },
  tokenizer(source) {
    const parenthesis = source.startsWith('\\(');
    if (!parenthesis && (!source.startsWith('$') || source.startsWith('$$') || /\s/.test(source[1] ?? ' '))) return undefined;
    const offset = parenthesis ? 2 : 1;
    const delimiter = parenthesis ? '\\)' : '$';
    let end = source.indexOf(delimiter, offset);
    while (end !== -1 && escaped(source, end)) end = source.indexOf(delimiter, end + delimiter.length);
    if (end <= offset) return undefined;
    const text = source.slice(offset, end);
    if (text.includes('\n') || (!parenthesis && /\s$/.test(text))) return undefined;
    return { type: 'richMath', raw: source.slice(0, end + delimiter.length), text, display: false };
  },
};

// Private lexer options: never mutate marked's global extensions or reinterpret code tokens.
export const richMarkdownOptions = new Marked({ extensions: [blockMath, inlineMath] }).defaults;
