import React, { useEffect, useState } from 'react';
import { Button, Card, Input, Loading, Modal, Stack, Switch, Tabs } from '@dgos/ui';
import { usePageError } from '../pageError';
import { useSkillsStore } from '../stores/skillsStore';
import './SkillsManagement.css';

const NOTICE = 'flex flex-wrap items-start justify-between gap-4 rounded-lg border p-4 text-sm wrap-anywhere';
const NOTICE_INFO = `${NOTICE} border-info bg-info-bg text-info-text`;
const NOTICE_WARNING = `${NOTICE} border-warning bg-warning-bg text-warning-text`;
const NOTICE_SUCCESS = `${NOTICE} border-success bg-success-bg text-success-text`;
const NOTICE_ERROR = `${NOTICE} border-error bg-error-bg text-error-text`;
const CHIP = 'inline-flex items-center rounded-full px-2 text-xs font-medium';
const CHIP_NEUTRAL = `${CHIP} bg-secondary text-secondary-foreground`;
const IDENTITY_CHIP: Record<string, string> = {
  dgos_official: `${CHIP} bg-success-bg text-success-text`,
  dgos_curated: `${CHIP} bg-info-bg text-info-text`,
  third_party: `${CHIP} bg-warning-bg text-warning-text`,
  legacy_registry: CHIP_NEUTRAL,
};

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
  const reportError = usePageError('SkillsManagement');

  const [view, setView] = useState<'browse' | 'installed'>('browse');
  const [origin, setOrigin] = useState<'all' | 'official' | 'legacy'>('all');
  const [searchQuery, setSearchQuery] = useState('');
  const [selectedCategory, setSelectedCategory] = useState<string>('all');
  const [detailModalOpen, setDetailModalOpen] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const [actionNotice, setActionNotice] = useState<string | null>(null);

  // Desktop-only placeholder; derive from a trusted session before remote use.

  useEffect(() => {
    void initialize();
  }, [initialize]);

  const handleSearch = async () => {
    setActionError(null);
    setSelectedCategory('all');
    try {
      // Search the local cache, including when the registry is offline.
      await searchSkills(searchQuery.trim());
    } catch (error) {
      setActionError(`Search failed: ${error}`);
      reportError(error);
    }
  };

  const handleViewDetail = async (skillId: string) => {
    setActionError(null);
    try {
      await getSkillDetail(skillId);
      setDetailModalOpen(true);
    } catch (error) {
      setActionError(`Unable to load skill details: ${error}`);
      reportError(error);
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
      reportError(error);
    }
  };

  const handleToggle = async (skillId: string, enabled: boolean) => {
    setActionError(null);
    try {
      await toggleSkill(skillId, enabled);
    } catch (error) {
      setActionError(`Toggle failed: ${error}`);
      reportError(error);
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
    <div className="mx-auto flex h-full max-w-350 flex-col gap-4 overflow-y-auto p-6 text-foreground">
      <header>
        <h1 className="m-0 text-2xl font-semibold text-foreground">DGOS Skills Center</h1>
        <p className="m-0 mt-1 text-sm text-muted-foreground">Local instructions and resources, organized for your work.</p>
      </header>

      <Tabs value={view} onValueChange={(next) => setView(next as 'browse' | 'installed')}>
        <Tabs.List aria-label="Skills views" className="flex-wrap">
          <Tabs.Trigger value="browse">Browse Skills</Tabs.Trigger>
          <Tabs.Trigger value="installed">Installed ({installedSkills.length})</Tabs.Trigger>
        </Tabs.List>

        {registryUrl === 'bundled://official' && (
          <div className={NOTICE_INFO} role="status">
            <div className="min-w-0 flex-1">
              <strong>Official library is available locally</strong>
              <p className="m-0 mt-2">No registry connection required. Third-party sources are not yet connected. Skills provide instructions, not executable scripts.</p>
            </div>
          </div>
        )}
        {actionNotice && <p className={`${NOTICE_SUCCESS} m-0`} role="status">{actionNotice}</p>}
        {(registryOffline || registryError) && (
          <div className={NOTICE_WARNING} role="status">
            <div className="min-w-0 flex-1 basis-60">
              <strong>{registryOffline && registryCachedCount > 0 ? 'Offline mode' : 'Skills registry unavailable'}</strong>
              <p className="m-0 mt-2">
                The registry service could not be reached. Its address or service may be unavailable;
                this does not necessarily mean your device is offline.
              </p>
              <p className="m-0 mt-2">{registryCachedCount > 0
                ? 'Showing cached skills. Installed skills remain available.'
                : 'No registry has been cached yet. Online browsing requires an available registry service. You can still manage installed skills.'}</p>
              {registryError && (
                <details className="mt-2">
                  <summary className="cursor-pointer">Technical details</summary>
                  {registryUrl && <p className="m-0 mt-2">Registry: {registryUrl}</p>}
                  <p className="m-0 mt-2">{registryError}</p>
                </details>
              )}
            </div>
            <Button variant="secondary" onClick={handleRetry} disabled={loading}>
              {loading ? 'Retrying...' : 'Retry connection'}
            </Button>
          </div>
        )}
        {actionError && <p className={`${NOTICE_ERROR} m-0`} role="alert">{actionError}</p>}
        {installedError && (
          <div className={NOTICE_ERROR} role="alert">
            <p className="m-0 min-w-0 flex-1 basis-60">Unable to load installed skills: {installedError}</p>
            <Button variant="secondary" onClick={() => { void initialize(); }} disabled={installedLoading}>Retry loading skills</Button>
          </div>
        )}
        {updatesError && <p className={`${NOTICE_ERROR} m-0`} role="alert">Unable to check skill updates: {updatesError}</p>}

        <Tabs.Content value="browse">
          <Stack gap={4}>
            <div role="group" aria-label="Content source" className="flex flex-wrap gap-2">
              {([['all', 'All content'], ['official', 'DGOS Official'], ['legacy', 'Legacy archive']] as const).map(([value, label]) => (
                <Button
                  key={value}
                  size="sm"
                  variant={origin === value ? 'primary' : 'secondary'}
                  aria-pressed={origin === value}
                  onClick={() => setOrigin(value)}
                >
                  {label}
                </Button>
              ))}
            </div>

            <Stack direction="horizontal" gap={2} className="flex-wrap items-end">
              <div className="min-w-0 flex-1 basis-60">
                <Input
                  type="text"
                  placeholder="Search skills..."
                  value={searchQuery}
                  onChange={(e) => setSearchQuery(e.target.value)}
                  aria-label="Search skills"
                  onKeyDown={(e) => { if (e.key === 'Enter' && !loading) void handleSearch(); }}
                />
              </div>
              <Button onClick={handleSearch} disabled={loading}>
                {loading ? 'Searching...' : 'Search'}
              </Button>
            </Stack>

            <div role="group" aria-label="Category" className="flex flex-wrap gap-2">
              {categories.map(cat => (
                <Button
                  key={cat}
                  size="sm"
                  variant={selectedCategory === cat ? 'primary' : 'secondary'}
                  aria-pressed={selectedCategory === cat}
                  onClick={() => setSelectedCategory(cat)}
                >
                  {cat}
                </Button>
              ))}
            </div>

            {loading && <Loading label="Loading skills..." />}
            {!loading && filteredSkills.length === 0 && (
              <div className="rounded-lg border border-dashed border-border p-8 text-center">
                <p className="m-0 text-sm text-muted-foreground">{registryOffline || registryError
                  ? registryCachedCount > 0
                    ? 'No cached skills match your filters.'
                    : 'No skills are available to browse until the registry service is reachable.'
                  : 'No skills found.'}</p>
              </div>
            )}
            <div className="skills-grid" aria-busy={loading}>
              {filteredSkills.map(skill => (
                <Card key={skill.skill_id} className="flex min-w-0 flex-col">
                  <Card.Header className="flex flex-wrap items-center justify-between gap-2">
                    <h3 className="m-0 min-w-0 text-base font-semibold wrap-anywhere">{skill.name}</h3>
                    <span className={CHIP_NEUTRAL}>{skill.category}</span>
                  </Card.Header>
                  <Card.Body className="flex flex-1 flex-col gap-3 wrap-anywhere">
                    <p className="m-0 text-muted-foreground">{skill.description}</p>
                    {skill.identity === 'dgos_official' && <p className="m-0 text-xs text-muted-foreground">Bundled content · {skill.license_spdx || 'License not declared'}</p>}
                    <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
                      <span>v{skill.latest_version}</span>
                      <span>{skill.author}</span>
                      <span className={IDENTITY_CHIP[skill.identity || 'legacy_registry'] ?? CHIP_NEUTRAL}>{identityLabel(skill.identity || 'legacy_registry')}</span>
                    </div>
                    <div className="flex flex-wrap gap-1">
                      {skill.tags.map(tag => (
                        <span key={tag} className={CHIP_NEUTRAL}>{tag}</span>
                      ))}
                    </div>
                  </Card.Body>
                  <Card.Footer className="flex-wrap justify-start">
                    <Button variant="secondary" onClick={() => handleViewDetail(skill.skill_id)}>
                      Details
                    </Button>
                    {isInstalled(skill.skill_id) ? (
                      <Button variant="secondary" disabled>Installed</Button>
                    ) : (
                      <Button
                        onClick={() => handleInstall(skill.skill_id)}
                        disabled={installing || skill.identity !== 'dgos_official'}
                      >
                        {skill.identity !== 'dgos_official' ? 'Legacy: read-only' : installing ? 'Adding...' : 'Add to My Skills'}
                      </Button>
                    )}
                  </Card.Footer>
                </Card>
              ))}
            </div>
          </Stack>
        </Tabs.Content>

        <Tabs.Content value="installed">
          <Stack gap={4}>
            {installedLoading && <Loading label="Loading installed skills..." />}
            {installedSkills.length === 0 && !installedLoading && !installedError ? (
              <div className="flex flex-col items-center gap-4 rounded-lg border border-dashed border-border p-8 text-center">
                <p className="m-0 text-sm text-muted-foreground">No skills installed yet.</p>
                <Button onClick={() => setView('browse')}>Browse Skills</Button>
              </div>
            ) : (
              <Stack gap={3}>
                {installedSkills.map(skill => (
                  <Card key={skill.id}>
                    <Card.Body className="flex flex-wrap items-center justify-between gap-4">
                      <div className="flex min-w-0 flex-1 basis-60 flex-col gap-2 wrap-anywhere">
                        <div className="flex flex-wrap items-center gap-2">
                          <h3 className="m-0 text-base font-semibold">{skill.name}</h3>
                          <span className={CHIP_NEUTRAL}>
                            {skill.call_type === 'content' ? 'SKILL.md' : skill.call_type.toUpperCase()}
                          </span>
                          <span className={CHIP_NEUTRAL}>v{skill.version}</span>
                        </div>
                        <p className="m-0 text-muted-foreground">{skill.description}</p>
                        <div className="flex flex-wrap gap-x-4 gap-y-1 text-xs text-muted-foreground">
                          <span>Installed: {new Date(skill.installed_at * 1000).toLocaleDateString()}</span>
                          <span>Category: {skill.manifest.category}</span>
                        </div>
                      </div>

                      <div className="flex flex-wrap items-center gap-4">
                        <div className="inline-flex items-center gap-2">
                          <Switch
                            aria-label={`Enable ${skill.name}`}
                            checked={skill.enabled}
                            disabled={skill.call_type !== 'content' && !skill.enabled}
                            onChange={(checked) => { void handleToggle(skill.skill_id, checked); }}
                          />
                          <span className="text-sm text-foreground">{skill.enabled ? 'Enabled' : 'Disabled'}</span>
                        </div>
                        {skill.call_type === 'content' && <Button variant="secondary" onClick={() => handleViewDetail(skill.skill_id)}>View latest content</Button>}
                        {skill.call_type !== 'content' && <span className="text-xs text-muted-foreground">Legacy: execution unavailable</span>}
                      </div>
                    </Card.Body>
                  </Card>
                ))}
              </Stack>
            )}
          </Stack>
        </Tabs.Content>
      </Tabs>

      {/* Detail Modal */}
      {selectedSkill && (
        <Modal
          open={detailModalOpen}
          onOpenChange={setDetailModalOpen}
          title={selectedSkill.name}
          description={selectedSkill.description}
          footer={isInstalled(selectedSkill.skill_id) ? (
            <Button variant="secondary" disabled>Already Installed</Button>
          ) : (
            <Button
              disabled={installing || selectedSkill.format !== 'skill_md'}
              onClick={() => {
                handleInstall(selectedSkill.skill_id);
                setDetailModalOpen(false);
              }}
            >
              {selectedSkill.format === 'skill_md' ? 'Add to My Skills' : 'Legacy: read-only'}
            </Button>
          )}
        >
          <div className="flex min-w-0 flex-col gap-4 text-sm wrap-anywhere">
            {installedSkills.some(skill => skill.skill_id === selectedSkill.skill_id && skill.version !== selectedSkill.latest_version) && (
              <p className={`${NOTICE_WARNING} m-0`}>This preview shows the latest catalog version. Your selected version remains unchanged.</p>
            )}

            <section className="flex flex-col gap-2">
              <h3 className="m-0 text-base font-semibold">Information</h3>
              <div className="skills-detail-grid">
                <div><strong>Version:</strong> {selectedSkill.latest_version}</div>
                <div><strong>Author:</strong> {selectedSkill.author}</div>
                <div><strong>Category:</strong> {selectedSkill.category}</div>
                <div><strong>Content:</strong> {selectedSkill.format === 'skill_md' ? 'SKILL.md package' : 'Legacy manifest'}</div>
                <div><strong>License:</strong> {selectedSkill.license_spdx || 'Not declared'}</div>
                <div><strong>Source:</strong> {selectedSkill.source_kind || 'Legacy registry'}</div>
                <div><strong>Compatible apps:</strong> {selectedSkill.compatible_apps?.join(', ') || 'Not specified'}</div>
                {selectedSkill.format !== 'skill_md' && <div><strong>Call Type:</strong> {selectedSkill.manifest.call_type}</div>}
              </div>
            </section>

            <section className="flex flex-col gap-2">
              <h3 className="m-0 text-base font-semibold">Tags</h3>
              <div className="flex flex-wrap gap-1">
                {selectedSkill.tags.map(tag => (
                  <span key={tag} className={CHIP_NEUTRAL}>{tag}</span>
                ))}
              </div>
            </section>

            {selectedSkill.format === 'skill_md' && selectedSkill.content && (
              <section className="flex flex-col gap-2">
                <h3 className="m-0 text-base font-semibold">SKILL.md content</h3>
                <p className="m-0 text-xs text-muted-foreground">Content hash: {selectedSkill.content.content_hash}</p>
                <pre className="m-0 max-h-80 overflow-auto whitespace-pre-wrap rounded-md bg-muted p-3 text-xs">{selectedSkill.content.body}</pre>
                <p className="m-0">Verification: {selectedSkill.content.verification.status} (package integrity is not task-quality verification).</p>
                <p className="m-0">Revision: {selectedSkill.content.source_revision || 'Not specified'}</p>
                <details>
                  <summary className="cursor-pointer">License text</summary>
                  <pre className="m-0 mt-2 max-h-60 overflow-auto whitespace-pre-wrap rounded-md bg-muted p-3 text-xs">{selectedSkill.content.license_text}</pre>
                </details>
              </section>
            )}

            {selectedSkill.manifest.dependencies && (
              <section className="flex flex-col gap-2">
                <h3 className="m-0 text-base font-semibold">Dependencies</h3>
                <ul className="m-0 list-disc pl-5">
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
              </section>
            )}
          </div>
        </Modal>
      )}
    </div>
  );
};

export default SkillsManagement;
