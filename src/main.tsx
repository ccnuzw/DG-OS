import React from "react";
import ReactDOM from "react-dom/client";
import { BrowserRouter } from "react-router-dom";
import App from "./App";
import AccountGate from "./accounts/AccountGate";
import "./index.css";
import { ErrorBoundary, ModalHost, ToastViewport, initAppearance } from "@dgos/ui";
import { initLocale } from "./i18n";
import { LayoutScaleFeedback } from './ui/appearance/LayoutScaleFeedback';

// Sets data-theme/data-density and lifts the first-frame gate; never rejects (D065, FR-005 B05-25).
void (async () => {
  // Language is host-scoped and must settle before the account gate renders.
  // Otherwise the welcome/sign-in page could briefly render in the default
  // language and then switch after the host responds.
  await Promise.all([initAppearance(), initLocale()]);
  ReactDOM.createRoot(document.getElementById("root")!).render(
    <React.StrictMode>
      <ErrorBoundary>
        <BrowserRouter future={{ v7_startTransition: true, v7_relativeSplatPath: true }}>
          <AccountGate>
            <App />
          </AccountGate>
        </BrowserRouter>
        <ToastViewport />
        <LayoutScaleFeedback />
        <ModalHost />
      </ErrorBoundary>
    </React.StrictMode>,
  );
})();
