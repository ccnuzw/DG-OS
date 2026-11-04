export default {
 'generation.reasoningBudget': 'Reasoning token budget (compatible profiles only)', 'generation.effortMedium': 'Medium',
 'generation.reasoningCapability': 'Model reasoning capability: {state}',
 'generation.supported': 'Supported', 'generation.unsupported': 'Unsupported', 'generation.unknown': 'Unknown',
 'generation.length': 'Response length', 'generation.short': 'Short · 1,024 tokens', 'generation.medium': 'Medium · 4,096 tokens', 'generation.long': 'Long · 8,192 tokens',
 'generation.temperature': 'Temperature (empty uses provider default)', 'generation.reasoning': 'Reasoning effort', 'generation.default': 'Provider default', 'generation.low': 'Low', 'generation.high': 'High',
 'generation.instruction': 'Custom guidance', 'generation.capabilityNotice': 'Unsupported controls are rejected by the provider profile; guidance cannot grant permissions.', 'generation.outputLimit': 'Known output limit: {count} tokens.', 'generation.unknownLimit': 'Output limit unknown.',
 'generation.moveUp': 'Move up', 'generation.moveDown': 'Move down', 'generation.combinationNotice': 'Skills are sent in this order with pinned versions. Natural-language conflicts are not resolved automatically; later guidance does not guarantee precedence.'
} as const;
