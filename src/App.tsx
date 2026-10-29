import { useState } from "react";
import { Routes, Route, Link, useLocation, useNavigate } from "react-router-dom";
import { invoke } from "@tauri-apps/api/core";
import ProviderManagement from "./pages/ProviderManagement";
import SkillsManagement from "./pages/SkillsManagement";
import Workbench from "./pages/Workbench";
import McpManagement from "./pages/McpManagement";
import McpConnectionNotice from "./components/McpConnectionNotice";
import ModuleHostBootstrap from "./modules/ModuleHostBootstrap";
import ModuleManagement from "./pages/ModuleManagement";
import ModulePanel from "./pages/ModulePanel";
import { useModuleInstances } from "./modules/ModuleSlots";
import { ErrorPage } from "@dgos/ui";
import { isHostSettingsPath } from "./modules/hostRoutes";
import "./App.css";

// Router matching decodes the path, so `/%73ettings` reaches this route. The raw location path is
// what the Rust gate sees (D037), so the page guards on it and never shows Settings where Rust denies.
function SettingsHome() {
  return (
    <>
      <div>Settings Page</div>
      <Link to="/settings/modules">Module management</Link>
    </>
  );
}

function SettingsPage() {
  const { pathname } = useLocation();
  if (!isHostSettingsPath(pathname)) return <div>Not found</div>;
  return (
    <Routes>
      <Route path="modules" element={<ModuleManagement />} />
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
        <Link key={moduleId} to={`/settings/modules/${encodeURIComponent(moduleId)}`}>{moduleId}</Link>
      ))}
    </>
  );
}

// AC19: a path that matches no route shows the shared error page, not a blank content area.
export function NotFoundRoute() {
  const navigate = useNavigate();
  return <ErrorPage kind="not-found" onHome={() => navigate("/")} />;
}

function App() {
  const [greetMsg, setGreetMsg] = useState("");
  const [name, setName] = useState("");

  async function greet() {
    setGreetMsg(await invoke("greet", { name }));
  }
  return (
    <div className="app">
      <ModuleHostBootstrap />
      <nav className="app-nav">
        <Link to="/">Home</Link>
        <Link to="/providers">Providers</Link>
        <Link to="/skills">Skills</Link>
        <Link to="/workbench">Workbench</Link>
        <Link to="/mcp">MCP</Link>
        <Link to="/settings">Settings</Link>
        <Link to="/settings/modules">Modules</Link>
        <ModuleNavLinks />
      </nav>

      <McpConnectionNotice />

      <div className="app-content">
        <Routes>
          <Route path="/" element={
            <div className="container">
              <h1>Welcome to DG-OS</h1>
              <p>A local-first AI application development framework</p>

              <div className="row">
                <input
                  id="greet-input"
                  onChange={(e) => setName(e.currentTarget.value)}
                  placeholder="Enter a name..."
                />
                <button onClick={() => greet()}>Greet</button>
              </div>
              <p>{greetMsg}</p>
            </div>
          } />
          <Route path="/providers" element={<ProviderManagement />} />
          <Route path="/skills" element={<SkillsManagement />} />
          <Route path="/workbench" element={<Workbench />} />
          <Route path="/mcp" element={<McpManagement />} />
          <Route path="/settings/*" caseSensitive element={<SettingsPage />} />
          <Route path="*" element={<NotFoundRoute />} />
        </Routes>
      </div>
    </div>
  );
}

export default App;
