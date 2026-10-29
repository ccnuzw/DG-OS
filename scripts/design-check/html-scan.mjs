// R1/R2 for HTML documents (index.html). ESLint cannot parse HTML and Stylelint would need postcss-html for
// one 14-line file, so this is a small attribute scanner. It checks tags and attribute values only: script bodies
// and comments are blanked first (keeping line breaks), so a string inside <script> is never reported.
import {
  HEX_COLOR,
  COLOR_FUNCTION,
  TAILWIND_ARBITRARY_VALUE,
  TAILWIND_ARBITRARY_VARIANT,
  TAILWIND_BRACKET_TOKEN,
} from './patterns.mjs';

const TAG = /<([a-zA-Z][\w-]*)((?:\s+[^\s"'<>/=]+(?:\s*=\s*(?:"[^"]*"|'[^']*'|[^\s"'>]+))?)*)\s*\/?>/g;
const ATTRIBUTE = /([^\s"'<>/=]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'>]+)))?/g;

const blank = (match) => match.replace(/[^\n]/g, ' ');

const CLASS_RULES = [
  { re: new RegExp(HEX_COLOR), tag: 'R2', rule: 'html/hard-coded-color', message: '十六进制颜色字面量' },
  { re: new RegExp(COLOR_FUNCTION), tag: 'R2', rule: 'html/hard-coded-color', message: 'rgb()/hsl() 等颜色函数' },
  { re: new RegExp(TAILWIND_ARBITRARY_VALUE), tag: 'R2', rule: 'html/tailwind-arbitrary-value', message: 'Tailwind 任意值' },
  { re: new RegExp(TAILWIND_ARBITRARY_VARIANT), tag: 'R2', rule: 'html/tailwind-arbitrary-value', message: 'Tailwind 任意变体' },
  { re: new RegExp(TAILWIND_BRACKET_TOKEN), tag: 'R2', rule: 'html/tailwind-arbitrary-value', message: 'Tailwind 任意选择器或属性' },
];

/** @returns {{line: number, column: number, tag: string, rule: string, message: string}[]} */
export function scanHtml(source) {
  const text = source
    .replace(/<!--[\s\S]*?-->/g, blank)
    .replace(/(<script\b[^>]*>)([\s\S]*?)(<\/script>)/gi, (_, open, body, close) => open + blank(body) + close);
  const findings = [];
  const position = (offset) => {
    const before = text.slice(0, offset);
    const line = before.split('\n').length;
    return { line, column: offset - before.lastIndexOf('\n') };
  };

  for (const tag of text.matchAll(TAG)) {
    const [, name, attributes] = tag;
    if (name.toLowerCase() === 'style') {
      findings.push({ ...position(tag.index), tag: 'R1', rule: 'html/inline-style', message: '<style> 元素' });
    }
    const attributesOffset = tag.index + 1 + name.length;
    for (const attribute of attributes.matchAll(ATTRIBUTE)) {
      const [, attributeName, doubleQuoted, singleQuoted, bare] = attribute;
      const value = doubleQuoted ?? singleQuoted ?? bare ?? '';
      const at = position(attributesOffset + attribute.index);
      if (attributeName.toLowerCase() === 'style') {
        findings.push({ ...at, tag: 'R1', rule: 'html/inline-style', message: 'style 属性' });
      } else if (attributeName.toLowerCase() === 'class') {
        for (const { re, ...rest } of CLASS_RULES) {
          if (re.test(value)) findings.push({ ...at, ...rest });
        }
      }
    }
  }
  return findings;
}
