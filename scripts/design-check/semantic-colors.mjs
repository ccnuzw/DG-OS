// First-party consumers use semantic roles; the public FR-005 utilities remain defined for D067.
export const LEGACY_COLOR_NAMES = [
  'background', 'foreground', 'card', 'card-foreground', 'popover', 'popover-foreground',
  'primary-foreground', 'secondary', 'secondary-foreground', 'muted', 'muted-foreground',
  'accent', 'accent-foreground', 'destructive', 'destructive-foreground', 'input', 'ring', 'rail',
  'primary', 'primary-hover', 'primary-active', 'primary-light', 'on-primary',
  'border', 'divider', 'focus-ring', 'overlay', 'on-error',
  ...['success', 'warning', 'error', 'info'].flatMap(name => [name, `${name}-bg`, `${name}-text`]),
];

const LEGACY_NAMES = new Set(LEGACY_COLOR_NAMES);
const COLOR_UTILITY = /^(?:bg|text|border(?:-[trblxyse])?|divide(?:-[xy])?|outline|ring(?:-offset)?|shadow|inset-shadow|inset-ring|decoration|placeholder|caret|accent|fill|stroke|from|via|to)-(.+)$/;
const OPACITY = /^(?:\d+(?:\.\d+)?|\[[^\]\s]+\]|\([^()\s]+\))$/;

/** Find the final utility after variants, without treating colons inside data/aria values as separators. */
function utilityStart(token) {
  let start = 0;
  let depth = 0;
  for (let index = 0; index < token.length; index++) {
    const character = token[index];
    if (character === '\\') { index++; continue; }
    if (character === '[' || character === '(') depth++;
    else if (character === ']' || character === ')') depth--;
    else if (character === ':' && depth === 0) start = index + 1;
  }
  return start;
}

/** A complete Tailwind token, including optional variants, opacity and important modifiers. */
export function isLegacyColorClass(token) {
  let utility = token.slice(utilityStart(token)).replace(/^!|!$/g, '');
  const slash = utility.indexOf('/');
  if (slash >= 0) {
    if (!OPACITY.test(utility.slice(slash + 1))) return false;
    utility = utility.slice(0, slash);
  }
  if (/^text-[a-z0-9]+(?:-[a-z0-9]+)*-role$/.test(utility)) return true;
  const match = COLOR_UTILITY.exec(utility);
  return !!match && (LEGACY_NAMES.has(match[1]) || /^(?:bg|text)-[a-z0-9]+(?:-[a-z0-9]+)*$/.test(match[1]));
}

/** @returns {{value: string, index: number}[]} Complete tokens only: `bg-primary-extra` is not `bg-primary`. */
export function findLegacyColorClasses(text) {
  const findings = [];
  for (const match of text.matchAll(/(?:\[[^\]\s]*\]|[^\s"'`<>{},;\[\]])+/g)) {
    if (isLegacyColorClass(match[0])) findings.push({ value: match[0], index: match.index });
  }
  return findings;
}

/** Raw custom-property names also catch getPropertyValue('--color-primary') and template prefixes. */
export function findComputedColorNames(text) {
  return Array.from(text.matchAll(/(?<![\w-])--color-[\w-]*/g), match => ({ value: match[0], index: match.index }));
}

/** CSS declarations may define public names, but consumers may not read them through var(). */
export function findComputedColorReferences(text) {
  return Array.from(text.matchAll(/[vV][aA][rR]\(\s*(--color-[\w-]+)/g), match => ({
    value: match[1], index: match.index + match[0].lastIndexOf(match[1]),
  }));
}
