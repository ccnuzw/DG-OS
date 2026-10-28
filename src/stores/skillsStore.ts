import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';

export interface Skill {
  id: string;
  skill_id: string;
  name: string;
  description?: string;
  version: string;
  author?: string;
  install_path: string;
  call_type: 'http' | 'cli' | 'content';
  enabled: boolean;
  config?: any;
  manifest: SkillManifest;
  installed_at: number;
  updated_at: number;
  user_id: string;
}

export interface SkillInfo {
  skill_id: string;
  name: string;
  description?: string;
  latest_version: string;
  author?: string;
  category: string;
  tags: string[];
  identity: 'dgos_official' | 'dgos_curated' | 'third_party' | 'legacy_registry' | string;
  source_kind: string;
  license_spdx?: string;
  compatible_apps: string[];
}

export interface SkillDetail extends SkillInfo {
  manifest: SkillManifest;
  download_url: string;
  format?: 'skill_md' | 'legacy_manifest' | string;
  entry?: string;
  content?: {
    body: string;
    content_hash: string;
    source_revision?: string;
    license_text: string;
    required_tools: string[];
    verification: { status: string; scope: string[] };
  };
}

export interface SkillManifest {
  id: string;
  name: string;
  version: string;
  description: string;
  author: string;
  category: string;
  tags: string[];
  call_type: 'http' | 'cli' | 'content';
  dependencies: DependencyConfig;
  config_schema?: any;
  http?: HttpConfig;
  cli?: CliConfig;
}

export interface DependencyConfig {
  python?: string;
  node?: string;
  packages?: string[];
}

export interface HttpConfig {
  port: number;
  start_command: string;
  health_check: string;
}

export interface CliConfig {
  command: string;
}

export interface SkillUpdate {
  skill_id: string;
  current_version: string;
  latest_version: string;
  changelog?: string;
}

export interface DependencyStatus {
  satisfied: boolean;
  missing: string[];
}

export interface SkillUsageLog {
  id: string;
  skill_id: string;
  user_id: string;
  input_params: any;
  output_result?: any;
  success: boolean;
  error_message?: string;
  latency_ms: number;
  created_at: number;
}

export interface PreparedContent {
  skill_id: string;
  version: string;
  content_hash: string;
  body: string;
}

interface RegistrySnapshot {
  skills: SkillInfo[];
  registry_url: string;
  offline: boolean;
  warning: string | null;
}

let registryRequest: Promise<void> | null = null;
let initializationRequest: Promise<void> | null = null;

const errorMessage = (error: unknown) => error instanceof Error ? error.message : String(error);

interface SkillsState {
  // Registry
  registrySkills: SkillInfo[];
  selectedSkill: SkillDetail | null;
  
  // Installed
  installedSkills: Skill[];
  
  // Updates
  availableUpdates: SkillUpdate[];
  
  // Usage logs
  usageLogs: SkillUsageLog[];
  
  // Loading states
  loading: boolean;
  installedLoading: boolean;
  installing: boolean;
  registryOffline: boolean;
  registryUrl: string | null;
  registryCachedCount: number;
  registryError: string | null;
  installedError: string | null;
  updatesError: string | null;
  
  // Actions
  initialize: () => Promise<void>;
  fetchRegistry: () => Promise<void>;
  searchSkills: (query: string) => Promise<void>;
  getSkillDetail: (skillId: string) => Promise<void>;
  
  installSkill: (skillId: string) => Promise<void>;
  uninstallSkill: (skillId: string) => Promise<void>;
  listInstalledSkills: () => Promise<void>;
  toggleSkill: (skillId: string, enabled: boolean) => Promise<void>;
  
  updateSkillConfig: (skillId: string, config: any) => Promise<void>;
  
  callSkill: (skillId: string, params: any) => Promise<any>;
  prepareContentSkill: (skillId: string) => Promise<PreparedContent>;
  
  checkUpdates: () => Promise<void>;
  updateSkill: (skillId: string) => Promise<void>;
  
  checkDependencies: (skillId: string) => Promise<DependencyStatus>;
  
  listUsageLogs: (limit?: number) => Promise<void>;
}

