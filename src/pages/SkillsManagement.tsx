import React, { useEffect, useRef, useState } from 'react';
import { useSkillsStore } from '../stores/skillsStore';
import './SkillsManagement.css';

const SkillsManagement: React.FC = () => {
  const {
    registrySkills,
    selectedSkill,
    installedSkills,
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
    toggleSkill,
    checkDependencies,
  } = useSkillsStore();

  const [view, setView] = useState<'browse' | 'installed'>('browse');
  const [origin, setOrigin] = useState<'all' | 'official' | 'legacy'>('all');
  const [searchQuery, setSearchQuery] = useState('');
  const [selectedCategory, setSelectedCategory] = useState<string>('all');
  const [detailModalOpen, setDetailModalOpen] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const [actionNotice, setActionNotice] = useState<string | null>(null);
  const detailRef = useRef<HTMLDivElement>(null);

  // Desktop-only placeholder; derive from a trusted session before remote use.

  useEffect(() => {
    void initialize();
  }, [initialize]);

  useEffect(() => {
    if (!detailModalOpen) return;
    const previous = document.activeElement as HTMLElement | null;
    const dialog = detailRef.current;
    dialog?.querySelector<HTMLButtonElement>('button')?.focus();
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') setDetailModalOpen(false);
      if (event.key !== 'Tab' || !dialog) return;
      const controls = Array.from(dialog.querySelectorAll<HTMLElement>('button:not(:disabled), input, summary, [tabindex="0"]'));
      const first = controls[0];
      const last = controls[controls.length - 1];
      if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus(); }
      if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus(); }
    };
    document.addEventListener('keydown', onKey);
    return () => { document.removeEventListener('keydown', onKey); previous?.focus(); };
  }, [detailModalOpen]);

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
    await initialize();
  };

  const handleInstall = async (skillId: string) => {
    setActionError(null);
    setActionNotice(null);
    try {
      // Check dependencies first
      const depStatus = await checkDependencies(skillId);
      if (!depStatus.satisfied) {
        setActionError(`Missing dependencies: ${depStatus.missing.join(', ')}`);
        return;
      }

      await installSkill(skillId);
      setActionNotice('Skill added to My Skills. Select it explicitly in the AI Workbench to use it for a task.');
    } catch (error) {
      setActionError(`Unable to add skill: ${error}`);
    }
  };

  const handleToggle = async (skillId: string, enabled: boolean) => {
    setActionError(null);
    try {
      await toggleSkill(skillId, enabled);
    } catch (error) {
      setActionError(`Toggle failed: ${error}`);
    }
  };

  const categories = ['all', ...new Set(registrySkills.map(s => s.category))];
  const filteredSkills = registrySkills.filter(skill =>
    (selectedCategory === 'all' || skill.category === selectedCategory)
    && (origin === 'all' || (origin === 'official' ? skill.identity === 'dgos_official' : skill.source_kind === 'legacy_registry' || !skill.identity)));

  const isInstalled = (skillId: string) => {
    return installedSkills.some(s => s.skill_id === skillId);
  };

  const identityLabel = (identity: string) => ({
    dgos_official: 'DGOS Official',
    dgos_curated: 'DGOS Curated',
    third_party: 'Third-party',
    legacy_registry: 'Legacy format',
  }[identity] || identity);

  return (
    <div className="skills-management">
      <div className="skills-header">
        <div><h1>DGOS Skills Center</h1><p className="skills-subtitle">Local instructions and resources, organized for your work.</p></div>
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

      {registryUrl === 'bundled://official' && <div className="skills-local-summary" role="status"><strong>Official library is available locally</strong><p>No registry connection required. Third-party sources are not yet connected. Skills provide instructions, not executable scripts.</p></div>}
      {actionNotice && <p className="skills-notice" role="status">{actionNotice}</p>}
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
          <button onClick={() => { void initialize(); }} disabled={installedLoading}>Retry loading skills</button>
        </div>
      )}
      {updatesError && <p className="skills-error" role="alert">Unable to check skill updates: {updatesError}</p>}

      {view === 'browse' && (
        <div className="browse-view">
          <div className="category-filter" aria-label="Content source">
            {([['all', 'All content'], ['official', 'DGOS Official'], ['legacy', 'Legacy archive']] as const).map(([value, label]) => (
              <button key={value} aria-pressed={origin === value} className={origin === value ? 'active' : ''} onClick={() => setOrigin(value)}>{label}</button>
            ))}
          </div>
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
                aria-pressed={selectedCategory === cat}
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
                {skill.identity === 'dgos_official' && <p className="skill-provenance">Bundled content · {skill.license_spdx || 'License not declared'}</p>}
                <div className="skill-meta">
                  <span className="skill-version">v{skill.latest_version}</span>
                  <span className="skill-author">{skill.author}</span>
                  <span className={`skill-identity skill-identity-${skill.identity || 'legacy_registry'}`}>{identityLabel(skill.identity || 'legacy_registry')}</span>
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
                      disabled={installing || skill.identity !== 'dgos_official'}
                    >
                      {skill.identity !== 'dgos_official' ? 'Legacy: read-only' : installing ? 'Adding...' : 'Add to My Skills'}
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
                          {skill.call_type === 'content' ? 'SKILL.md' : skill.call_type.toUpperCase()}
                        </span>
                        <span className="badge version">v{skill.version}</span>
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
                        aria-label={`Enable ${skill.name}`}
                        disabled={skill.call_type !== 'content' && !skill.enabled}
                        onChange={(e) => handleToggle(skill.skill_id, e.target.checked)}
                      />
                      <span className="slider"></span>
                      <span className="label">{skill.enabled ? 'Enabled' : 'Disabled'}</span>
                    </label>

                    <div className="button-group">
                      {skill.call_type === 'content' && <button onClick={() => handleViewDetail(skill.skill_id)}>View latest content</button>}
                      {skill.call_type !== 'content' && <span className="skill-provenance">Legacy: execution unavailable</span>}
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
          <div ref={detailRef} role="dialog" aria-modal="true" aria-labelledby="skill-detail-title" className="modal-content" onClick={(e) => e.stopPropagation()}>
            <div className="modal-header">
              <h2 id="skill-detail-title">{selectedSkill.name}</h2>
              <button className="close" aria-label="Close skill details" onClick={() => setDetailModalOpen(false)}>×</button>
            </div>
            <div className="modal-body">
              <p className="description">{selectedSkill.description}</p>
              {installedSkills.some(skill => skill.skill_id === selectedSkill.skill_id && skill.version !== selectedSkill.latest_version) && (
                <p className="skills-notice">This preview shows the latest catalog version. Your selected version remains unchanged.</p>
              )}
              
              <div className="detail-section">
                <h3>Information</h3>
                <div className="detail-grid">
                  <div><strong>Version:</strong> {selectedSkill.latest_version}</div>
                  <div><strong>Author:</strong> {selectedSkill.author}</div>
                  <div><strong>Category:</strong> {selectedSkill.category}</div>
                  <div><strong>Content:</strong> {selectedSkill.format === 'skill_md' ? 'SKILL.md package' : 'Legacy manifest'}</div>
                  <div><strong>License:</strong> {selectedSkill.license_spdx || 'Not declared'}</div>
                  <div><strong>Source:</strong> {selectedSkill.source_kind || 'Legacy registry'}</div>
                  <div><strong>Compatible apps:</strong> {selectedSkill.compatible_apps?.join(', ') || 'Not specified'}</div>
                  {selectedSkill.format !== 'skill_md' && <div><strong>Call Type:</strong> {selectedSkill.manifest.call_type}</div>}
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

              {selectedSkill.format === 'skill_md' && selectedSkill.content && (
                <div className="detail-section skill-content-preview">
                  <h3>SKILL.md content</h3>
                  <p className="content-hash">Content hash: {selectedSkill.content.content_hash}</p>
                  <pre>{selectedSkill.content.body}</pre>
                  <p>Verification: {selectedSkill.content.verification.status} (package integrity is not task-quality verification).</p>
                  <p>Revision: {selectedSkill.content.source_revision || 'Not specified'}</p>
                  <details><summary>License text</summary><pre>{selectedSkill.content.license_text}</pre></details>
                </div>
              )}

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
                  disabled={installing || selectedSkill.format !== 'skill_md'}
                  onClick={() => {
                    handleInstall(selectedSkill.skill_id);
                    setDetailModalOpen(false);
                  }}
                >
                  {selectedSkill.format === 'skill_md' ? 'Add to My Skills' : 'Legacy: read-only'}
                </button>
              )}
            </div>
          </div>
        </div>
      )}


    </div>
  );
};

export default SkillsManagement;
