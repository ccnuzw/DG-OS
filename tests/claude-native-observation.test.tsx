import { describe, expect, it } from 'vitest';
import { projectSupportedCommands } from '../tools/agent-bridge/native-observation';
import { commandsSchema } from '../tools/agent-bridge/native-protocol';

type Command = Parameters<typeof projectSupportedCommands>[0][number];
const command = (name: string, builtin?: boolean, aliases?: string[]): Command => ({ name, builtin, aliases });

describe('locked Claude supportedCommands observation boundary', () => {
  it('preserves builtin markers and aliases without guessing provenance from names', () => {
    const result = projectSupportedCommands([command('help', true, ['assist']), command('help'), command('mcp__fixture__tool', false), command('legacy-command'), command('skill-name')]);
    expect(result).toEqual({ skills: [{ name: 'help', builtin: true, aliases: ['assist'] }, { name: 'help', builtin: null, aliases: [] }, { name: 'mcp__fixture__tool', builtin: false, aliases: [] }, { name: 'legacy-command', builtin: null, aliases: [] }, { name: 'skill-name', builtin: null, aliases: [] }], incomplete: false });
  });
  it('does not copy descriptions or invented source, path and ownership fields across private IPC', () => {
    const input = { ...command('fixture'), description: 'secret-description', argumentHint: 'secret-argument', source: 'user', path: '/native/fixture/SKILL.md', pluginId: 'fixture@market', instance: { id: 'untrusted' } };
    expect(projectSupportedCommands([input]).skills[0]).toEqual({ name: 'fixture', builtin: null, aliases: [] });
  });
  it('accepts native nested command tokens independently of file permission rules', () => {
    expect(projectSupportedCommands([command('apps/web:deploy', false, ['web/deploy'])])).toEqual({ skills: [{ name: 'apps/web:deploy', builtin: false, aliases: ['web/deploy'] }], incomplete: false });
  });
  it('retains completeness flags for malformed fields and the bounded catalog', () => {
    const invalid = { ...command('fixture'), builtin: 'true' } as unknown as Command;
    expect(projectSupportedCommands([invalid, command('/not-an-identifier')])).toEqual({ skills: [{ name: 'fixture', builtin: null, aliases: [] }, { name: null, builtin: null, aliases: [] }], incomplete: true });
    const result = projectSupportedCommands(Array.from({ length: 257 }, () => command('fixture', true)));
    expect(result.skills).toHaveLength(256);
    expect(result.incomplete).toBe(true);
    expect(projectSupportedCommands([])).toEqual({ skills: [], incomplete: false });
  });
  it('bounds and validates aliases without silently authorizing a partial catalog', () => {
    const aliases = ['review', 'review', '/leading-slash', 'space name', 'line\nfeed', 'trailing\n', 'x'.repeat(129)];
    expect(projectSupportedCommands([command('fixture', false, aliases)])).toEqual({ skills: [{ name: 'fixture', builtin: false, aliases: ['review'] }], incomplete: true });
    expect(projectSupportedCommands([command('fixture', false, Array.from({ length: 65 }, (_, index) => `alias-${index}`))]).incomplete).toBe(true);
    expect(projectSupportedCommands([{ ...command('fixture'), aliases: 'invalid' } as unknown as Command]).incomplete).toBe(true);
  });
  it('retains alias metadata through the actual bridge read schema', () => {
    const parsed = commandsSchema.parse([{ name: 'apps/web:deploy', aliases: ['deploy'], builtin: false }]);
    expect(projectSupportedCommands(parsed).skills[0].aliases).toEqual(['deploy']);
    expect(commandsSchema.safeParse([{ name: 'fixture', aliases: 'invalid' }]).success).toBe(false);
  });
});
