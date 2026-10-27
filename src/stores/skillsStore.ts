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
  call_type: 'http' | 'cli';
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
}

export interface SkillDetail extends SkillInfo {
  manifest: SkillManifest;
  download_url: string;
}

export interface SkillManifest {
  id: string;
  name: string;
  version: string;
  description: string;
  author: string;
  category: string;
  tags: string[];
  call_type: 'http' | 'cli';
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
  initialize: (userId: string) => Promise<void>;
  fetchRegistry: () => Promise<void>;
  searchSkills: (query: string) => Promise<void>;
  getSkillDetail: (skillId: string) => Promise<void>;
  
  installSkill: (skillId: string, userId: string) => Promise<void>;
  uninstallSkill: (skillId: string, userId: string) => Promise<void>;
  listInstalledSkills: (userId: string) => Promise<void>;
  toggleSkill: (skillId: string, userId: string, enabled: boolean) => Promise<void>;
  
  updateSkillConfig: (skillId: string, userId: string, config: any) => Promise<void>;
  
  callSkill: (skillId: string, userId: string, params: any) => Promise<any>;
  
  checkUpdates: (userId: string) => Promise<void>;
  updateSkill: (skillId: string, userId: string) => Promise<void>;
  
  checkDependencies: (skillId: string) => Promise<DependencyStatus>;
  
  listUsageLogs: (userId: string, limit?: number) => Promise<void>;
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

  initialize: (userId: string) => {
    // StrictMode replays effects; share the entire in-flight initialization.
    if (!initializationRequest) {
      initializationRequest = Promise.allSettled([
        get().fetchRegistry().then(() => get().checkUpdates(userId)),
        get().listInstalledSkills(userId),
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

  installSkill: async (skillId: string, userId: string) => {
    set({ installing: true });
    try {
      await invoke<Skill>('install_skill', { skillId, userId });
      await get().listInstalledSkills(userId);
    } catch (error) {
      console.error('Failed to install skill:', error);
      throw error;
    } finally {
      set({ installing: false });
    }
  },

  uninstallSkill: async (skillId: string, userId: string) => {
    try {
      await invoke('uninstall_skill', { skillId, userId });
      await get().listInstalledSkills(userId);
    } catch (error) {
      console.error('Failed to uninstall skill:', error);
      throw error;
    }
  },

  listInstalledSkills: async (userId: string) => {
    set({ installedLoading: true, installedError: null });
    try {
      const skills = await invoke<Skill[]>('list_installed_skills', { userId });
      set({ installedSkills: skills });
    } catch (error) {
      set({ installedError: errorMessage(error) });
      throw error;
    } finally {
      set({ installedLoading: false });
    }
  },

  toggleSkill: async (skillId: string, userId: string, enabled: boolean) => {
    try {
      await invoke('toggle_skill', { skillId, userId, enabled });
      await get().listInstalledSkills(userId);
    } catch (error) {
      console.error('Failed to toggle skill:', error);
      throw error;
    }
  },

  updateSkillConfig: async (skillId: string, userId: string, config: any) => {
    try {
      await invoke('update_skill_config', { skillId, userId, config });
      await get().listInstalledSkills(userId);
    } catch (error) {
      console.error('Failed to update skill config:', error);
      throw error;
    }
  },

  callSkill: async (skillId: string, userId: string, params: any) => {
    try {
      const result = await invoke('call_skill', { skillId, userId, params });
      return result;
    } catch (error) {
      console.error('Failed to call skill:', error);
      throw error;
    }
  },

  checkUpdates: async (userId: string) => {
    set({ updatesError: null });
    try {
      const updates = await invoke<SkillUpdate[]>('check_skill_updates', { userId });
      set({ availableUpdates: updates });
    } catch (error) {
      set({ updatesError: errorMessage(error) });
      throw error;
    }
  },

  updateSkill: async (skillId: string, userId: string) => {
    set({ installing: true });
    try {
      await invoke<Skill>('update_skill', { skillId, userId });
      await get().listInstalledSkills(userId);
      await get().checkUpdates(userId);
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

  listUsageLogs: async (userId: string, limit?: number) => {
    try {
      const logs = await invoke<SkillUsageLog[]>('list_skill_usage_logs', { userId, limit });
      set({ usageLogs: logs });
    } catch (error) {
      console.error('Failed to list usage logs:', error);
      throw error;
    }
  },
}));