export const useSkillsStore = create<SkillsState>((set, get) => ({
  registrySkills: [],
  selectedSkill: null,
  installedSkills: [],
  availableUpdates: [],
  usageLogs: [],
  loading: false,
  installedLoading: false,
  installing: false,
  registryOffline: false,
  registryUrl: null,
  registryCachedCount: 0,
  registryError: null,
  installedError: null,
  updatesError: null,

  initialize: () => {
    // StrictMode replays effects; share the entire in-flight initialization.
    if (!initializationRequest) {
      initializationRequest = Promise.allSettled([
        get().fetchRegistry().then(() => get().checkUpdates()),
        get().listInstalledSkills(),
      ]).then(() => {}).finally(() => { initializationRequest = null; });
    }
    return initializationRequest;
  },

  fetchRegistry: () => {
    if (!registryRequest) {
      set({ loading: true, registryError: null });
      registryRequest = invoke<RegistrySnapshot>('fetch_skills_registry')
        .then(({ skills, registry_url, offline, warning }) => {
          set({
            registrySkills: skills,
            registryUrl: registry_url,
            registryCachedCount: skills.length,
            registryOffline: offline,
            registryError: warning,
          });
        })
        .catch((error: unknown) => {
          set({ registryError: errorMessage(error) });
        })
        .finally(() => {
          set({ loading: false });
          registryRequest = null;
        });
    }
    return registryRequest;
  },

  searchSkills: async (query: string) => {
    set({ loading: true });
    try {
      const skills = await invoke<SkillInfo[]>('search_skills', { query });
      set({ registrySkills: skills });
    } catch (error) {
      console.error('Failed to search skills:', error);
      throw error;
    } finally {
      set({ loading: false });
    }
  },

  getSkillDetail: async (skillId: string) => {
    set({ loading: true });
    try {
      const detail = await invoke<SkillDetail>('get_skill_detail', { skillId });
      set({ selectedSkill: detail });
    } catch (error) {
      console.error('Failed to get skill detail:', error);
      throw error;
    } finally {
      set({ loading: false });
    }
  },

  installSkill: async (skillId: string) => {
    set({ installing: true });
    try {
      await invoke<Skill>('install_skill', { skillId });
      await get().listInstalledSkills();
    } catch (error) {
      console.error('Failed to install skill:', error);
      throw error;
    } finally {
      set({ installing: false });
    }
  },

  uninstallSkill: async (skillId: string) => {
    try {
      await invoke('uninstall_skill', { skillId });
      await get().listInstalledSkills();
    } catch (error) {
      console.error('Failed to uninstall skill:', error);
      throw error;
    }
  },

  listInstalledSkills: async () => {
    set({ installedLoading: true, installedError: null });
    try {
      const skills = await invoke<Skill[]>('list_installed_skills', {});
      set({ installedSkills: skills });
    } catch (error) {
      set({ installedError: errorMessage(error) });
      throw error;
    } finally {
      set({ installedLoading: false });
    }
  },

  toggleSkill: async (skillId: string, enabled: boolean) => {
    try {
      await invoke('toggle_skill', { skillId, enabled });
      await get().listInstalledSkills();
    } catch (error) {
      console.error('Failed to toggle skill:', error);
      throw error;
    }
  },

  updateSkillConfig: async (skillId: string, config: any) => {
    try {
      await invoke('update_skill_config', { skillId, config });
      await get().listInstalledSkills();
    } catch (error) {
      console.error('Failed to update skill config:', error);
      throw error;
    }
  },

  callSkill: async (skillId: string, params: any) => {
    try {
      const result = await invoke('call_skill', { skillId, params });
      return result;
    } catch (error) {
      console.error('Failed to call skill:', error);
      throw error;
    }
  },

  prepareContentSkill: (skillId: string) =>
    invoke<PreparedContent>('prepare_content_skill', { skillId }),

  checkUpdates: async () => {
    set({ updatesError: null });
    try {
      const updates = await invoke<SkillUpdate[]>('check_skill_updates', {});
      set({ availableUpdates: updates });
    } catch (error) {
      set({ updatesError: errorMessage(error) });
      throw error;
    }
  },

  updateSkill: async (skillId: string) => {
    set({ installing: true });
    try {
      await invoke<Skill>('update_skill', { skillId });
      await get().listInstalledSkills();
      await get().checkUpdates();
    } catch (error) {
      console.error('Failed to update skill:', error);
      throw error;
    } finally {
      set({ installing: false });
    }
  },

  checkDependencies: async (skillId: string) => {
    try {
      const status = await invoke<DependencyStatus>('check_dependencies', { skillId });
      return status;
    } catch (error) {
      console.error('Failed to check dependencies:', error);
      throw error;
    }
  },

  listUsageLogs: async (limit?: number) => {
    try {
      const logs = await invoke<SkillUsageLog[]>('list_skill_usage_logs', { limit });
      set({ usageLogs: logs });
    } catch (error) {
      console.error('Failed to list usage logs:', error);
      throw error;
    }
  },
}));
