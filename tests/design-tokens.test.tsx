// @vitest-environment node
import { describe, expect, it } from 'vitest';
import { PRESET_IDS, resolveTheme, contrastRatio } from '../src/ui/theme/kernel';
import pairsFile from './fixtures/design-tokens/contrast-pairs.json';

describe('five preset semantic contrast',()=>{
  for(const preset of PRESET_IDS)for(const mode of ['light','dark'] as const){
    it(`${preset}/${mode} meets every text and control contrast pair`,()=>{
      const colors=resolveTheme(preset,mode);
      for(const pair of pairsFile.pairs){
        expect(contrastRatio(colors[`--color-${pair.fg}`],colors[`--color-${pair.bg}`]),`${pair.fg} on ${pair.bg}`).toBeGreaterThanOrEqual(pair.kind==='text'?4.5:3);
      }
    });
  }
  it('inherits interface changes into reading and code until explicitly overridden',()=>{
    const colors=resolveTheme('studio','light',{interface:{background:'#ffffff'},reading:{background:'#ffffee',foreground:'#222222'}});
    expect(colors['--surface-code']).toBe('#ffffff');
    expect(colors['--surface-reading']).toBe('#ffffee');
    expect(colors['--text-reading']).toBe('#222222');
  });
  it('rejects illegible text and CSS injection in overrides',()=>{
    expect(()=>resolveTheme('studio','light',{interface:{foreground:'#ffffff'}})).toThrow();
    expect(()=>resolveTheme('studio','light',{code:{background:'#ffffff;url(x)'}})).toThrow();
  });
});
