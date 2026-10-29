// Style-compliance rules R1 (no inline styles), R2 (no hard-coded colours / Tailwind arbitrary values) and
// R5 (no outline removal without a focus-visible style), D068 and AC24.
// Only `no-restricted-syntax` is enabled: this config is a design gate, not a general lint profile.
// Each message starts with its rule id; scripts/design-check.mjs prints that id next to `file:line`.
import tseslint from 'typescript-eslint';
import {
  COLOR_FUNCTION,
  COLOR_KEYS,
  COLOR_KEYWORD_DECLARATION,
  COLOR_KEYWORD_FALLBACK,
  COLOR_KEYWORD_WHOLE,
  HEX_COLOR,
  OUTLINE_WITHOUT_FOCUS_VISIBLE,
  TAILWIND_ARBITRARY_VALUE,
  TAILWIND_ARBITRARY_VARIANT,
  TAILWIND_BRACKET_TOKEN,
} from './scripts/design-check/patterns.mjs';
import { IGNORE_GLOBS, SCRIPT_GLOBS, TOKEN_GLOBS } from './scripts/design-check/scope.mjs';

const R1 = 'R1 禁止内联样式';
const R2 = 'R2 禁止硬编码颜色';
const R2_TW = 'R2 禁止 Tailwind 任意值';
const R5 = 'R5 焦点环';

/** A string literal or template chunk whose text matches `pattern`. */
function text(pattern) {
  return [`Literal[value=/${pattern}/]`, `TemplateElement[value.raw=/${pattern}/]`];
}

const CALLEES = '/^(?:createElement|h|jsx|jsxs|jsxDEV)$/';

const inlineStyle = [
  ['JSXAttribute[name.name="style"]', 'JSX 的 style 属性'],
  ['JSXOpeningElement[name.name="style"]', 'JSX 的 <style> 元素'],
  ['ObjectExpression > Property[computed=false][key.name="style"]', '对象字面量的 style 键（含 createElement/h 的 props）'],
  ['ObjectExpression > Property[key.value="style"]', '对象字面量的 "style" 键'],
  ['MemberExpression[computed=false][property.name="style"]', '.style 成员访问'],
  ['MemberExpression[computed=true][property.value="style"]', '["style"] 成员访问'],
  ['CallExpression[callee.property.name="setAttribute"][arguments.0.value="style"]', 'setAttribute(\'style\', …)'],
  [`CallExpression[callee.name=${CALLEES}][arguments.0.value="style"]`, "createElement('style')"],
  [`CallExpression[callee.property.name=${CALLEES}][arguments.0.value="style"]`, "React.createElement('style')"],
  ['MemberExpression[property.name="adoptedStyleSheets"]', 'adoptedStyleSheets'],
  ['MemberExpression[property.name="insertRule"]', 'insertRule'],
].map(([selector, what]) => ({ selector, message: `${R1}：${what}。改用 @dgos/ui 组件或令牌类（D068）` }));

const hardCodedColor = [
  ...text(HEX_COLOR).map((selector) => ({ selector, message: `${R2}：十六进制颜色字面量。使用令牌（D068）` })),
  ...text(COLOR_FUNCTION).map((selector) => ({ selector, message: `${R2}：rgb()/hsl() 等颜色函数。使用令牌（D068）` })),
  ...text(COLOR_KEYWORD_DECLARATION).map((selector) => ({ selector, message: `${R2}：颜色关键字。使用令牌（D068）` })),
  ...text(COLOR_KEYWORD_FALLBACK).map((selector) => ({ selector, message: `${R2}：var() 的字面颜色回退。使用令牌（D068）` })),
  // `fill="red"` / `{ color: 'red' }`: only where the key says the value is a colour, so enum values like variant: 'red' pass.
  {
    selector: `JSXAttribute[name.name=/${COLOR_KEYS}/] > Literal[value=/${COLOR_KEYWORD_WHOLE}/i]`,
    message: `${R2}：颜色属性的关键字值。使用令牌（D068）`,
  },
  {
    selector: `Property[key.name=/${COLOR_KEYS}/] > Literal[value=/${COLOR_KEYWORD_WHOLE}/i]`,
    message: `${R2}：颜色属性的关键字值。使用令牌（D068）`,
  },
];

const tailwindArbitrary = [
  ...text(TAILWIND_ARBITRARY_VALUE).map((selector) => ({
    selector,
    message: `${R2_TW}：bg-[#fff]、p-[13px]、w-[--radix-x] 这类写法。使用令牌类；Radix 变量写 w-(--radix-x)（D068）`,
  })),
  ...text(TAILWIND_ARBITRARY_VARIANT).map((selector) => ({
    selector,
    message: `${R2_TW}：任意变体（min-[400px]:、supports-[…]:）。data-[…]: 与 aria-[…]: 允许（D068）`,
  })),
  ...text(TAILWIND_BRACKET_TOKEN).map((selector) => ({
    selector,
    message: `${R2_TW}：[&>svg]:、[--x:1px] 这类任意选择器或属性（D068）`,
  })),
];

const focusRing = text(OUTLINE_WITHOUT_FOCUS_VISIBLE).map((selector) => ({
  selector,
  message: `${R5}：outline-none 没有配对的 focus-visible: 样式。改用 focus-visible:outline-2 focus-visible:outline-ring 之类（AC24）`,
}));

const restricted = [...inlineStyle, ...hardCodedColor, ...tailwindArbitrary, ...focusRing];
const restrictedNoColor = [...inlineStyle, ...tailwindArbitrary, ...focusRing];

export default [
  { ignores: IGNORE_GLOBS },
  {
    files: SCRIPT_GLOBS,
    languageOptions: {
      parser: tseslint.parser,
      parserOptions: { ecmaFeatures: { jsx: true }, sourceType: 'module' },
    },
    // A disable comment must not be a way around the gate; the check also reports the attempt (see design-check.mjs).
    linterOptions: { noInlineConfig: true, reportUnusedDisableDirectives: 'off' },
    rules: { 'no-restricted-syntax': ['error', ...restricted] },
  },
  {
    // AC12 exception: token definitions may spell colours. Inline styles and arbitrary values stay forbidden.
    files: TOKEN_GLOBS,
    rules: { 'no-restricted-syntax': ['error', ...restrictedNoColor] },
  },
];
