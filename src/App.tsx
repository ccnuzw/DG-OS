import { useState } from "react";
import { Routes, Route } from "react-router-dom";
import { invoke } from "@tauri-apps/api/core";
import "./App.css";

function App() {
  const [greetMsg, setGreetMsg] = useState("");
  const [name, setName] = useState("");

  async function greet() {
    setGreetMsg(await invoke("greet", { name }));
  }

  return (
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

      <Routes>
        <Route path="/" element={<div>Home Page</div>} />
        <Route path="/settings" element={<div>Settings Page</div>} />
      </Routes>
    </div>
  );
}

export default App;
