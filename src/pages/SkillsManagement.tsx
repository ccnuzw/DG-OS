import { t, useLocale as usePageLocale } from '../i18n';
import React, { useEffect, useState } from 'react';
import { Button, Card, Input, Loading, Modal, Stack, Switch, Tabs } from '@dgos/ui';
import { usePageError } from '../pageError';
import { SkillInstallPreview, useSkillsStore } from '../stores/skillsStore';
import { formatDate } from '../i18n';
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
  usePageLocale();
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
    previewExternalSource,
    previewExternalPackage,
    confirmExternalInstall,
    cancelExternalInstall,
  } = useSkillsStore();
  const reportError = usePageError('SkillsManagement');

  const [view, setView] = useState<'browse' | 'installed'>('browse');
  const [origin, setOrigin] = useState<'all' | 'official' | 'legacy'>('all');
  const [searchQuery, setSearchQuery] = useState('');
  const [selectedCategory, setSelectedCategory] = useState<string>('all');
  const [detailModalOpen, setDetailModalOpen] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const [actionNotice, setActionNotice] = useState<string | null>(null);
  const [externalSource, setExternalSource] = useState('');
  const [externalPreview, setExternalPreview] = useState<SkillInstallPreview | null>(null);

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
      setActionError(t('skills.searchFailed', { error: String(error) }));
      reportError(error);
    }
  };

  const handleViewDetail = async (skillId: string) => {
    setActionError(null);
    try {
      await getSkillDetail(skillId);
      setDetailModalOpen(true);
    } catch (error) {
      setActionError(t('skills.detailFailed', { error: String(error) }));
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
        setActionError(t('skills.missingDependencies', { items: depStatus.missing.join(', ') }));
        return;
      }

      await installSkill(skillId);
      setActionNotice(t('skills.added'));
    } catch (error) {
      setActionError(t('skills.addFailed', { error: String(error) }));
      reportError(error);
    }
  };

  const handleToggle = async (skillId: string, enabled: boolean) => {
    setActionError(null);
    try {
      await toggleSkill(skillId, enabled);
    } catch (error) {
      setActionError(t('skills.toggleFailed', { error: String(error) }));
      reportError(error);
    }
  };

  const handleExternalPreview = async () => {
    setActionError(null);
    setActionNotice(null);
    try {
      setExternalPreview(await previewExternalSource(externalSource.trim()));
    } catch (error) {
      setActionError(t('skills.sourceFailed', { error: String(error) }));
      reportError(error);
    }
  };

  const handleExternalPackage = async (event: React.ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0];
    event.target.value = '';
    if (!file) return;
    setActionError(null);
    setActionNotice(null);
    try {
      const bytes = Array.from(new Uint8Array(await file.arrayBuffer()));
      setExternalPreview(await previewExternalPackage(bytes));
    } catch (error) {
      setActionError(t('skills.packageFailed', { error: String(error) }));
      reportError(error);
    }
  };

  const handleExternalConfirm = async () => {
    if (!externalPreview) return;
    setActionError(null);
    try {
      await confirmExternalInstall(externalPreview.operation_id);
      setActionNotice(t('skills.installedSuccess', { name: externalPreview.name }));
      setExternalPreview(null);
      setExternalSource('');
    } catch (error) {
      setActionError(t('skills.installSourceFailed', { error: String(error) }));
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
    dgos_official: t('skills.identity.official'),
    dgos_curated: t('skills.identity.curated'),
    third_party: t('skills.identity.thirdParty'),
    legacy_registry: t('skills.identity.legacy'),
  }[identity] || identity);

  return (
    <div className="mx-auto flex h-full max-w-350 flex-col gap-4 overflow-y-auto p-6 text-foreground">
      <header>
        <h1 className="m-0 text-2xl font-semibold text-foreground">{t('skills.title')}</h1>
        <p className="m-0 mt-1 text-sm text-muted-foreground">{t('skills.subtitle')}</p>
      </header>

      <Tabs value={view} onValueChange={(next) => setView(next as 'browse' | 'installed')}>
        <Tabs.List aria-label={t('skills.views')} className="flex-wrap">
          <Tabs.Trigger value="browse">{t('skills.browse')}</Tabs.Trigger>
          <Tabs.Trigger value="installed">{t('skills.installedCount', { count: installedSkills.length })}</Tabs.Trigger>
        </Tabs.List>

        {registryUrl === 'bundled://official' && (
          <div className={NOTICE_INFO} role="status">
            <div className="min-w-0 flex-1">
              <strong>{t('skills.officialLocal')}</strong>
              <p className="m-0 mt-2">{t('skills.officialInfo')}</p>
            </div>
          </div>
        )}
        {actionNotice && <p className={`${NOTICE_SUCCESS} m-0`} role="status">{actionNotice}</p>}
        {(registryOffline || registryError) && (
          <div className={NOTICE_WARNING} role="status">
            <div className="min-w-0 flex-1 basis-60">
              <strong>{registryOffline && registryCachedCount > 0 ? t('skills.offline') : t('skills.unavailable')}</strong>
              <p className="m-0 mt-2">
                {t('skills.registryUnavailable')}</p>
              <p className="m-0 mt-2">{registryCachedCount > 0
                ? t('skills.showingCached')
                : t('skills.noRegistryCache')}</p>
              {registryError && (
                <details className="mt-2">
                  <summary className="cursor-pointer">{t('skills.technicalDetails')}</summary>
                  {registryUrl && <p className="m-0 mt-2">{t('detail.registry')}{registryUrl}</p>}
                  <p className="m-0 mt-2">{registryError}</p>
                </details>
              )}
            </div>
            <Button variant="secondary" onClick={handleRetry} disabled={loading}>
              {loading ? t('skills.retrying') : t('skills.retryConnection')}
            </Button>
          </div>
        )}
        {actionError && <p className={`${NOTICE_ERROR} m-0`} role="alert">{actionError}</p>}
        {installedError && (
          <div className={NOTICE_ERROR} role="alert">
            <p className="m-0 min-w-0 flex-1 basis-60">{t('detail.installedError')}{installedError}</p>
            <Button variant="secondary" onClick={() => { void initialize(); }} disabled={installedLoading}>{t('skills.retryInstalled')}</Button>
          </div>
        )}
        {updatesError && <p className={`${NOTICE_ERROR} m-0`} role="alert">{t('detail.updateError')}{updatesError}</p>}

        <Card>
          <Card.Header>
            <h2 className="m-0 text-base font-semibold">{t('skills.installSourceTitle')}</h2>
          </Card.Header>
          <Card.Body className="flex flex-col gap-3">
            <p className="m-0 text-sm text-muted-foreground">
              {t('detail.sourceHelp')}</p>
            <div className="flex flex-wrap items-end gap-2">
              <div className="min-w-0 flex-1 basis-80">
                <Input
                  value={externalSource}
                  onChange={(event) => setExternalSource(event.target.value)}
                  placeholder={t('skills.sourcePlaceholder')}
                  aria-label={t('skills.sourceLabel')}
                />
              </div>
              <Button onClick={() => { void handleExternalPreview(); }} disabled={!externalSource.trim() || installing}>
                {t('skills.previewSource')}</Button>
              <label className="inline-flex cursor-pointer items-center rounded-md border border-border px-3 py-2 text-sm font-medium hover:bg-muted">
                {t('detail.upload')}<input className="sr-only" type="file" accept=".zip,application/zip" onChange={(event) => { void handleExternalPackage(event); }} disabled={installing} />
              </label>
            </div>
            {externalPreview && (
              <div className={`${NOTICE_INFO} m-0`} role="dialog" aria-label={t('skills.previewLabel')}>
                <div className="min-w-0 flex-1">
                  <strong>{externalPreview.name} · {externalPreview.version}</strong>
                  <p className="m-0 mt-1">{externalPreview.description}</p>
                  <p className="m-0 mt-1 text-xs">
                    {externalPreview.source_kind} · {externalPreview.file_count} {t('detail.fileLicense')}{externalPreview.license_status}
                    {externalPreview.resolved_commit ? ` · commit ${externalPreview.resolved_commit}` : ''}
                  </p>
                  <p className="m-0 mt-1 break-all text-xs">{t('common.sha256')} {externalPreview.content_hash}</p>
                </div>
                <div className="flex shrink-0 gap-2">
                  <Button variant="secondary" onClick={() => { void cancelExternalInstall(externalPreview.operation_id); setExternalPreview(null); }}>
                    {t('module.cancel')}</Button>
                  <Button onClick={() => { void handleExternalConfirm(); }} disabled={installing}>
                    {t('skills.confirmInstall')}</Button>
                </div>
              </div>
            )}
          </Card.Body>
        </Card>

        <Tabs.Content value="browse">
          <Stack gap={4}>
            <div role="group" aria-label={t('skills.contentSource')} className="flex flex-wrap gap-2">
              {([['all', t('skills.allContent')], ['official', t('skills.identity.official')], ['legacy', t('skills.legacyArchive')]] as const).map(([value, label]) => (
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
                  placeholder={t('skills.searchPlaceholder')}
                  value={searchQuery}
                  onChange={(e) => setSearchQuery(e.target.value)}
                  aria-label={t('skills.searchLabel')}
                  onKeyDown={(e) => { if (e.key === 'Enter' && !loading) void handleSearch(); }}
                />
              </div>
              <Button onClick={handleSearch} disabled={loading}>
                {loading ? t('skills.searching') : t('skills.search')}
              </Button>
            </Stack>

            <div role="group" aria-label={t('skills.category')} className="flex flex-wrap gap-2">
              {categories.map(cat => (
                <Button
                  key={cat === 'all' ? t('skills.all') : cat}
                  size="sm"
                  variant={selectedCategory === cat ? 'primary' : 'secondary'}
                  aria-pressed={selectedCategory === cat}
                  onClick={() => setSelectedCategory(cat)}
                >
                  {cat === 'all' ? t('skills.all') : cat}
                </Button>
              ))}
            </div>

            {loading && <Loading label={t('skills.loading')} />}
            {!loading && filteredSkills.length === 0 && (
              <div className="rounded-lg border border-dashed border-border p-8 text-center">
                <p className="m-0 text-sm text-muted-foreground">{registryOffline || registryError
                  ? registryCachedCount > 0
                    ? t('skills.noCachedMatch')
                    : t('skills.noBrowseUntilOnline')
                  : t('skills.noSkillsFound')}</p>
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
                    {skill.identity === 'dgos_official' && <p className="m-0 text-xs text-muted-foreground">{t('detail.bundled')}{skill.license_spdx || 'License not declared'}</p>}
                    <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
                      <span>{t('common.versionPrefix')}{skill.latest_version}</span>
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
                      {t('skills.details')}</Button>
                    {isInstalled(skill.skill_id) ? (
                      <Button variant="secondary" disabled>{t('skills.installedAt')}</Button>
                    ) : (
                      <Button
                        onClick={() => handleInstall(skill.skill_id)}
                        disabled={installing || skill.identity !== 'dgos_official'}
                      >
                        {skill.identity !== 'dgos_official' ? t('skills.legacyReadOnly') : installing ? t('skills.adding') : t('skills.addToMySkills')}
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
            {installedLoading && <Loading label={t('skills.loadingInstalled')} />}
            {installedSkills.length === 0 && !installedLoading && !installedError ? (
              <div className="flex flex-col items-center gap-4 rounded-lg border border-dashed border-border p-8 text-center">
                <p className="m-0 text-sm text-muted-foreground">{t('skills.noInstalled')}</p>
                <Button onClick={() => setView('browse')}>{t('skills.browse')}</Button>
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
                          <span className={CHIP_NEUTRAL}>{t('common.versionPrefix')}{skill.version}</span>
                        </div>
                        <p className="m-0 text-muted-foreground">{skill.description}</p>
                        <div className="flex flex-wrap gap-x-4 gap-y-1 text-xs text-muted-foreground">
                           <span>{t('detail.installed')}{formatDate(skill.installed_at * 1000)}</span>
                          <span>{t('detail.category')}{skill.manifest.category}</span>
                        </div>
                      </div>

                      <div className="flex flex-wrap items-center gap-4">
                        <div className="inline-flex items-center gap-2">
                          <Switch
                            aria-label={t(skill.enabled ? 'skills.disableSkill' : 'skills.enableSkill', { name: skill.name })}
                            checked={skill.enabled}
                            disabled={skill.call_type !== 'content' && !skill.enabled}
                            onChange={(checked) => { void handleToggle(skill.skill_id, checked); }}
                          />
                          <span className="text-sm text-foreground">{skill.enabled ? t('providerForm.enabled') : t('module.disabled')}</span>
                        </div>
                        {skill.call_type === 'content' && <Button variant="secondary" onClick={() => handleViewDetail(skill.skill_id)}>{t('skills.viewLatest')}</Button>}
                        {skill.call_type !== 'content' && <span className="text-xs text-muted-foreground">{t('skills.legacyExecutionUnavailable')}</span>}
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
            <Button variant="secondary" disabled>{t('skills.alreadyInstalled')}</Button>
          ) : (
            <Button
              disabled={installing || selectedSkill.format !== 'skill_md'}
              onClick={() => {
                handleInstall(selectedSkill.skill_id);
                setDetailModalOpen(false);
              }}
            >
              {selectedSkill.format === 'skill_md' ? t('skills.addToMySkills') : t('skills.legacyReadOnly')}
            </Button>
          )}
        >
          <div className="flex min-w-0 flex-col gap-4 text-sm wrap-anywhere">
            {installedSkills.some(skill => skill.skill_id === selectedSkill.skill_id && skill.version !== selectedSkill.latest_version) && (
              <p className={`${NOTICE_WARNING} m-0`}>{t('skills.latestPreviewWarning')}</p>
            )}

            <section className="flex flex-col gap-2">
              <h3 className="m-0 text-base font-semibold">{t('skills.information')}</h3>
              <div className="skills-detail-grid">
                <div><strong>{t('detail.version')}</strong> {selectedSkill.latest_version}</div>
                <div><strong>{t('detail.author')}</strong> {selectedSkill.author}</div>
                <div><strong>{t('detail.category')}</strong> {selectedSkill.category}</div>
                <div><strong>{t('detail.content')}</strong> {selectedSkill.format === 'skill_md' ? t('skills.skillPackage') : t('skills.legacyManifest')}</div>
                <div><strong>{t('detail.license')}</strong> {selectedSkill.license_spdx || 'Not declared'}</div>
                <div><strong>{t('detail.source')}</strong> {selectedSkill.source_kind || 'Legacy registry'}</div>
                <div><strong>{t('detail.compatibleApps')}</strong> {selectedSkill.compatible_apps?.join(', ') || 'Not specified'}</div>
                {selectedSkill.format !== 'skill_md' && <div><strong>{t('detail.callType')}</strong> {selectedSkill.manifest.call_type}</div>}
              </div>
            </section>

            <section className="flex flex-col gap-2">
              <h3 className="m-0 text-base font-semibold">{t('skills.tags')}</h3>
              <div className="flex flex-wrap gap-1">
                {selectedSkill.tags.map(tag => (
                  <span key={tag} className={CHIP_NEUTRAL}>{tag}</span>
                ))}
              </div>
            </section>

            {selectedSkill.format === 'skill_md' && selectedSkill.content && (
              <section className="flex flex-col gap-2">
                <h3 className="m-0 text-base font-semibold">{t('skills.skillContent')}</h3>
                <p className="m-0 text-xs text-muted-foreground">{t('detail.contentHash')}{selectedSkill.content.content_hash}</p>
                <pre className="m-0 max-h-80 overflow-auto whitespace-pre-wrap rounded-md bg-muted p-3 text-xs">{selectedSkill.content.body}</pre>
                <p className="m-0">{t('detail.verification')}{selectedSkill.content.verification.status} {t('skills.integrityNote')}</p>
                <p className="m-0">{t('detail.revision')}{selectedSkill.content.source_revision || 'Not specified'}</p>
                <details>
                  <summary className="cursor-pointer">{t('skills.licenseText')}</summary>
                  <pre className="m-0 mt-2 max-h-60 overflow-auto whitespace-pre-wrap rounded-md bg-muted p-3 text-xs">{selectedSkill.content.license_text}</pre>
                </details>
              </section>
            )}

            {selectedSkill.manifest.dependencies && (
              <section className="flex flex-col gap-2">
                <h3 className="m-0 text-base font-semibold">{t('skills.dependencies')}</h3>
                <ul className="m-0 list-disc pl-5">
                  {selectedSkill.manifest.dependencies.python && (
                    <li>{t('skills.runtime.python')}{selectedSkill.manifest.dependencies.python}</li>
                  )}
                  {selectedSkill.manifest.dependencies.node && (
                    <li>{t('skills.runtime.node')}{selectedSkill.manifest.dependencies.node}</li>
                  )}
                  {selectedSkill.manifest.dependencies.packages && (
                    <li>{t('detail.packages')}{selectedSkill.manifest.dependencies.packages.join(', ')}</li>
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
