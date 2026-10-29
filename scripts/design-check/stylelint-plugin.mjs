// R3 (D068): every `var(--x)` must resolve to a custom property defined in the token files.
//
// Stylelint's built-in `no-unknown-custom-properties` is not enough here, verified against 17.16.0:
//  - it counts a declaration anywhere in the linted file as a definition, so a page that defines its own
//    `--mcp-ink` passes, which defeats the point of forcing the private namespaces to migrate;
//  - it skips every `var(--x, fallback)`, so `var(--gone, 4px)` passes.
// "Defined" therefore means: declared (or `@property`-registered) in the config's `referenceFiles`
// (`DEFINITION_GLOBS` in scope.mjs), or owned by a runtime package (`runtimePrefixes`).
import stylelint from 'stylelint';

const { createPlugin, utils } = stylelint;

export const ruleName = 'dgos/defined-custom-properties';

const messages = utils.ruleMessages(ruleName, {
  rejected: (name) => `Custom property "${name}" is not defined in the design tokens`,
});

const VAR_REFERENCE = /var\(\s*(--[\w-]+)/g;

const rule = (primary, secondary) => (root, result) => {
  const valid = utils.validateOptions(
    result,
    ruleName,
    { actual: primary, possible: [true] },
    {
      actual: secondary,
      possible: { runtimePrefixes: (value) => typeof value === 'string' },
      optional: true,
    },
  );
  if (!valid) return;

  const runtimePrefixes = secondary?.runtimePrefixes ?? [];
  const defined = new Set();
  for (const reference of result.stylelint.referenceRoots) {
    reference.walkDecls(/^--/, (decl) => defined.add(decl.prop));
    reference.walkAtRules('property', (atRule) => defined.add(atRule.params.trim()));
  }

  root.walkDecls((decl) => {
    const valueStart = decl.prop.length + (decl.raws.between ?? ':').length;
    for (const match of decl.value.matchAll(VAR_REFERENCE)) {
      const name = match[1];
      if (defined.has(name) || runtimePrefixes.some((prefix) => name.startsWith(prefix))) continue;
      const index = valueStart + match.index + match[0].lastIndexOf(name);
      utils.report({
        result,
        ruleName,
        message: messages.rejected(name),
        node: decl,
        index,
        endIndex: index + name.length,
      });
    }
  });
};

rule.ruleName = ruleName;
rule.messages = messages;

export default createPlugin(ruleName, rule);
