// Style-compliance rules R2 (colours), R3 (defined variables) and R4 (px report) for CSS, D068.
// Paths are relative to the scanned root (the runner passes it as `configBasedir`).
import definedCustomProperties from './scripts/design-check/stylelint-plugin.mjs';
import { DEFINITION_GLOBS, RUNTIME_VARIABLE_PREFIXES, TOKEN_GLOBS } from './scripts/design-check/scope.mjs';

/** Tailwind 4 directives that appear in this repo's stylesheets or that components may use. */
const TAILWIND_AT_RULES = ['theme', 'source', 'apply', 'utility', 'variant', 'custom-variant', 'plugin', 'config', 'reference'];

const COLOR_RULES = {
  'color-no-hex': true,
  'color-named': 'never',
  // `color-mix()` and `var()` stay allowed: they carry tokens, not channels.
  'function-disallowed-list': ['rgb', 'rgba', 'hsl', 'hsla', 'hwb', 'lab', 'lch', 'oklab', 'oklch', 'color'],
};

// R4: px in type size, padding, margin and gap. Reported, never blocks (warning severity).
const PX_REPORT = [{ '/^(font-size|padding|margin|gap|row-gap|column-gap)(-.+)?$/': ['px'] }, { severity: 'warning' }];

export default {
  plugins: [definedCustomProperties],
  referenceFiles: DEFINITION_GLOBS,
  rules: {
    ...COLOR_RULES,
    'dgos/defined-custom-properties': [true, { runtimePrefixes: RUNTIME_VARIABLE_PREFIXES }],
    'at-rule-no-unknown': [true, { ignoreAtRules: TAILWIND_AT_RULES }],
    'declaration-property-unit-disallowed-list': PX_REPORT,
  },
  overrides: [
    {
      // AC12 exception: the token files define the colours and the px scale.
      files: TOKEN_GLOBS,
      rules: {
        'color-no-hex': null,
        'color-named': null,
        'function-disallowed-list': null,
        'declaration-property-unit-disallowed-list': null,
      },
    },
  ],
};
