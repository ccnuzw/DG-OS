import { describe, expect, test } from 'bun:test';
import { commandsSchema, modelsSchema, mcpStatusSchema, nativeCapabilities, isUnsupportedControl, resultRequestId, validateQuestionAnswers, enrichActiveSkillSources, observeActiveSkillSources } from './native-protocol';
import { projectSupportedCommands } from './native-observation';

describe('Claude optional protocol boundaries', () => {
  const native = { supportedCommands() {}, mcpServerStatus() {}, reloadSkills() {}, reloadPlugins() {}, reconnectMcpServer() {}, toggleMcpServer() {} };
  test('unknown producers retain independent reads without speculative writes', () => {
    expect(nativeCapabilities(native, '2.1.999')).toEqual({ observe_skills: true, observe_mcp: true, reload_skills: false, reload_plugins: false, reconnect_mcp: false, toggle_mcp: false });
    expect(nativeCapabilities({ supportedCommands() {} }, '2.1.319')).toEqual({ observe_skills: true, observe_mcp: false, reload_skills: false, reload_plugins: false, reconnect_mcp: false, toggle_mcp: false });
    expect(nativeCapabilities(native, '2.1.319').toggle_mcp).toBe(true);
  });
  test('each read schema rejects malformed metadata independently', () => {
    expect(commandsSchema.safeParse([{ name: 'help', builtin: true }]).success).toBe(true);
    expect(commandsSchema.safeParse(undefined).success).toBe(false);
    expect(modelsSchema.safeParse([{ value: 'custom', displayName: 'Custom', description: '', supportsEffort: 'yes' }]).success).toBe(false);
    expect(mcpStatusSchema.safeParse([{ name: 'fixture', status: 'connected', tools: [{ name: 'ping' }] }]).success).toBe(true);
    expect(mcpStatusSchema.safeParse([{ name: 'fixture', status: 'connected', tools: 'ping' }]).success).toBe(false);
  });
  test('only the exact native dispatcher refusal establishes non-execution', () => {
    expect(isUnsupportedControl(new Error('Unsupported control request subtype'), 'mcp_status')).toBe(true);
    expect(isUnsupportedControl(new Error('Unsupported control request subtype: mcp_status'), 'mcp_status')).toBe(true);
    for (const message of ['Unsupported control request subtype: mcp_toggle', 'request timed out', 'unauthorized', 'Unsupported model', 'mcp_status is not available on this connection']) {
      expect(isUnsupportedControl(new Error(message), 'mcp_status')).toBe(false);
    }
  });
  test('real zero-model-turn local result closes its send while notification cannot invent a result', () => {
    const local = { type: 'result', local_command: 'help', subtype: 'success', is_error: false, num_turns: 0 };
    expect(resultRequestId(local, 'send', 'help', true)).toBe('send');
    expect(resultRequestId({ ...local, user_message_uuid: 'earlier' }, 'send', 'help', true)).toBeUndefined();
    expect(resultRequestId(local, 'send', 'usage', true)).toBeUndefined();
    expect(resultRequestId(local, 'send', 'help', false)).toBeUndefined();
    expect(resultRequestId({ type: 'result', user_message_uuids: ['other'], user_message_uuid: 'send' }, 'send', undefined, false)).toBe('send');
    expect(resultRequestId({ type: 'result' }, 'send', undefined, true)).toBeUndefined();
    expect(resultRequestId(local, undefined, 'help', true)).toBeUndefined();
  });
  test('question approval requires every answer and preserves multi-select/free text strings', () => {
    const input = { questions: [{ question: 'single' }, { question: 'multi', multiSelect: true }, { question: 'free' }] };
    expect(validateQuestionAnswers(input, { single: 'A', multi: 'A, B', free: 'Custom answer' })).toBe(true);
    expect(validateQuestionAnswers(input, { single: 'A' })).toBe(false);
    expect(validateQuestionAnswers(input, { single: 'A', multi: '', free: 'answer' })).toBe(false);
    expect(validateQuestionAnswers(input, { single: 'A', multi: 'B', free: 'answer', foreign: 'C' })).toBe(false);
    expect(validateQuestionAnswers({ questions: [{ question: 'same' }, { question: 'same' }] }, { same: 'A' })).toBe(false);
    expect(validateQuestionAnswers(input, undefined)).toBe(false);
  });
  test('active native source evidence accepts omitted builtin markers but rejects collisions', () => {
    const commands = [
      { name: 'apps/web:deploy', builtin: false }, { name: 'help', builtin: true },
      { name: 'legacy' }, { name: 'duplicate', builtin: false }, { name: 'duplicate', builtin: false },
      { name: 'source-duplicate', builtin: false }, { name: 'alias-collision', builtin: false },
      { name: 'other', builtin: false, aliases: ['alias-collision'] },
    ];
    const usage = { skills: { skillFrontmatter: [
      { name: 'apps/web:deploy', source: 'plugin', path: '/native/skills/deploy/SKILL.md', pluginName: 'fixture', pluginId: 'fixture@marketplace', tokens: 20, root: 'secret' },
      ...['help', 'legacy', 'duplicate', 'source-duplicate', 'source-duplicate', 'alias-collision'].map(name => ({ name, source: 'project', path: '/unproven' })),
    ] } };
    const result = enrichActiveSkillSources(commands, projectSupportedCommands(commands), usage);
    expect(result.incomplete).toBe(false);
    expect(result.source_incomplete).toBe(true);
    expect(result.skills[0].active_source).toEqual({ source: 'plugin', path: '/native/skills/deploy/SKILL.md', plugin_id: 'fixture@marketplace', plugin_name: 'fixture' });
    expect(result.skills[2].active_source?.source).toBe('project');
    for (const command of result.skills.filter((_, index) => index !== 0 && index !== 2)) expect(command.active_source).toBeUndefined();
    expect(JSON.stringify(result)).not.toContain('secret');
  });
  test('optional source metadata stays unknown without blocking commands', async () => {
    const commands = [{ name: 'fixture', builtin: false }];
    const projection = projectSupportedCommands(commands);
    for (const usage of [undefined, {}, { skills: { skillFrontmatter: 'invalid' } }, { skills: { skillFrontmatter: [{ name: 'fixture', source: 1 }] } }]) {
      const result = enrichActiveSkillSources(commands, projection, usage);
      expect(result.skills[0].active_source).toBeUndefined();
      expect(result.incomplete).toBe(false);
      expect(result.source_incomplete).toBe(true);
    }
    for (const native of [{}, { async getContextUsage() { throw new Error('Unsupported control request subtype: get_context_usage'); } }]) {
      expect((await observeActiveSkillSources(native, commands, projection)).skills[0].name).toBe('fixture');
    }
    const options: unknown[] = [];
    const native = { async getContextUsage(value: unknown) { options.push(value); return { skills: { skillFrontmatter: [{ name: 'fixture', source: 'project', path: '/bad\npath', pluginName: 'fixture' }] } }; } };
    const result = await observeActiveSkillSources(native, commands, projection);
    expect(options).toEqual([{ detail: 'summary' }]);
    expect(result.skills[0].active_source).toEqual({ source: 'project', plugin_name: 'fixture', plugin_id: undefined, path: undefined });
    expect(result.incomplete).toBe(false);
  });
  test('candidate snapshots retain hidden and shadowed rows without exposing native handles', async () => {
    const rows = [
      { name: 'same', source: 'user', state: 'on', advertised: true },
      { name: 'same', source: 'project', state: 'name-only', advertised: false },
      { name: 'hidden', source: 'project', state: 'off', advertised: false },
    ];
    const native = { async getSkillsDialog() { return { skills: rows.map(row => ({ ...row, handles: { secret: 'SENTINEL' }, description: 'SENTINEL' })) }; } };
    const result = await observeActiveSkillSources(native, [], { skills: [], incomplete: false });
    expect(result.skill_candidates).toEqual(rows);
    expect(result.skill_candidates_incomplete).toBe(false);
    expect(JSON.stringify(result)).not.toContain('SENTINEL');
    const malformed = await observeActiveSkillSources({ async getSkillsDialog() { return { skills: [...rows, { name: 'invalid', state: 'future' }] }; } }, [], { skills: [], incomplete: false });
    expect(malformed.skill_candidates).toEqual(rows);
    expect(malformed.skill_candidates_incomplete).toBe(true);
    const refused = await observeActiveSkillSources({ async getSkillsDialog() { throw Error('refused'); } }, [{ name: 'help', builtin: true }], projectSupportedCommands([{ name: 'help', builtin: true }]));
    expect(refused.skills[0].name).toBe('help');
    expect(refused.skill_candidates_incomplete).toBe(true);
  });
});
