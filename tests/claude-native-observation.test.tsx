import { describe, expect, it } from 'vitest';
import { projectSupportedCommands } from '../tools/agent-bridge/native-observation';

type Command = Parameters<typeof projectSupportedCommands>[0][number];
const command = (name: string, builtin?: boolean): Command => ({ name, builtin, description: 'secret-description', argumentHint: 'secret-argument', aliases: ['secret-alias'] });

describe('locked Claude supportedCommands observation boundary', () => {
  it('preserves actual builtin markers without guessing provenance from names', () => {
    const result = projectSupportedCommands([command('help', true), command('help'), command('mcp__fixture__tool', false), command('legacy-command'), command('skill-name')]);
    expect(result).toEqual({ skills: [{ name: 'help', builtin: true }, { name: 'help', builtin: null }, { name: 'mcp__fixture__tool', builtin: false }, { name: 'legacy-command', builtin: null }, { name: 'skill-name', builtin: null }], incomplete: false });
    expect(JSON.stringify(result)).not.toContain('secret-');
  });
  it('does not copy invented source, path or ownership fields across private IPC', () => {
    const input = { ...command('fixture'), source: 'user', path: '/native/fixture/SKILL.md', pluginId: 'fixture@market', instance: { id: 'untrusted' } };
    expect(projectSupportedCommands([input]).skills[0]).toEqual({ name: 'fixture', builtin: null });
  });
  it('retains completeness flags for malformed fields and the bounded catalog', () => {
    const invalid = { ...command('fixture'), builtin: 'true' } as unknown as Command;
    expect(projectSupportedCommands([invalid, command('/not-an-identifier')])).toEqual({ skills: [{ name: 'fixture', builtin: null }, { name: null, builtin: null }], incomplete: true });
    const result = projectSupportedCommands(Array.from({ length: 257 }, () => command('fixture', true)));
    expect(result.skills).toHaveLength(256);
    expect(result.incomplete).toBe(true);
    expect(projectSupportedCommands([])).toEqual({ skills: [], incomplete: false });
  });
});
