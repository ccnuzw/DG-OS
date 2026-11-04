import { afterEach, describe, expect, it, vi } from 'vitest';

const invoke = vi.hoisted(() => vi.fn());

import { formatMessage } from '../src/i18n/message';
vi.mock('@tauri-apps/api/core', () => ({ invoke }));

import {
  getLocale,
  getLanguagePreference,
  initLocale,
  resetLocaleForTesting,
  updateLanguage,
} from '../src/i18n';

afterEach(() => {
  invoke.mockReset();
  resetLocaleForTesting();
});

describe('host language runtime', () => {
  it('uses simplified Chinese when the host command is unavailable', async () => {
    invoke.mockRejectedValue(new Error('command not found'));

    await initLocale();

    expect(getLanguagePreference()).toBe('zh-CN');
    expect(getLocale()).toBe('zh-CN');
  });

  it('loads the host preference and resolved locale', async () => {
    invoke.mockResolvedValue({
      success: true,
      data: { preference: 'system', locale: 'en-US', catalog_version: 'v1' },
    });

    await initLocale();

    expect(getLanguagePreference()).toBe('system');
    expect(getLocale()).toBe('en-US');
  });

  it('applies a successful change and sends only the typed language field', async () => {
    invoke
      .mockResolvedValueOnce({ success: true, data: { preference: 'zh-CN', locale: 'zh-CN', catalog_version: 'v1' } })
      .mockResolvedValueOnce({ success: true, data: { preference: 'en-US', locale: 'en-US', catalog_version: 'v1' } });

    await initLocale();
    await updateLanguage('en-US');

    expect(invoke).toHaveBeenNthCalledWith(2, 'set_language_preferences', { language: 'en-US' });
    expect(getLanguagePreference()).toBe('en-US');
    expect(getLocale()).toBe('en-US');
  });

  it('rolls back the optimistic change when saving fails', async () => {
    invoke
      .mockResolvedValueOnce({ success: true, data: { preference: 'zh-CN', locale: 'zh-CN', catalog_version: 'v1' } })
      .mockResolvedValueOnce({ success: false, error: { code: 1000000 } });

    await initLocale();
    await expect(updateLanguage('en-US')).rejects.toMatchObject({ code: 1000000 });

    expect(getLanguagePreference()).toBe('zh-CN');
    expect(getLocale()).toBe('zh-CN');
  });
});

it('interpolates user values as literal text and diagnoses missing parameters',()=>{
  const catalog={'example':'Hello {name}'};
  expect(formatMessage(catalog,'example',{name:'<user>{name}'})).toBe('Hello <user>{name}');
  expect(formatMessage(catalog,'example')).toBe('⟦example⟧');
  expect(formatMessage(catalog,'missing')).toBe('⟦missing⟧');
});
