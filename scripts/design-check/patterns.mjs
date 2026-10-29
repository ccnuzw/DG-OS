// Regex sources shared by the ESLint selectors (eslint.config.mjs) and the HTML scanner.
// Sources must not contain "/" because esquery's attribute-regex syntax ends at the first slash.

/** CSS named colours except `transparent` and `currentcolor`, which are allowed (R2). */
export const NAMED_COLORS = [
  'aliceblue', 'antiquewhite', 'aqua', 'aquamarine', 'azure', 'beige', 'bisque', 'black', 'blanchedalmond', 'blue',
  'blueviolet', 'brown', 'burlywood', 'cadetblue', 'chartreuse', 'chocolate', 'coral', 'cornflowerblue', 'cornsilk',
  'crimson', 'cyan', 'darkblue', 'darkcyan', 'darkgoldenrod', 'darkgray', 'darkgreen', 'darkgrey', 'darkkhaki',
  'darkmagenta', 'darkolivegreen', 'darkorange', 'darkorchid', 'darkred', 'darksalmon', 'darkseagreen',
  'darkslateblue', 'darkslategray', 'darkslategrey', 'darkturquoise', 'darkviolet', 'deeppink', 'deepskyblue',
  'dimgray', 'dimgrey', 'dodgerblue', 'firebrick', 'floralwhite', 'forestgreen', 'fuchsia', 'gainsboro',
  'ghostwhite', 'gold', 'goldenrod', 'gray', 'green', 'greenyellow', 'grey', 'honeydew', 'hotpink', 'indianred',
  'indigo', 'ivory', 'khaki', 'lavender', 'lavenderblush', 'lawngreen', 'lemonchiffon', 'lightblue', 'lightcoral',
  'lightcyan', 'lightgoldenrodyellow', 'lightgray', 'lightgreen', 'lightgrey', 'lightpink', 'lightsalmon',
  'lightseagreen', 'lightskyblue', 'lightslategray', 'lightslategrey', 'lightsteelblue', 'lightyellow', 'lime',
  'limegreen', 'linen', 'magenta', 'maroon', 'mediumaquamarine', 'mediumblue', 'mediumorchid', 'mediumpurple',
  'mediumseagreen', 'mediumslateblue', 'mediumspringgreen', 'mediumturquoise', 'mediumvioletred', 'midnightblue',
  'mintcream', 'mistyrose', 'moccasin', 'navajowhite', 'navy', 'oldlace', 'olive', 'olivedrab', 'orange',
  'orangered', 'orchid', 'palegoldenrod', 'palegreen', 'paleturquoise', 'palevioletred', 'papayawhip', 'peachpuff',
  'peru', 'pink', 'plum', 'powderblue', 'purple', 'rebeccapurple', 'red', 'rosybrown', 'royalblue', 'saddlebrown',
  'salmon', 'sandybrown', 'seagreen', 'seashell', 'sienna', 'silver', 'skyblue', 'slateblue', 'slategray',
  'slategrey', 'snow', 'springgreen', 'steelblue', 'tan', 'teal', 'thistle', 'tomato', 'turquoise', 'violet',
  'wheat', 'white', 'whitesmoke', 'yellow', 'yellowgreen',
];

const NAMES = NAMED_COLORS.join('|');

/** `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`; not an HTML entity (`&#123;`) and not a longer word. */
export const HEX_COLOR = String.raw`(?<![\w&])#(?:[0-9a-fA-F]{8}|[0-9a-fA-F]{6}|[0-9a-fA-F]{3,4})(?![\w-])`;

/** Colour functions that carry literal channels. `color-mix()` and `var()` are fine. */
export const COLOR_FUNCTION = String.raw`(?<![\w-])(?:rgba?|hsla?|hwb|lab|lch|oklab|oklch|color)\(`;

/** A whole string that is one colour keyword (used only where a colour is expected). */
export const COLOR_KEYWORD_WHOLE = `^(?:${NAMES})$`;

/** CSS-looking text with a colour keyword: `border: 1px solid red`, `fill: white`. */
export const COLOR_KEYWORD_DECLARATION = String.raw`(?:color|background|border|outline|fill|stroke|shadow)[\w-]*\s*:[^;'"]*\b(?:${NAMES})\b`;

/** `var(--x, red)`: a literal fallback defeats the token. */
export const COLOR_KEYWORD_FALLBACK = String.raw`var\(\s*--[\w-]+\s*,\s*(?:${NAMES})\b`;

/** Property or attribute names whose string value is a colour. */
export const COLOR_KEYS = String.raw`^(?:color|backgroundColor|background|borderColor|outlineColor|fill|stroke|stopColor|floodColor|lightingColor|caretColor|accentColor)$`;

/**
 * Tailwind arbitrary value on a utility: `bg-[#fff]`, `p-[13px]`, `w-[--radix-x]`.
 * `data-[..]:` and `aria-[..]:` are variants, not values: they are always followed by `:`, so `(?!:)` skips them
 * (`group-data-[x=y]:p-2` is therefore allowed, while the `p-[3px]` after it is still caught).
 * The `/50` opacity modifier and a trailing `!` may follow.
 */
export const TAILWIND_ARBITRARY_VALUE = String.raw`(?<![\w\[\]-])[a-z][a-z0-9]*(?:-[a-z0-9]+)*-\[[^\]\s]+\](?![:\]])`;

/** Arbitrary variants (`min-[400px]:`, `supports-[display:grid]:`); `data-[..]:` and `aria-[..]:` stay allowed. */
export const TAILWIND_ARBITRARY_VARIANT = String.raw`(?<![\w\[\]-])(?!(?:[a-z-]+-)?(?:data|aria)-\[)[a-z][a-z0-9]*(?:-[a-z0-9]+)*-\[[^\]\s]+\]:`;

/** Bracket-first tokens: `[&>svg]:size-4`, `[--x:1px]`, `[mask-type:luminance]`. Requires `&`, `@`, `:` or `--` inside. */
export const TAILWIND_BRACKET_TOKEN = String.raw`(?<![\w\[\]-])\[(?=[^\]\s]*(?:&|@|:|--))[^\]\s]+\]`;

/** An inline `style="..."` attribute inside an HTML string. */
export const HTML_STYLE_ATTRIBUTE = String.raw`<[a-zA-Z][^>]*\sstyle\s*=`;

/**
 * R5 (AC24): a class string that removes the outline (`outline-none`, also `focus:outline-none`) without any
 * `focus-visible:` utility in the same string. `outline-hidden` is not matched: Tailwind keeps a forced-colors
 * outline for it. A cva() call that splits the pair across two strings is reported too; keep the pair together.
 */
export const OUTLINE_WITHOUT_FOCUS_VISIBLE = String.raw`^(?![\s\S]*focus-visible:)[\s\S]*(?<![\w-])outline-none(?![\w-])`;
