import { useState } from "react";
import { Routes, Route, Link } from "react-router-dom";
import { invoke } from "@tauri-apps/api/core";
import ProviderManagement from "./pages/ProviderManagement";
import "./App.css";

function App() {
  const [greetMsg, setGreetMsg] = useState("");
  const [name, setName] = useState("");

  async function greet() {
    setGreetMsg(await invoke("greet", { name }));
  }
  return (
    <div className="app">
      <nav className="app-nav">
        <Link to="/">Home</Link>
        <Link to="/providers">Providers</Link>
        <Link to="/settings">Settings</Link>
      </nav>

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
          <Route path="/settings" element={<div>Settings Page</div>} />
        </Routes>
      </div>
    </div>
  );
}

export default App;
