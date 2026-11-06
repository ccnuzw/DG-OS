import stylelint from 'stylelint';
import { findComputedColorReferences, findLegacyColorClasses, isLegacyColorClass } from './semantic-colors.mjs';

const { createPlugin, utils } = stylelint;
export const ruleName = 'dgos/semantic-colors';
const messages = utils.ruleMessages(ruleName, {
  legacy: name => `R2 First-party colour utility "${name}" must use a semantic role`,
  computed: name => `R2 Computed colour "${name}" may only be read by the colour definition layer`,
  override: name => `R2 Theme property "${name}" may only be defined by tokens or the Tailwind mapping`,
});
const COLOR_ROLE = /^--(?:color-|surface-|text-(?:primary|secondary|tertiary|disabled|reading|content|code)$|accent-|border-|status-|overlay-)/;

const rule = (primary, secondary) => (root, result) => {
  if (!utils.validateOptions(result, ruleName,
    { actual: primary, possible: [true] },
    { actual: secondary, possible: { allowComputed: [true, false], allowDefinitions: [true, false] }, optional: true },
  )) return;
  const { allowComputed = false, allowDefinitions = false } = secondary ?? {};
  // Reference definitions are the same authority as R3; private local declarations do not grant ownership.
  const defined = new Set();
  for (const reference of result.stylelint.referenceRoots ?? []) {
    reference.walkDecls(/^--/, decl => defined.add(decl.prop));
    reference.walkAtRules('property', atRule => defined.add(atRule.params.trim()));
  }
  function report(node, name, index, message) {
    utils.report({ result, ruleName, node, message: message(name), index, endIndex: index + name.length });
  }
  function checkDefinition(node, name, index) {
    if (!allowDefinitions && (defined.has(name) || COLOR_ROLE.test(name))) report(node, name, index, messages.override);
  }
  function checkReferences(node, text, start) {
    if (allowComputed) return;
    for (const match of findComputedColorReferences(text)) report(node, match.value, start + match.index, messages.computed);
  }
  root.walkDecls(decl => {
    checkDefinition(decl, decl.prop, 0);
    checkReferences(decl, decl.value, decl.prop.length + (decl.raws.between ?? ':').length);
  });
  root.walkAtRules(atRule => {
    const start = 1 + atRule.name.length + (atRule.raws.afterName ?? ' ').length;
    if (atRule.name === 'property') checkDefinition(atRule, atRule.params.trim(), start + atRule.params.indexOf(atRule.params.trim()));
    checkReferences(atRule, atRule.params, start);
    if (atRule.name === 'apply') {
      for (const match of findLegacyColorClasses(atRule.params)) report(atRule, match.value, start + match.index, messages.legacy);
    }
  });
  root.walkRules(cssRule => {
    // Tailwind escapes punctuation in generated selectors; decode it before checking a complete class token.
    for (const match of cssRule.selector.matchAll(/\.((?:\\[^\r\n]|[\w-])+)/g)) {
      const name = match[1].replace(/\\([^\r\n])/g, '$1');
      if (isLegacyColorClass(name)) report(cssRule, match[1], match.index + 1, messages.legacy);
    }
  });
};
rule.ruleName = ruleName;
rule.messages = messages;
export default createPlugin(ruleName, rule);
