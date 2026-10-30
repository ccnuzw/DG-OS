import { useState, type ReactNode } from "react";
import { Routes, Route, Link, NavLink, useLocation, useNavigate } from "react-router-dom";
import { invoke } from "@tauri-apps/api/core";
import ProviderManagement from "./pages/ProviderManagement";
import SkillsManagement from "./pages/SkillsManagement";
import Workbench from "./pages/Workbench";
import McpManagement from "./pages/McpManagement";
import McpConnectionNotice from "./components/McpConnectionNotice";
import ModuleHostBootstrap from "./modules/ModuleHostBootstrap";
import ModuleManagement from "./pages/ModuleManagement";
import ModulePanel from "./pages/ModulePanel";
import AppearanceSettings from "./pages/AppearanceSettings";
import { useModuleInstances } from "./modules/ModuleSlots";
import { Button, ErrorPage, Input, cn } from "@dgos/ui";
import { isHostSettingsPath } from "./modules/hostRoutes";
import { ROUTES } from "./appRoutes";
import { usePageError } from "./pageError";

// Router matching decodes the path, so `/%73ettings` reaches this route. The raw location path is
// what the Rust gate sees (D037), so the page guards on it and never shows Settings where Rust denies.
function SettingsHome() {
  return (
    <div className="flex flex-col items-start gap-3 p-6">
      <div className="text-lg font-semibold">Settings Page</div>
      <Link
        to="/settings/modules"
        className="text-primary underline focus-visible:outline-2 focus-visible:outline-focus-ring"
      >
        Module management
      </Link>
      <Link
        to={ROUTES.appearance}
        className="text-primary underline focus-visible:outline-2 focus-visible:outline-focus-ring"
      >
        Appearance
      </Link>
    </div>
  );
}

function SettingsPage() {
  const { pathname } = useLocation();
  if (!isHostSettingsPath(pathname)) return <div className="p-6">Not found</div>;
  return (
    <Routes>
      <Route path="modules" element={<ModuleManagement />} />
      <Route path="appearance" element={<AppearanceSettings />} />
      <Route path="modules/:moduleId" element={<ModulePanel />} />
      <Route path="*" element={<SettingsHome />} />
    </Routes>
  );
}

// One entry per module that currently has a live instance; it disappears when the instance unloads (D055).
function ModuleNavLinks() {
  const instances = useModuleInstances();
  const moduleIds = [...new Set(instances.map((instance) => instance.moduleId))];
  return (
    <>
      {moduleIds.map((moduleId) => (
        <NavItem key={moduleId} to={`/settings/modules/${encodeURIComponent(moduleId)}`}>{moduleId}</NavItem>
      ))}
    </>
  );
}

// `end` keeps "Home" and "Settings" from staying highlighted on every nested route.
function NavItem({ to, end, children }: { to: string; end?: boolean; children: ReactNode }) {
  return (
    <NavLink
      to={to}
      end={end}
      className={({ isActive }) =>
        cn(
          "rounded-md px-3 py-2 text-sm no-underline transition-colors",
          "focus-visible:outline-2 focus-visible:outline-focus-ring",
          isActive ? "bg-primary text-on-primary" : "text-foreground hover:bg-secondary",
        )
      }
    >
      {children}
    </NavLink>
  );
}

// AC19: a path that matches no route shows the shared error page, not a blank content area.
export function NotFoundRoute() {
  const navigate = useNavigate();
  return <ErrorPage kind="not-found" onHome={() => navigate("/")} />;
}

function Home() {
  const [greetMsg, setGreetMsg] = useState("");
  const [name, setName] = useState("");
  const reportError = usePageError("Home");

  async function greet() {
    try {
      setGreetMsg(await invoke("greet", { name }));
    } catch (error) {
      reportError(error);
    }
  }
  return (
    <div className="flex h-full flex-col items-center justify-center gap-4 p-8 text-center">
      <h1 className="text-4xl font-semibold">Welcome to DG-OS</h1>
      <p className="text-muted-foreground">A local-first AI application development framework</p>
      <div className="flex flex-wrap items-end justify-center gap-2">
        <Input
          id="greet-input"
          aria-label="Name"
          onChange={(e) => setName(e.currentTarget.value)}
          placeholder="Enter a name..."
        />
        <Button onClick={() => void greet()}>Greet</Button>
      </div>
      <p className="text-muted-foreground">{greetMsg}</p>
    </div>
  );
}

function App() {
  return (
    <div className="flex h-screen w-full flex-col">
      <ModuleHostBootstrap />
      <nav className="flex flex-wrap gap-2 border-b border-border bg-muted px-6 py-3">
        <NavItem to={ROUTES.home} end>Home</NavItem>
        <NavItem to={ROUTES.providers}>Providers</NavItem>
        <NavItem to={ROUTES.skills}>Skills</NavItem>
        <NavItem to={ROUTES.workbench}>Workbench</NavItem>
        <NavItem to={ROUTES.mcp}>MCP</NavItem>
        <NavItem to={ROUTES.settings} end>Settings</NavItem>
        <NavItem to={ROUTES.modules} end>Modules</NavItem>
        <ModuleNavLinks />
      </nav>

      <McpConnectionNotice />

      <div className="app-content min-h-0 flex-1 overflow-hidden">
        <Routes>
          <Route path={ROUTES.home} element={<Home />} />
          <Route path={ROUTES.providers} element={<ProviderManagement />} />
          <Route path={ROUTES.skills} element={<SkillsManagement />} />
          <Route path={ROUTES.workbench} element={<Workbench />} />
          <Route path={ROUTES.mcp} element={<McpManagement />} />
          <Route path="/settings/*" caseSensitive element={<SettingsPage />} />
          <Route path="*" element={<NotFoundRoute />} />
        </Routes>
      </div>
    </div>
  );
}

export default App;
