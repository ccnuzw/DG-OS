import { z } from 'zod';

// SDK 0.3.317 private-stdio controls were checked against the installed CLI
// 2.1.319 handlers. Read schemas do not authorize mutations on other producers.
export const verifiedWriteVersion = '2.1.319';
export function verifiedWrites(version: unknown): boolean { return version === verifiedWriteVersion; }
export const commandMetadata = z.object({ name: z.string(), description: z.string().optional(), argumentHint: z.string().optional(), builtin: z.boolean().optional(), aliases: z.array(z.string()).optional() });
export const commandsSchema = z.array(commandMetadata);

const sourceText = (limit: number) => z.string().min(1).max(limit).refine(value => !/[\u0000-\u001f\u007f-\u009f]/.test(value));
export const activeSkillSourceSchema = z.object({
  name: z.string(), source: sourceText(128),
  path: sourceText(2048).optional().catch(undefined),
  pluginId: sourceText(256).optional().catch(undefined),
  pluginName: sourceText(256).optional().catch(undefined),
});
const activeSkillsUsageSchema = z.object({ skills: z.object({ skillFrontmatter: z.array(z.unknown()) }) });
type SourceCommand = { name: string; builtin?: boolean; aliases?: readonly string[] };

// Context frontmatter covers active skills only, never hidden/overridden instances.
// Provenance is display evidence: no path here grants resource/file authority.
export function enrichActiveSkillSources<T extends { name: string | null; builtin: boolean | null }>(
  commands: readonly SourceCommand[], projection: { skills: T[]; incomplete: boolean }, usage: unknown,
) {
  const parsed = activeSkillsUsageSchema.safeParse(usage);
  const commandNames = new Map<string, number>();
  for (const command of commands) {
    for (const name of new Set([command.name, ...(command.aliases ?? [])])) commandNames.set(name, (commandNames.get(name) ?? 0) + 1);
  }
  const rows = parsed.success ? parsed.data.skills.skillFrontmatter : [];
  const sourceNames = new Map<string, number>();
  for (const row of rows) {
    const name = typeof row === 'object' && row !== null && 'name' in row ? row.name : undefined;
    if (typeof name === 'string') sourceNames.set(name, (sourceNames.get(name) ?? 0) + 1);
  }
  const sources = new Map<string, z.infer<typeof activeSkillSourceSchema>>();
  for (const row of rows) {
    const source = activeSkillSourceSchema.safeParse(row);
    if (source.success && sourceNames.get(source.data.name) === 1) sources.set(source.data.name, source.data);
  }
  return { ...projection, source_incomplete: true, skills: projection.skills.map(command => {
    const source = command.name !== null && command.builtin !== true && commandNames.get(command.name) === 1 ? sources.get(command.name) : undefined;
    return { ...command, ...(source ? { active_source: { source: source.source, path: source.path, plugin_id: source.pluginId, plugin_name: source.pluginName } } : {}) };
  }) };
}

const skillCandidateSchema = z.object({
  name: sourceText(128).refine(value => !value.startsWith('/') && /^[A-Za-z0-9_\-.:@/]+$/.test(value)),
  source: sourceText(128), state: z.enum(['on', 'name-only', 'user-invocable-only', 'off']), advertised: z.boolean(),
});
function projectSkillCandidates(dialog: unknown) {
  const parsed = z.object({ skills: z.array(z.unknown()) }).safeParse(dialog);
  let incomplete = !parsed.success || parsed.data.skills.length > 256;
  const candidates: z.infer<typeof skillCandidateSchema>[] = [];
  if (parsed.success) for (const row of parsed.data.skills.slice(0, 256)) {
    const candidate = skillCandidateSchema.safeParse(row);
    if (candidate.success) candidates.push(candidate.data);
    else incomplete = true;
  }
  return { skill_candidates: candidates, skill_candidates_incomplete: incomplete };
}

export async function observeActiveSkillSources<T extends { name: string | null; builtin: boolean | null }>(
  native: object, commands: readonly SourceCommand[], projection: { skills: T[]; incomplete: boolean },
) {
  let usage: unknown, dialog: unknown;
  try {
    if ('getContextUsage' in native && typeof native.getContextUsage === 'function') {
      usage = await native.getContextUsage({ detail: 'summary' });
    }
  } catch {
    // Optional metadata refusal/failure must not disable the supportedCommands read.
  }
  try {
    if ('getSkillsDialog' in native && typeof native.getSkillsDialog === 'function') dialog = await native.getSkillsDialog();
  } catch {
    // Optional candidate refusal does not erase active commands or their sources.
  }
  return { ...enrichActiveSkillSources(commands, projection, usage), ...projectSkillCandidates(dialog) };
}
export const modelsSchema = z.array(z.object({
  value: z.string(), displayName: z.string(), description: z.string(), resolvedModel: z.string().optional(),
  supportsEffort: z.boolean().optional(), supportedEffortLevels: z.array(z.enum(['low', 'medium', 'high', 'xhigh', 'max'])).optional(),
}));
export const mcpStatusSchema = z.array(z.object({
  name: z.string(), status: z.string(), source: z.string().optional(), scope: z.string().optional(),
  tools: z.array(z.object({ name: z.string() })).optional(),
}));
export type NativeCapabilities = {
  observe_skills: boolean; observe_mcp: boolean; reload_skills: boolean;
  reload_plugins: boolean; reconnect_mcp: boolean; toggle_mcp: boolean;
};
export function nativeCapabilities(native: object, version: unknown): NativeCapabilities {
  const has = (name: string) => typeof (native as Record<string, unknown>)[name] === 'function';
  const write = verifiedWrites(version);
  return { observe_skills: has('supportedCommands'), observe_mcp: has('mcpServerStatus'),
    reload_skills: write && has('reloadSkills'), reload_plugins: write && has('reloadPlugins'),
    reconnect_mcp: write && has('reconnectMcpServer'), toggle_mcp: write && has('toggleMcpServer') };
}
// The CLI dispatcher's exact default error, optionally followed by its subtype.
// Auth, policy, transport and malformed-response errors are never Unsupported.
export function isUnsupportedControl(error: unknown, subtype: string): boolean {
  return error instanceof Error && (error.message === 'Unsupported control request subtype'
    || error.message === `Unsupported control request subtype: ${subtype}`);
}

// A local command still emits a real success/error result (SDKResultSuccess has
// local_command; the CLI's !shouldQuery branch sets num_turns=0). Its output
// notification alone must never complete a turn. Older producers omit UUID
// echoes: only the verified local-command result may use our single-flight ID.
export function resultRequestId(message: {
  type: string; local_command?: string; user_message_uuid?: string; user_message_uuids?: string[];
}, active: string | undefined, localCommand: string | undefined, writeProtocolVerified: boolean): string | undefined {
  if (!active) return undefined;
  const echoed = [...(message.user_message_uuids ?? []), ...(message.user_message_uuid ? [message.user_message_uuid] : [])];
  if (echoed.length) return echoed.includes(active) ? active : undefined;
  if (message.type !== 'result') return active;
  return writeProtocolVerified && localCommand !== undefined && message.local_command === localCommand ? active : undefined;
}

export function validateQuestionAnswers(input: Record<string, unknown>, answers: Record<string, string> | undefined): boolean {
  const questions = z.array(z.object({ question: z.string() }).passthrough()).min(1).safeParse(input.questions);
  return questions.success && answers !== undefined
    && new Set(questions.data.map(question => question.question)).size === questions.data.length
    && Object.keys(answers).length === questions.data.length
    && questions.data.every(question => typeof answers[question.question] === 'string' && answers[question.question].trim().length > 0);
}
