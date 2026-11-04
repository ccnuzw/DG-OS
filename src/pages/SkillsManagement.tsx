import { t, useLocale as usePageLocale } from '../i18n';
import React, { useEffect, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { Button, EmptyState, FilterBar, Input, Modal, Notice, ResourceCard, ResourceDetail, ResourceLayout, ResourceList, SearchField, SectionHeader, StatusBadge, Switch, Tabs } from '@dgos/ui';
import { usePageError } from '../pageError';
import { SkillInstallPreview, useSkillsStore } from '../stores/skillsStore';
import { formatDate } from '../i18n';
import './SkillsManagement.css';
import { useSurfaceCloseGuard, useSurfaceStatus } from '../workspace/SurfaceContext';
import { confirmDiscard } from '../workspace/closeCoordinator';


const SkillsManagement: React.FC = () => {
  usePageLocale();
  const navigate = useNavigate();
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
    uninstallSkill,
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
  const [actionError, setActionError] = useState<string | null>(null);
  const [actionNotice, setActionNotice] = useState<string | null>(null);
  const [externalSource, setExternalSource] = useState('');
  const [externalPreview, setExternalPreview] = useState<SkillInstallPreview | null>(null);
  useSurfaceStatus({ dirty: Boolean(externalSource || externalPreview) });
  useSurfaceCloseGuard(async () => {
    if (installing) return false;
    if (!externalSource && !externalPreview) return true;
    if (!await confirmDiscard()) return false;
    if (externalPreview) await cancelExternalInstall(externalPreview.operation_id);
    return true;
  });

  // Desktop-only placeholder; derive from a trusted session before remote use.

  useEffect(() => {
    void initialize();
  }, [initialize]);

  const handleSearch = async () => {
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


  const handleUninstall = (skillId: string, name: string) => {
    Modal.confirm({
      title: t('skills.uninstallTitle'),
      content: t('skills.uninstallConfirm', { name }),
      confirmText: t('skills.uninstall'),
      cancelText: t('module.cancel'),
      confirmVariant: 'danger',
      onConfirm: async () => {
        setActionError(null);
        try {
          await uninstallSkill(skillId);
          setActionNotice(t('skills.uninstalledSuccess', { name }));
        } catch (error) {
          setActionError(t('skills.uninstallFailed', { error: String(error) }));
          reportError(error);
        }
      },
    });
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
    <main className="skills-page flex h-full min-h-0 flex-col overflow-hidden bg-surface-canvas text-foreground">
      <header className="flex shrink-0 flex-wrap items-start justify-between gap-3 border-b border-border-subtle bg-surface-canvas px-5 py-3">
        <div className="min-w-0"><h1 className="m-0 text-lg font-semibold leading-tight">{t('skills.title')}</h1><p className="m-0 mt-1 max-w-prose break-words text-sm text-muted-foreground">{t('skills.subtitle')}</p></div>
        <div className="flex max-w-full flex-wrap items-center gap-2"><span className="text-xs text-muted-foreground" role="status">{registryOffline || registryError ? t('skills.unavailable') : t('skills.officialLocal')}</span>{(registryOffline || registryError) && <Button variant="secondary" size="xs" onClick={() => { void handleRetry(); }} disabled={loading}>{loading ? t('skills.retrying') : t('skills.retryConnection')}</Button>}</div>
      </header>
      <div className="min-h-0 flex-1 overflow-y-auto"><div className="skills-content mx-auto flex w-full max-w-350 flex-col gap-3 px-5 py-4">
        <Tabs value={view} onValueChange={(next) => setView(next as 'browse' | 'installed')}>
          <Tabs.List aria-label={t('skills.views')} className="w-fit max-w-full flex-wrap"><Tabs.Trigger value="browse">{t('skills.browse')}</Tabs.Trigger><Tabs.Trigger value="installed">{t('skills.installedCount', { count: installedSkills.length })}</Tabs.Trigger></Tabs.List>
          {registryUrl === 'bundled://official' && <p className="my-2 text-xs text-muted-foreground">{t('skills.officialInfo')}</p>}
          {actionNotice && <Notice tone="success">{actionNotice}</Notice>}
          {(registryOffline || registryError) && <Notice tone="warning" title={registryOffline && registryCachedCount > 0 ? t('skills.offline') : t('skills.unavailable')}><p>{t('skills.registryUnavailable')}</p><p>{registryCachedCount > 0 ? t('skills.showingCached') : t('skills.noRegistryCache')}</p>{registryError && <details className="mt-2"><summary className="cursor-pointer">{t('skills.technicalDetails')}</summary>{registryUrl && <p className="mt-2 break-all">{t('detail.registry')}{registryUrl}</p>}<p className="break-words">{registryError}</p></details>}</Notice>}
          {actionError && <Notice tone="error">{actionError}</Notice>}
          {installedError && <Notice tone="error" action={<Button variant="secondary" size="xs" onClick={() => { void initialize(); }} disabled={installedLoading}>{t('skills.retryInstalled')}</Button>}>{t('detail.installedError')}{installedError}</Notice>}
          {updatesError && <Notice tone="error">{t('detail.updateError')}{updatesError}</Notice>}
          <details className="rounded-md border border-border-subtle bg-surface-panel"><summary className="cursor-pointer px-3 py-2 text-sm font-medium focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-focus-ring">{t('skills.installSourceTitle')} <span className="ml-2 text-xs font-normal text-muted-foreground">{t('skills.sourceFormats')}</span></summary><div className="flex flex-col gap-3 border-t border-border-subtle p-3"><p className="m-0 text-sm text-muted-foreground">{t('detail.sourceHelp')}</p><div className="flex min-w-0 flex-wrap items-end gap-2"><div className="min-w-0 flex-1 basis-64"><Input value={externalSource} onChange={(event) => setExternalSource(event.target.value)} placeholder={t('skills.sourcePlaceholder')} aria-label={t('skills.sourceLabel')} /></div><Button size="xs" onClick={() => { void handleExternalPreview(); }} disabled={!externalSource.trim() || installing}>{t('skills.previewSource')}</Button><label className="inline-flex min-h-7 cursor-pointer items-center rounded-md border border-border-control px-3 text-sm font-medium hover:bg-surface-hover focus-within:outline-2 focus-within:outline-offset-2 focus-within:outline-focus-ring">{t('detail.upload')}<input className="sr-only" type="file" accept=".zip,application/zip" onChange={(event) => { void handleExternalPackage(event); }} disabled={installing} /></label></div>{externalPreview && <Notice tone="info" title={`${externalPreview.name} · ${externalPreview.version}`}><p>{externalPreview.description}</p><p className="break-words">{externalPreview.source_kind} · {externalPreview.file_count} {t('detail.fileLicense')}{externalPreview.license_status}{externalPreview.resolved_commit ? ` · commit ${externalPreview.resolved_commit}` : ''}</p><p className="break-all">{t('common.sha256')} {externalPreview.content_hash}</p><div className="mt-2 flex flex-wrap justify-end gap-2"><Button variant="secondary" size="xs" onClick={() => { void cancelExternalInstall(externalPreview.operation_id); setExternalPreview(null); }}>{t('module.cancel')}</Button><Button size="xs" onClick={() => { void handleExternalConfirm(); }} disabled={installing}>{t('skills.confirmInstall')}</Button></div></Notice>}</div></details>
          <Tabs.Content value="browse" className="mt-3">
            <FilterBar label={t('skills.contentSource')} className="mb-3"><div role="group" aria-label={t('skills.contentSource')} className="flex flex-wrap gap-1">{([['all', t('skills.allContent')], ['official', t('skills.identity.official')], ['legacy', t('skills.legacyArchive')]] as const).map(([value, label]) => <Button key={value} size="xs" variant={origin === value ? 'primary' : 'secondary'} aria-pressed={origin === value} onClick={() => setOrigin(value)}>{label}</Button>)}</div><SearchField value={searchQuery} onChange={setSearchQuery} label={t('skills.searchLabel')} placeholder={t('skills.searchPlaceholder')} className="min-w-52 flex-1" /><Button size="xs" onClick={() => { void handleSearch(); }} disabled={loading}>{loading ? t('skills.searching') : t('skills.search')}</Button></FilterBar>
            <div role="group" aria-label={t('skills.category')} className="mb-3 flex flex-wrap gap-1">{categories.map(cat => <Button key={cat} size="xs" variant={selectedCategory === cat ? 'primary' : 'ghost'} aria-pressed={selectedCategory === cat} onClick={() => setSelectedCategory(cat)}>{cat === 'all' ? t('skills.all') : cat}</Button>)}</div>
            <ResourceLayout listLabel={t('skills.browse')} detailLabel={t('skills.details')} className="skills-resource-layout" list={<ResourceList loading={loading} loadingLabel={t('skills.loading')} empty={!loading && filteredSkills.length === 0} emptyContent={<EmptyState title={registryOffline || registryError ? registryCachedCount > 0 ? t('skills.noCachedMatch') : t('skills.noBrowseUntilOnline') : t('skills.noSkillsFound')} action={<Button size="xs" variant="secondary" onClick={() => { setOrigin('all'); setSelectedCategory('all'); setSearchQuery(''); }}>{t('skills.clearFilters')}</Button>} />} className="skills-resource-list">{filteredSkills.map(skill => {
              const selected = selectedSkill?.skill_id === skill.skill_id, installed = isInstalled(skill.skill_id);
              return <ResourceCard key={skill.skill_id} title={skill.name} description={skill.description} status={<StatusBadge status={skill.identity === 'dgos_official' ? 'success' : skill.identity === 'legacy_registry' ? 'neutral' : 'info'} label={identityLabel(skill.identity || 'legacy_registry')} />} className={selected ? 'border-primary bg-surface-selected' : ''}>
                <div className="flex flex-wrap gap-x-3 gap-y-1 text-xs text-muted-foreground"><span>{t('common.versionPrefix')}{skill.latest_version}</span><span>{skill.author}</span><span>{skill.category}</span></div><div className="mt-2 flex flex-wrap gap-1">{skill.tags.map(tag => <span key={tag} className="rounded-sm bg-surface-inset px-1.5 py-0.5 text-xs text-muted-foreground">{tag}</span>)}</div><div className="mt-3 flex flex-wrap items-center gap-2 border-t border-border-subtle pt-2"><Button size="xs" variant="secondary" onClick={() => { void handleViewDetail(skill.skill_id); }}>{t('skills.details')}</Button>{installed ? <StatusBadge status="success" label={t('skills.installedBadge')} /> : skill.identity === 'dgos_official' ? <Button size="xs" onClick={() => { void handleInstall(skill.skill_id); }} disabled={installing}>{installing ? t('skills.adding') : t('skills.addToMySkills')}</Button> : <StatusBadge status="neutral" label={t('skills.legacyReadOnly')} />}</div>
              </ResourceCard>;
            })}</ResourceList>} detail={<ResourceDetail label={t('skills.details')} className="skills-resource-detail" empty={!selectedSkill} emptyContent={<EmptyState title={t('skills.selectSkill')} description={t('skills.selectSkillHelp')} />} loading={loading && Boolean(selectedSkill)}>{selectedSkill && <div className="flex min-w-0 flex-col gap-3"><SectionHeader as="h2" title={selectedSkill.name} description={selectedSkill.description} />
              {installedSkills.some(skill => skill.skill_id === selectedSkill.skill_id && skill.version !== selectedSkill.latest_version) && <Notice tone="warning">{t('skills.latestPreviewWarning')}</Notice>}
              <dl className="skills-facts grid min-w-0 gap-x-3 gap-y-2 border-y border-border-subtle py-3 text-sm"><dt>{t('detail.version')}</dt><dd>{selectedSkill.latest_version}</dd><dt>{t('detail.author')}</dt><dd>{selectedSkill.author || t('skills.notSpecified')}</dd><dt>{t('detail.category')}</dt><dd>{selectedSkill.category}</dd><dt>{t('detail.license')}</dt><dd>{selectedSkill.license_spdx || t('skills.notSpecified')}</dd><dt>{t('detail.source')}</dt><dd>{selectedSkill.source_kind || t('skills.identity.legacy')}</dd><dt>{t('detail.compatibleApps')}</dt><dd>{selectedSkill.compatible_apps?.join(', ') || t('skills.notSpecified')}</dd><dt>{t('detail.content')}</dt><dd>{selectedSkill.format === 'skill_md' ? t('skills.skillPackage') : t('skills.legacyManifest')}</dd></dl>
              <section><h3 className="mb-1 text-sm font-medium">{t('skills.tags')}</h3><div className="flex flex-wrap gap-1">{selectedSkill.tags.map(tag => <span key={tag} className="rounded-sm bg-surface-inset px-1.5 py-0.5 text-xs">{tag}</span>)}</div></section>
              {selectedSkill.format === 'skill_md' && selectedSkill.content && <section className="min-w-0"><h3 className="mb-1 text-sm font-medium">{t('skills.skillContent')}</h3><p className="break-all text-xs text-muted-foreground">{t('detail.contentHash')}{selectedSkill.content.content_hash}</p><pre className="max-h-64 overflow-auto whitespace-pre-wrap break-words rounded-md bg-surface-inset p-3 text-xs">{selectedSkill.content.body}</pre><p className="break-words text-xs">{t('detail.verification')}{selectedSkill.content.verification.status} {t('skills.integrityNote')}</p><p className="break-words text-xs">{t('detail.revision')}{selectedSkill.content.source_revision || t('skills.notSpecified')}</p><details><summary className="cursor-pointer text-sm">{t('skills.licenseText')}</summary><pre className="max-h-48 overflow-auto whitespace-pre-wrap break-words rounded-md bg-surface-inset p-3 text-xs">{selectedSkill.content.license_text}</pre></details></section>}
              {selectedSkill.manifest.dependencies && <section><h3 className="mb-1 text-sm font-medium">{t('skills.dependencies')}</h3><ul className="list-disc pl-5 text-sm">{selectedSkill.manifest.dependencies.python && <li>{t('skills.runtime.python')}{selectedSkill.manifest.dependencies.python}</li>}{selectedSkill.manifest.dependencies.node && <li>{t('skills.runtime.node')}{selectedSkill.manifest.dependencies.node}</li>}{selectedSkill.manifest.dependencies.packages && <li>{t('detail.packages')}{selectedSkill.manifest.dependencies.packages.join(', ')}</li>}</ul></section>}
              <div className="flex flex-wrap gap-2 border-t border-border-subtle pt-3">{isInstalled(selectedSkill.skill_id) ? <StatusBadge status="success" label={t('skills.alreadyInstalled')} /> : selectedSkill.identity === 'dgos_official' ? <Button size="xs" onClick={() => { void handleInstall(selectedSkill.skill_id); }} disabled={installing || selectedSkill.format !== 'skill_md'}>{t('skills.addToMySkills')}</Button> : <StatusBadge status="neutral" label={t('skills.legacyReadOnly')} />}</div>
              {isInstalled(selectedSkill.skill_id) ? <Button size="xs" variant="secondary" onClick={() => navigate('/tasks/single')}>{t('workbench.single')}</Button> : null}
            </div>}</ResourceDetail>} />
          </Tabs.Content>
          <Tabs.Content value="installed" className="mt-3"><ResourceList label={t('skills.installed')} loading={installedLoading} loadingLabel={t('skills.loadingInstalled')} error={Boolean(installedError)} errorContent={<Notice tone="error" action={<Button variant="secondary" size="xs" onClick={() => { void initialize(); }} disabled={installedLoading}>{t('skills.retryInstalled')}</Button>}>{t('detail.installedError')}{installedError}</Notice>} empty={!installedLoading && !installedError && installedSkills.length === 0} emptyContent={<EmptyState title={t('skills.noInstalled')} action={<Button size="xs" onClick={() => setView('browse')}>{t('skills.browse')}</Button>} />}><div className="flex flex-col gap-2">{installedSkills.map(skill => <ResourceCard key={skill.id} title={skill.name} description={skill.description} status={<StatusBadge status={skill.enabled ? 'success' : 'neutral'} label={skill.enabled ? t('providerForm.enabled') : t('module.disabled')} />}><div className="flex flex-wrap gap-x-4 gap-y-2 text-xs text-muted-foreground"><span>{skill.call_type === 'content' ? 'SKILL.md' : skill.call_type.toUpperCase()}</span><span>{t('common.versionPrefix')}{skill.version}</span><span>{t('detail.installed')}{formatDate(skill.installed_at * 1000)}</span><span>{t('detail.category')}{skill.manifest.category}</span></div><div className="mt-3 flex flex-wrap items-center gap-3 border-t border-border-subtle pt-2"><Switch aria-label={t(skill.enabled ? 'skills.disableSkill' : 'skills.enableSkill', { name: skill.name })} checked={skill.enabled} disabled={skill.call_type !== 'content' && !skill.enabled} onChange={(checked) => { void handleToggle(skill.skill_id, checked); }} />{skill.call_type === 'content' && <Button variant="secondary" size="xs" onClick={() => { void handleViewDetail(skill.skill_id); }}>{t('skills.viewLatest')}</Button>}{skill.call_type === 'content' && skill.install_path.startsWith('external/') && <Button variant="danger" size="xs" onClick={() => handleUninstall(skill.skill_id, skill.name)}>{t('skills.uninstall')}</Button>}{skill.call_type !== 'content' && <StatusBadge status="warning" label={t('skills.legacyExecutionUnavailable')} />}</div></ResourceCard>)}</div></ResourceList></Tabs.Content>
        </Tabs>
      </div></div>
    </main>
  );
};

export default SkillsManagement;
