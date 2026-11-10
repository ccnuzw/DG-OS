import type { SlashCommand } from '@anthropic-ai/claude-agent-sdk';

// Command identifiers are native dispatch tokens, never paths or file authorization.
function validCommandName(value: unknown): value is string {
  return typeof value === 'string' && value.length > 0 && value.length <= 128 && value[0] !== '/' && !/[^a-zA-Z0-9_.:@/\-]/.test(value);
}

// SDK aliases are dispatch metadata; they carry no provenance or resource authority.
export function projectSupportedCommands(commands: readonly Pick<SlashCommand, 'name' | 'builtin' | 'aliases'>[]) {
  let incomplete = commands.length > 256;
  const skills = commands.slice(0, 256).map(command => {
    const name = validCommandName(command.name) ? command.name : null;
    const builtin = typeof command.builtin === 'boolean' ? command.builtin : null;
    incomplete ||= name === null || command.builtin != null && builtin === null;
    const rawAliases: unknown = command.aliases ?? [];
    const aliases = Array.isArray(rawAliases) ? [...new Set(rawAliases.slice(0, 64).filter(validCommandName))] : [];
    incomplete ||= !Array.isArray(rawAliases) || rawAliases.length > 64 || rawAliases.some(alias => !validCommandName(alias));
    return { name, builtin, aliases };
  });
  return { skills, incomplete };
}
