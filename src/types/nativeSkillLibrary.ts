import type { NativeDependencies, NativeInstance, NativeSelection, NativeSkill } from './nativeExtensions';

export interface NativeLibrarySkill {
  path: string; name: string; description: string; content_hash: string;
  file_count: number; script_count: number; executable_files: number;
  dynamic_commands: boolean; dependencies: NativeDependencies;
}
export interface NativeSkillSource {
  id: string; revision: number;
  repository: { owner: string; repo: string; requested_ref: string | null; skill_subpath: string | null; normalized_source: string };
  indexed_at: number | null; commit: string | null; error: string | null;
  skills: NativeLibrarySkill[]; rejected: [string, string][];
}
export interface NativeLibraryResult { source_id: string; repository_url: string; indexed_at: number; commit: string; stale: boolean; skill: NativeLibrarySkill }
export interface NativeLibrarySearch { sources: NativeSkillSource[]; results: NativeLibraryResult[] }
export interface NativePickedSkillSource { handle: string; label: string; expires_at: number; skills: NativeLibrarySkill[]; rejected: [string, string][] }
export interface NativeSkillRequest { selection: NativeSelection; slug: string; expected: NativeInstance }
export interface NativeSkillInspection { skill: NativeSkill; target_path: string }
export interface NativeSkillFile { instance: NativeInstance; path: string; hash: string; bytes: number; text: string | null; reason: 'sensitive' | 'too_large' | 'binary' | null }
export interface NativeDependencyReport {
  declarations: NativeDependencies;
  tools: { declaration: { program: string; version_requirement: string | null }; availability: 'FoundUnprobed' | 'Missing' | 'Unreadable' | 'Unknown'; candidate: string | null; executable: boolean }[];
  unknown: boolean;
}
export interface NativeVersionPlan { plan_id: string; executable: string; digest: string; argv: string[]; cwd: string; expires_at: number; deadline_seconds: number; environment: string; requirement: string | null }
export interface NativeVersionResult { status: string; output: string | null; exit_code: number | null; cleanup_confirmed: boolean; requirement_satisfied: boolean | null }
export interface NativePreparedSkillUpdate { handle: string; skill_path: string }
