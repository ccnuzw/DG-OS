import type { NativeObservedSkill } from '../../types/nativeExtensions';

/** Explicit Codex invocation requires a unique, enabled native path, not a display name. */
export function codexSkillCandidates<T extends NativeObservedSkill>(skills: T[]): T[] {
  return skills.filter(skill => skill.classification === 'skill' && skill.enabled === true && Boolean(skill.path) && skills.filter(other => other.name === skill.name && other.path === skill.path).length === 1);
}
