import { findComputedColorNames, findLegacyColorClasses } from './semantic-colors.mjs';

/** Checks strings, not arbitrary identifiers: Button variant="primary" and status="error" remain valid. */
export const semanticColorsRule = {
  meta: {
    type: 'problem',
    schema: [{ type: 'object', properties: { allowComputed: { type: 'boolean' } }, additionalProperties: false }],
    messages: {
      legacy: 'R2 第一方颜色必须使用语义角色；旧颜色工具类 "{{value}}" 仅保留为公开兼容契约。',
      computed: 'R2 第一方禁止直接使用计算颜色 "{{value}}"；改用 --surface-*、--text-*、--accent-*、--border-* 或 --status-*。',
    },
  },
  create(context) {
    const allowComputed = context.options[0]?.allowComputed ?? false;
    function inspect(node, text) {
      for (const { value } of findLegacyColorClasses(text)) context.report({ node, messageId: 'legacy', data: { value } });
      if (!allowComputed) {
        for (const { value } of findComputedColorNames(text)) context.report({ node, messageId: 'computed', data: { value } });
      }
    }
    return {
      Literal(node) { if (typeof node.value === 'string') inspect(node, node.value); },
      TemplateElement(node) { inspect(node, node.value.cooked ?? node.value.raw); },
    };
  },
};
