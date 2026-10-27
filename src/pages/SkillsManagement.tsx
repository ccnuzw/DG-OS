import React, { useEffect, useState } from 'react';
import { useSkillsStore } from '../stores/skillsStore';
import type { Skill } from '../stores/skillsStore';
import './SkillsManagement.css';

const SkillsManagement: React.FC = () => {
  const {
    registrySkills,
    selectedSkill,
    installedSkills,
    availableUpdates,
    loading,
    installedLoading,
    installing,
    registryOffline,
    registryUrl,
    registryCachedCount,
    registryError,
    installedError,
    updatesError,
    initialize,
    searchSkills,
    getSkillDetail,
    installSkill,
    uninstallSkill,
    listInstalledSkills,
    toggleSkill,
    updateSkillConfig,
    updateSkill,
    checkDependencies,
  } = useSkillsStore();

  const [view, setView] = useState<'browse' | 'installed'>('browse');
  const [searchQuery, setSearchQuery] = useState('');
  const [selectedCategory, setSelectedCategory] = useState<string>('all');
  const [configModalOpen, setConfigModalOpen] = useState(false);
  const [configSkill, setConfigSkill] = useState<Skill | null>(null);
  const [configValues, setConfigValues] = useState<Record<string, unknown>>({});
  const [detailModalOpen, setDetailModalOpen] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);

  // Mock user ID - in production this would come from auth
  const userId = 'default-user';

  useEffect(() => {
    void initialize(userId);
  }, [initialize, userId]);

  const handleSearch = async () => {
    setActionError(null);
    setSelectedCategory('all');
    try {
      // Search the local cache, including when the registry is offline.
      await searchSkills(searchQuery.trim());
    } catch (error) {
      setActionError(`Search failed: ${error}`);
    }
  };

  const handleViewDetail = async (skillId: string) => {
    setActionError(null);
    try {
      await getSkillDetail(skillId);
      setDetailModalOpen(true);
    } catch (error) {
      setActionError(`Unable to load skill details: ${error}`);
    }
  };

  const handleRetry = async () => {
    setActionError(null);
    setSearchQuery('');
    setSelectedCategory('all');
    await initialize(userId);
  };

  const handleInstall = async (skillId: string) => {
    try {
      // Check dependencies first
      const depStatus = await checkDependencies(skillId);
      if (!depStatus.satisfied) {
        alert(`Missing dependencies:\n${depStatus.missing.join('\n')}`);
        return;
      }

      await installSkill(skillId, userId);
      alert('Skill installed successfully!');
      await listInstalledSkills(userId);
    } catch (error) {
      alert(`Installation failed: ${error}`);
    }
  };

  const handleUninstall = async (skillId: string) => {
    if (confirm('Are you sure you want to uninstall this skill?')) {
      try {
        await uninstallSkill(skillId, userId);
        alert('Skill uninstalled successfully!');
      } catch (error) {
        alert(`Uninstall failed: ${error}`);
      }
    }
  };

  const handleToggle = async (skillId: string, enabled: boolean) => {
    try {
      await toggleSkill(skillId, userId, enabled);
    } catch (error) {
      alert(`Toggle failed: ${error}`);
    }
  };

  const handleOpenConfig = (skill: Skill) => {
    setConfigSkill(skill);
    setConfigValues(skill.config || {});
    setConfigModalOpen(true);
  };

  const handleSaveConfig = async () => {
    if (!configSkill) return;

    try {
      await updateSkillConfig(configSkill.skill_id, userId, configValues);
      alert('Configuration saved successfully!');
      setConfigModalOpen(false);
      await listInstalledSkills(userId);
    } catch (error) {
      alert(`Save failed: ${error}`);
    }
  };

  const handleUpdate = async (skillId: string) => {
    if (confirm('Update this skill to the latest version?')) {
      try {
        await updateSkill(skillId, userId);
        alert('Skill updated successfully!');
      } catch (error) {
        alert(`Update failed: ${error}`);
      }
    }
  };

  const categories = ['all', ...new Set(registrySkills.map(s => s.category))];
  const filteredSkills = selectedCategory === 'all' 
    ? registrySkills 
    : registrySkills.filter(s => s.category === selectedCategory);

  const isInstalled = (skillId: string) => {
    return installedSkills.some(s => s.skill_id === skillId);
  };

  const hasUpdate = (skillId: string) => {
    return availableUpdates.some(u => u.skill_id === skillId);
  };

  return (
    <div className="skills-management">
      <div className="skills-header">
        <h1>Skills Management</h1>
        <div className="view-toggle">
          <button 
            className={view === 'browse' ? 'active' : ''} 
            onClick={() => setView('browse')}
          >
            Browse Skills
          </button>
          <button 
            className={view === 'installed' ? 'active' : ''} 
            onClick={() => setView('installed')}
          >
            Installed ({installedSkills.length})
          </button>
        </div>
      </div>

      {(registryOffline || registryError) && (
        <div className="skills-notice" role="status">
          <div>
            <strong>{registryOffline && registryCachedCount > 0 ? 'Offline mode' : 'Skills registry unavailable'}</strong>
            <p>
              The registry service could not be reached. Its address or service may be unavailable;
              this does not necessarily mean your device is offline.
            </p>
            <p>{registryCachedCount > 0
              ? 'Showing cached skills. Installed skills remain available.'
              : 'No registry has been cached yet. Online browsing requires an available registry service. You can still manage installed skills.'}</p>
            {registryError && (
              <details>
                <summary>Technical details</summary>
                {registryUrl && <p>Registry: {registryUrl}</p>}
                <p>{registryError}</p>
              </details>
            )}
          </div>
          <button onClick={handleRetry} disabled={loading}>
            {loading ? 'Retrying...' : 'Retry connection'}
          </button>
        </div>
      )}
      {actionError && <p className="skills-error" role="alert">{actionError}</p>}
      {installedError && (
        <div className="skills-error" role="alert">
          <p>Unable to load installed skills: {installedError}</p>
          <button onClick={() => { void initialize(userId); }} disabled={installedLoading}>Retry loading skills</button>
        </div>
      )}
      {updatesError && <p className="skills-error" role="alert">Unable to check skill updates: {updatesError}</p>}

      {view === 'browse' && (
        <div className="browse-view">
          <div className="search-bar">
            <input
              type="text"
              placeholder="Search skills..."
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              aria-label="Search skills"
              onKeyDown={(e) => { if (e.key === 'Enter' && !loading) void handleSearch(); }}
            />
            <button onClick={handleSearch} disabled={loading}>
              {loading ? 'Searching...' : 'Search'}
            </button>
          </div>

          <div className="category-filter">
            {categories.map(cat => (
              <button
                key={cat}
                className={selectedCategory === cat ? 'active' : ''}
                onClick={() => setSelectedCategory(cat)}
              >
                {cat}
              </button>
            ))}
          </div>

          {loading && <p role="status">Loading skills...</p>}
          {!loading && filteredSkills.length === 0 && (
            <div className="empty-state">
              <p>{registryOffline || registryError
                ? registryCachedCount > 0
                  ? 'No cached skills match your filters.'
                  : 'No skills are available to browse until the registry service is reachable.'
                : 'No skills found.'}</p>
            </div>
          )}
          <div className="skills-grid" aria-busy={loading}>
            {filteredSkills.map(skill => (
              <div key={skill.skill_id} className="skill-card">
                <div className="skill-header">
                  <h3>{skill.name}</h3>
                  <span className="skill-category">{skill.category}</span>
                </div>
                <p className="skill-description">{skill.description}</p>
                <div className="skill-meta">
                  <span className="skill-version">v{skill.latest_version}</span>
                  <span className="skill-author">{skill.author}</span>
                </div>
                <div className="skill-tags">
                  {skill.tags.map(tag => (
                    <span key={tag} className="tag">{tag}</span>
                  ))}
                </div>
                <div className="skill-actions">
                  <button onClick={() => handleViewDetail(skill.skill_id)}>
                    Details
                  </button>
                  {isInstalled(skill.skill_id) ? (
                    <button className="installed" disabled>Installed</button>
                  ) : (
                    <button 
                      className="install"
                      onClick={() => handleInstall(skill.skill_id)}
                      disabled={installing}
                    >
                      {installing ? 'Installing...' : 'Install'}
                    </button>
                  )}
                </div>
              </div>
            ))}
          </div>
        </div>
      )}

      {view === 'installed' && (
        <div className="installed-view">
          {installedLoading && <p role="status">Loading installed skills...</p>}
          {installedSkills.length === 0 && !installedLoading && !installedError ? (
            <div className="empty-state">
              <p>No skills installed yet.</p>
              <button onClick={() => setView('browse')}>Browse Skills</button>
            </div>
          ) : (
            <div className="installed-list">
              {installedSkills.map(skill => (
                <div key={skill.id} className="installed-skill-card">
                  <div className="skill-info">
                    <div className="skill-header">
                      <h3>{skill.name}</h3>
                      <div className="skill-badges">
                        <span className={`badge ${skill.call_type}`}>
                          {skill.call_type.toUpperCase()}
                        </span>
                        <span className="badge version">v{skill.version}</span>
                        {hasUpdate(skill.skill_id) && (
                          <span className="badge update">Update Available</span>
                        )}
                      </div>
                    </div>
                    <p>{skill.description}</p>
                    <div className="skill-meta">
                      <span>Installed: {new Date(skill.installed_at * 1000).toLocaleDateString()}</span>
                      <span>Category: {skill.manifest.category}</span>
                    </div>
                  </div>

                  <div className="skill-controls">
                    <label className="toggle-switch">
                      <input
                        type="checkbox"
                        checked={skill.enabled}
                        onChange={(e) => handleToggle(skill.skill_id, e.target.checked)}
                      />
                      <span className="slider"></span>
                      <span className="label">{skill.enabled ? 'Enabled' : 'Disabled'}</span>
                    </label>

                    <div className="button-group">
                      {skill.manifest.config_schema && (
                        <button onClick={() => handleOpenConfig(skill)}>
                          Configure
                        </button>
                      )}
                      {hasUpdate(skill.skill_id) && (
                        <button 
                          className="update"
                          onClick={() => handleUpdate(skill.skill_id)}
                        >
                          Update
                        </button>
                      )}
                      <button 
                        className="danger"
                        onClick={() => handleUninstall(skill.skill_id)}
                      >
                        Uninstall
                      </button>
                    </div>
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>
      )}

      {/* Detail Modal */}
      {detailModalOpen && selectedSkill && (
        <div className="modal-overlay" onClick={() => setDetailModalOpen(false)}>
          <div className="modal-content" onClick={(e) => e.stopPropagation()}>
            <div className="modal-header">
              <h2>{selectedSkill.name}</h2>
              <button className="close" onClick={() => setDetailModalOpen(false)}>×</button>
            </div>
            <div className="modal-body">
              <p className="description">{selectedSkill.description}</p>
              
              <div className="detail-section">
                <h3>Information</h3>
                <div className="detail-grid">
                  <div><strong>Version:</strong> {selectedSkill.latest_version}</div>
                  <div><strong>Author:</strong> {selectedSkill.author}</div>
                  <div><strong>Category:</strong> {selectedSkill.category}</div>
                  <div><strong>Call Type:</strong> {selectedSkill.manifest.call_type}</div>
                </div>
              </div>

              <div className="detail-section">
                <h3>Tags</h3>
                <div className="tags-list">
                  {selectedSkill.tags.map(tag => (
                    <span key={tag} className="tag">{tag}</span>
                  ))}
                </div>
              </div>

              {selectedSkill.manifest.dependencies && (
                <div className="detail-section">
                  <h3>Dependencies</h3>
                  <ul>
                    {selectedSkill.manifest.dependencies.python && (
                      <li>Python {selectedSkill.manifest.dependencies.python}</li>
                    )}
                    {selectedSkill.manifest.dependencies.node && (
                      <li>Node.js {selectedSkill.manifest.dependencies.node}</li>
                    )}
                    {selectedSkill.manifest.dependencies.packages && (
                      <li>Packages: {selectedSkill.manifest.dependencies.packages.join(', ')}</li>
                    )}
                  </ul>
                </div>
              )}
            </div>
            <div className="modal-footer">
              {isInstalled(selectedSkill.skill_id) ? (
                <button disabled>Already Installed</button>
              ) : (
                <button 
                  className="primary"
                  onClick={() => {
                    handleInstall(selectedSkill.skill_id);
                    setDetailModalOpen(false);
                  }}
                >
                  Install Skill
                </button>
              )}
            </div>
          </div>
        </div>
      )}

      {/* Config Modal */}
      {configModalOpen && configSkill && (
        <div className="modal-overlay" onClick={() => setConfigModalOpen(false)}>
          <div className="modal-content" onClick={(e) => e.stopPropagation()}>
            <div className="modal-header">
              <h2>Configure {configSkill.name}</h2>
              <button className="close" onClick={() => setConfigModalOpen(false)}>×</button>
            </div>
            <div className="modal-body">
              <form onSubmit={(e) => { e.preventDefault(); handleSaveConfig(); }}>
                {configSkill.manifest.config_schema &&
                  Object.entries(configSkill.manifest.config_schema).map(([key, schema]: [string, any]) => (
                    <div key={key} className="form-group">
                      <label htmlFor={key}>
                        {key}
                        {schema.required && <span className="required">*</span>}
                      </label>
                      {schema.encrypted && <span className="hint">🔒 Encrypted</span>}
                      <input
                        id={key}
                        type={schema.encrypted ? 'password' : 'text'}
                        value={(configValues[key] as string) || ''}
                        onChange={(e) => setConfigValues({
                          ...configValues,
                          [key]: e.target.value
                        })}
                        required={schema.required}
                        placeholder={schema.description}
                      />
                      {schema.description && (
                        <span className="help-text">{schema.description}</span>
                      )}
                    </div>
                  ))
                }
              </form>
            </div>
            <div className="modal-footer">
              <button onClick={() => setConfigModalOpen(false)}>Cancel</button>
              <button className="primary" onClick={handleSaveConfig}>
                Save Configuration
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
};

export default SkillsManagement;
