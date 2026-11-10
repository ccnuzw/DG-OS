import type { NativeObservedSkill } from '../../types/nativeExtensions';

// SDK names take precedence over aliases. Builtin precedence applies only to
// actual names; an alias shared by multiple commands has no verified target.
export function commandReason(commands: NativeObservedSkill[], command: NativeObservedSkill, name: string) {
  const exact = commands.filter(row => row.name === name);
  if (exact.some(row => row.builtin === true || row.classification === 'builtin_command')) return 'agentSkills.commands.builtin';
  const matches = exact.length ? exact : commands.filter(row => row.aliases?.includes(name));
  if (matches.length !== 1 || matches[0] !== command) return 'agentSkills.commands.ambiguous';
  if (command.builtin === true || command.classification === 'builtin_command') return 'agentSkills.commands.builtin';
  if (command.enabled === false) return 'agentSkills.commands.disabled';
  return null;
}
