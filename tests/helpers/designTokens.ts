// Test-only readers for src/ui: a small CSS block parser, WCAG contrast, and a Tailwind compile of the
// real entry stylesheet. Kept out of src/ so no runtime code depends on them.
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const require = createRequire(join(root, 'package.json'));

export const uiPath = (...parts: string[]) => join(root, 'src/ui', ...parts);
export const readUi = (...parts: string[]) => readFileSync(uiPath(...parts), 'utf8');

export interface Block {
  prelude: string;
  declarations: Map<string, string>;
}

/** Top-level `prelude { name: value; ... }` blocks of a flat stylesheet (no nested rules needed here). */
export function parseBlocks(css: string): Block[] {
  const source = css.replace(/\/\*[\s\S]*?\*\//g, '');
  const blocks: Block[] = [];
  const pattern = /([^{}]+)\{([^{}]*)\}/g;
  for (let match = pattern.exec(source); match; match = pattern.exec(source)) {
    const declarations = new Map<string, string>();
    for (const part of match[2].split(';')) {
      const colon = part.indexOf(':');
      if (colon > 0) declarations.set(part.slice(0, colon).trim(), part.slice(colon + 1).trim());
    }
    blocks.push({ prelude: match[1].trim().replace(/\s+/g, ' '), declarations });
  }
  return blocks;
}

const LIGHT_PRELUDES = new Set(['@theme static', ':root']);
const DARK_PRELUDE = ':root[data-theme="dark"]';
const TOKEN_FILES = ['colors.css', 'scale.css'];

export function tokenBlocks(): Block[] {
  return TOKEN_FILES.flatMap((file) => parseBlocks(readUi('tokens', file)));
}

/** `--name` -> value for the light theme and for the dark theme (light values overridden by dark). */
export function themeTokens(): { light: Map<string, string>; dark: Map<string, string>; darkOnly: Map<string, string> } {
  const light = new Map<string, string>();
  const darkOnly = new Map<string, string>();
  for (const block of tokenBlocks()) {
    const target = LIGHT_PRELUDES.has(block.prelude) ? light : block.prelude === DARK_PRELUDE ? darkOnly : null;
    if (!target) continue;
    for (const [name, value] of block.declarations) target.set(name, value);
  }
  light.delete('--color-*');
  return { light, darkOnly, dark: new Map([...light, ...darkOnly]) };
}

// WCAG 2.x relative luminance and contrast ratio.
export function parseHex(hex: string): [number, number, number] {
  const match = /^#([0-9a-f]{6})$/i.exec(hex.trim());
  if (!match) throw new Error(`expected a #rrggbb colour, got ${JSON.stringify(hex)}`);
  const value = parseInt(match[1], 16);
  return [(value >> 16) & 255, (value >> 8) & 255, value & 255];
}

function luminance(hex: string): number {
  const [r, g, b] = parseHex(hex).map((channel) => {
    const c = channel / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

export function contrast(a: string, b: string): number {
  const [x, y] = [luminance(a), luminance(b)];
  return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
}

/** Compile src/ui/styles.css with the real Tailwind compiler and build the CSS for `candidates`. */
export async function compileUi(candidates: string[]): Promise<string> {
  const { compile } = require('tailwindcss') as typeof import('tailwindcss');
  const tailwindDir = dirname(require.resolve('tailwindcss/package.json'));
  const entry = uiPath('styles.css');
  const compiler = await compile(readFileSync(entry, 'utf8'), {
    base: dirname(entry),
    async loadStylesheet(id, base) {
      const path = id.startsWith('tailwindcss/') ? join(tailwindDir, id.slice('tailwindcss/'.length)) : resolve(base, id);
      return { path, base: dirname(path), content: readFileSync(path, 'utf8') };
    },
  });
  return compiler.build(candidates);
}
