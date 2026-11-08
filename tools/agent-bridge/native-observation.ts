import type { SlashCommand } from '@anthropic-ai/claude-agent-sdk';

// SDK 0.3.317 SlashCommand has no provenance fields beyond builtin. Keep the
// marker, never descriptions/aliases or guessed sources derived from names.
export function projectSupportedCommands(commands: readonly SlashCommand[]) {
  let incomplete = commands.length > 256;
  const skills = commands.slice(0, 256).map(command => {
    const name = typeof command.name === 'string' && command.name.length > 0 && command.name.length <= 128 && /^[a-zA-Z0-9_.:@\-]+$/.test(command.name) ? command.name : null;
    const builtin = typeof command.builtin === 'boolean' ? command.builtin : null;
    incomplete ||= name === null || command.builtin != null && builtin === null;
    return { name, builtin };
  });
  return { skills, incomplete };
}
