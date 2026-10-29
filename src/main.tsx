import React from "react";
import ReactDOM from "react-dom/client";
import { BrowserRouter } from "react-router-dom";
import App from "./App";
import "./index.css";
import { ErrorBoundary, ModalHost, ToastViewport, initAppearance } from "@dgos/ui";

// Sets data-theme/data-density and lifts the first-frame gate; never rejects (D065, FR-005 B05-25).
void initAppearance();

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <ErrorBoundary>
      <BrowserRouter future={{ v7_startTransition: true, v7_relativeSplatPath: true }}>
        <App />
      </BrowserRouter>
      <ToastViewport />
      <ModalHost />
    </ErrorBoundary>
  </React.StrictMode>,
);
