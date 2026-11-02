import { useState, type FocusEvent, type ReactNode } from 'react';
import { Route, Routes, useNavigate, useParams } from 'react-router-dom';
import { Tabs } from '@dgos/ui';
import { ROUTES } from '../appRoutes';
import ProviderManagement from './ProviderManagement';
import SkillsManagement from './SkillsManagement';
import McpManagement from './McpManagement';
import ModuleManagement from './ModuleManagement';
import ModulePanel from './ModulePanel';
import AppearanceSettings from './AppearanceSettings';
import AboutSettings from './AboutSettings';
import LanSettings from './LanSettings';
import { useLocale } from '../i18n';

// V1-FR-007 §7.1 (D085): the Settings shell. The selected tab comes from the URL, so a deep link opens on its tab
// and the tab follows back/forward. Tabs activate manually: arrow keys only move focus, Enter/Space or a click
// selects, because selecting a tab loads host data.
//
// Provider, Skills and MCP stay mounted (hidden) once visited, so an unsaved form, a selection or a search survives
// switching tabs inside Settings. The other sections hold no page state worth keeping: module state lives in the
// host and module DOM in the runtime host (D055), appearance in its controller, About and LAN have nothing to edit.

type SectionKey = 'providers' | 'skills' | 'mcp' | 'modules' | 'appearance' | 'about' | 'lan';

const SECTION_ROUTES: ReadonlyArray<{ key: SectionKey; route: string }> = [
  { key: 'providers', route: ROUTES.settingsProviders }, { key: 'skills', route: ROUTES.settingsSkills },
  { key: 'mcp', route: ROUTES.settingsMcp }, { key: 'modules', route: ROUTES.modules },
  { key: 'appearance', route: ROUTES.appearance }, { key: 'about', route: ROUTES.about }, { key: 'lan', route: ROUTES.lan },
];

const KEPT_ALIVE: Record<SectionKey, boolean> = {
  providers: true, skills: true, mcp: true, modules: false, appearance: false, about: false, lan: false,
};

const PANEL = 'min-h-0 flex-1 overflow-y-auto';

// The tab bar scrolls sideways at high scales. A scroller clips at its padding box, and a non-visible overflow-x
// makes overflow-y clip too, so the scroller is a wrapper whose padding holds the 2px ring and 2px offset of
// FOCUS_RING (scales down to 75% stay >= 4px); the list keeps its own baseline that the triggers overlap (-mb-px).
// focus() does not scroll a partly visible tab, so a focused tab is scrolled in explicitly; scroll-px keeps its
// ring clear of the scroller's side edges.
const TAB_SCROLLER = 'shrink-0 overflow-x-auto scroll-px-6 px-6 pt-2 pb-2';

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
      <h1 className="m-0 text-2xl font-semibold text-foreground">{t('settings.title')}</h1>
      <p className="m-0 text-sm text-muted-foreground">
        {t('settings.choose')}
      </p>
    </div>
  );
}

export default function SettingsShell() {
  const { t } = useLocale();
  const sections = SECTION_ROUTES.map(entry => ({ ...entry, label: entry.key === 'modules' ? t('settings.modules') : entry.key === 'appearance' ? t('settings.appearance') : entry.key === 'about' ? t('settings.about') : entry.key === 'lan' ? t('settings.lan') : entry.key === 'providers' ? t('nav.providers') : entry.key === 'skills' ? t('nav.skills') : t('nav.mcp') }));
  const { '*': splat = '' } = useParams();
  const navigate = useNavigate();
  const active = sectionOf(splat);
  const [visited, setVisited] = useState<ReadonlySet<SectionKey>>(() => new Set(active ? [active] : []));
  if (active && KEPT_ALIVE[active] && !visited.has(active)) setVisited(new Set([...visited, active]));

  const select = (value: string) => {
    const section = sections.find(entry => entry.key === value);
    if (section) navigate(section.route);
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
    <div className="flex h-full min-h-0 flex-col" data-settings-shell="">
      <Tabs value={active ?? ''} onValueChange={select} activationMode="manual" className="min-h-0 flex-1 gap-0">
        <div className={TAB_SCROLLER} data-settings-tab-scroller="">
          <Tabs.List aria-label={t('settings.aria')} className="flex w-max min-w-full" onFocus={revealFocusedTab}>
            {sections.map(section => (
              <Tabs.Trigger key={section.key} value={section.key} className="shrink-0">{section.label}</Tabs.Trigger>
            ))}
          </Tabs.List>
        </div>
        {active === null && <div className={PANEL}><Overview /></div>}
        {panel('providers', <ProviderManagement />)}
        {panel('skills', <SkillsManagement />)}
        {panel('mcp', <McpManagement />)}
        {panel('modules', (
          <Routes>
            <Route path="modules" element={<ModuleManagement />} />
            <Route path="modules/:moduleId" element={<ModulePanel />} />
          </Routes>
        ))}
        {panel('appearance', <AppearanceSettings />)}
        {panel('about', <AboutSettings />)}
        {panel('lan', <LanSettings />)}
      </Tabs>
    </div>
  );
}
