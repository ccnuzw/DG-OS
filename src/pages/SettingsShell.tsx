import { useState, type FocusEvent, type ReactNode } from 'react';
import { Route, Routes, useNavigate, useParams } from 'react-router-dom';
import { Icon, Select, Tabs } from '@dgos/ui';
import { ROUTES } from '../appRoutes';
import ProviderManagement from './ProviderManagement';
import ModuleManagement from './ModuleManagement';
import AccountSettings from './AccountSettings';
import AppearanceSettings from './AppearanceSettings';
import AboutSettings from './AboutSettings';
import LanSettings from './LanSettings';
import CliSettings from './CliSettings';
import StorageSettings from '../features/settings/storage/StorageSettings';
import { useLocale } from '../i18n';

// URL-owned category selection. Provider state survives category changes; each page owns its leave guard.

type SectionKey = 'providers' | 'modules' | 'appearance' | 'about' | 'lan' | 'account' | 'cli' | 'storage';

const SECTION_ROUTES: ReadonlyArray<{ key: SectionKey; route: string }> = [
  { key: 'providers', route: ROUTES.settingsProviders }, { key: 'modules', route: ROUTES.modules },
  { key: 'appearance', route: ROUTES.appearance }, { key: 'about', route: ROUTES.about }, { key: 'lan', route: ROUTES.lan },
  { key: 'account', route: ROUTES.account },
  { key: 'cli', route: ROUTES.cli },
  { key: 'storage', route: ROUTES.storage },
];

const KEPT_ALIVE: Record<SectionKey, boolean> = {
  providers: true, modules: false, appearance: false, about: false, lan: false, account: false, cli: false, storage: false,
};

const PANEL = 'workspace-settings-panel';
const SECTION_ICONS: Record<SectionKey, string> = { providers: 'globe', modules: 'menu', appearance: 'palette', about: 'file', lan: 'globe', account: 'home', cli: 'code', storage: 'folder' };

const revealFocusedTab = (event: FocusEvent<HTMLDivElement>) => {
  if (event.target.getAttribute('role') === 'tab') event.target.scrollIntoView({ block: 'nearest', inline: 'nearest' });
};

// The router has already decoded the splat, so this agrees with the routes it matched. Only `modules` has a deeper
// level (`modules/:moduleId`); any other path that is not exactly a section shows the overview, as before (D037 parity).
function sectionOf(splat: string): SectionKey | null {
  const parts = splat.split('/').filter(Boolean);
  const section = SECTION_ROUTES.find(entry => entry.key === parts[0]);
  if (!section) return null;
  if (parts.length === 1 || (section.key === 'modules' && parts.length === 2)) return section.key;
  return null;
}

function Overview() {
  const { t } = useLocale();
  return (
    <div className="mx-auto flex max-w-2xl flex-col gap-3 p-6 text-foreground">
      <Icon name="settings" size="lg" />
      <p className="m-0 text-sm text-muted-foreground">
        {t('settings.choose')}
      </p>
    </div>
  );
}

export default function SettingsShell() {
  const { t } = useLocale();
  const sections = SECTION_ROUTES.map(entry => ({ ...entry, label: t(entry.key === 'providers' ? 'nav.providers' : entry.key === 'account' ? 'account.title' : `settings.${entry.key}`) }));
  const { '*': splat = '' } = useParams();
  const navigate = useNavigate();
  const active = sectionOf(splat);
  const [visited, setVisited] = useState<ReadonlySet<SectionKey>>(() => new Set(active ? [active] : []));
  if (active && KEPT_ALIVE[active] && !visited.has(active)) setVisited(new Set([...visited, active]));

  const select = (value: string) => {
    const section = sections.find(entry => entry.key === value);
    if (section) navigate(section.route, { replace: true });
  };

  const panel = (key: SectionKey, content: ReactNode) => {
    if (KEPT_ALIVE[key]) {
      return (
        <Tabs.Content key={key} value={key} forceMount hidden={active !== key} className={PANEL}>
          {visited.has(key) ? content : null}
        </Tabs.Content>
      );
    }
    return <Tabs.Content key={key} value={key} className={PANEL}>{content}</Tabs.Content>;
  };

  return (
    <div className="workspace-settings-shell" data-settings-shell="">
      <div className="workspace-settings-select"><Select label={t('settings.aria')} value={active ?? ''} options={sections.map(section => ({ value: section.key, label: section.label }))} onChange={select} /></div>
      <Tabs value={active ?? ''} onValueChange={select} orientation="vertical" activationMode="manual" className="workspace-settings-tabs flex-row gap-0">
        <div className="workspace-settings-navigation" data-settings-tab-scroller="">
          <Tabs.List aria-label={t('settings.aria')} className="workspace-settings-categories items-stretch border-0" onFocus={revealFocusedTab}>
            {sections.map(section => <Tabs.Trigger key={section.key} value={section.key} className="workspace-settings-category mb-0 h-auto whitespace-normal border-b-0 px-2 py-2 text-left data-[state=active]:border-transparent data-[state=active]:bg-surface-selected"><Icon name={SECTION_ICONS[section.key]} size="sm" /><span>{section.label}</span></Tabs.Trigger>)}
          </Tabs.List>
        </div>
        {active === null && <div className={PANEL}><Overview /></div>}
        {panel('providers', <ProviderManagement />)}
        {panel('modules', (
          <Routes>
            <Route path="modules" element={<ModuleManagement />} />
          </Routes>
        ))}
        {panel('appearance', <AppearanceSettings />)}
        {panel('about', <AboutSettings />)}
        {panel('lan', <LanSettings />)}
        {panel('account', <AccountSettings />)}
        {panel('cli', <CliSettings />)}
        {panel('storage', <StorageSettings />)}
      </Tabs>
    </div>
  );
}
