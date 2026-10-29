// Single source of truth for what `npm run design:check` scans (FR-005 §9.1, D068).
// Used by eslint.config.mjs, stylelint.config.mjs and scripts/design-check.mjs so they cannot drift apart.

/** First-party script sources (ESLint). `bundled-modules/**` is limited to the shipped entry files. */
export const SCRIPT_GLOBS = ['src/**/*.{ts,tsx,js,jsx,mjs}', 'bundled-modules/**/entry.js'];

/** Stylesheets (Stylelint). */
export const STYLE_GLOBS = ['src/**/*.css'];

/** HTML documents (scanned by scripts/design-check/html-scan.mjs). */
export const HTML_FILES = ['index.html'];

/** Never scanned. `tests/` holds the violating fixtures on purpose. */
export const IGNORE_GLOBS = ['**/node_modules/**', 'dist/**', 'tests/**', 'skills/**', 'target/**', 'src-tauri/**'];

/** Token definitions may use literal colours and px (AC12 exception). */
export const TOKEN_GLOBS = ['src/ui/tokens/**'];

/**
 * Files whose custom properties count as "defined" for R3. A variable that is only declared in a page
 * stylesheet is deliberately NOT defined: private per-page namespaces must migrate to tokens (D068).
 */
export const DEFINITION_GLOBS = ['src/ui/tokens/**/*.css', 'src/ui/tailwind.css'];

/** Runtime variables owned by a third-party package; CSS may read them but cannot define them. */
export const RUNTIME_VARIABLE_PREFIXES = ['--radix-'];

/** Baseline files, relative to the scanned root. */
export const BASELINE_DIR = 'scripts/design-check/baseline';
export const ESLINT_BASELINE = `${BASELINE_DIR}/eslint-suppressions.json`;
export const STYLE_BASELINE = `${BASELINE_DIR}/legacy-style.json`;
