import { useCallback, useEffect, useMemo, useState, type ReactNode } from 'react';
import { Button, Loading, toast } from '@dgos/ui';
import { AccountContext, type AccountContextValue } from './AccountContext';
import { databaseFailure, getAccountState, takeCleanupNotice, type AccountState, type DatabaseFailure } from './accountApi';
import { AuthShell } from './AuthShell';
import { SignInPage } from './SignInPage';
import { WelcomePage } from './WelcomePage';

const CLEANUP_TOAST_MS = 10_000;

// V1-FR-008. Reads the account state once on mount and decides what the window shows:
//   no_accounts -> welcome page (or the app, if the user skipped it for this page load),
//   signed_out  -> sign-in page, the app is not rendered,
//   signed_in   -> the app, with the state available through AccountContext.
// A host that does not register the account commands (the WebView smoke harnesses) has no account storage: the
// app renders without the gate, as before FR-008. Any other read failure shows the retry panel. The gate is not the
// security boundary: the host resolves the session for every command and answers 1008007 while signed out.
// Session changes end in a full reload (see ./reload), so the gate starts over with the new state.
export default function AccountGate({ children }: { children: ReactNode }) {
  const [state, setState] = useState<AccountState | null>(null);
  const [unsupported, setUnsupported] = useState(false);
  const [failed, setFailed] = useState(false);
  // Startup could not open or migrate the database (1008012, D116). A retry cannot help: the host stopped
  // before managing anything else, so this page stays until the user deals with the file and restarts.
  const [database, setDatabase] = useState<DatabaseFailure | null>(null);
  const [attempt, setAttempt] = useState(0);
  // D087: not stored. A reload asks again.
  const [skipped, setSkipped] = useState(false);

  useEffect(() => {
    let cancelled = false;
    getAccountState().then(
      (next) => {
        if (cancelled) return;
        if (next === null) {
          setUnsupported(true);
          return;
        }
        setState(next);
        // After a deletion whose credential cleanup is pending the page reloads once; the reminder shows here.
        if (takeCleanupNotice()) {
          toast.warning('账户已删除，但部分系统凭据尚未清理', {
            description: '宿主会在之后的启动中自动重试清理',
            duration: CLEANUP_TOAST_MS,
          });
        }
      },
      (error: unknown) => {
        if (cancelled) return;
        const failure = databaseFailure(error);
        if (failure) setDatabase(failure);
        else setFailed(true);
      },
    );
    return () => {
      cancelled = true;
    };
  }, [attempt]);

  const refresh = useCallback(async () => {
    const next = await getAccountState();
    if (next) setState(next);
  }, []);

  const retry = () => {
    setFailed(false);
    setAttempt((value) => value + 1);
  };

  const value = useMemo<AccountContextValue | null>(() => (state ? { state, refresh } : null), [state, refresh]);

  if (database) return <DatabaseFailurePage failure={database} />;
  if (failed) {
    return (
      <AuthShell title="无法读取账户状态">
        <p className="m-0 text-sm text-muted-foreground">读取本机账户信息失败，请重试。</p>
        <div>
          <Button onClick={retry}>重试</Button>
        </div>
      </AuthShell>
    );
  }
  if (unsupported) return <>{children}</>;
  if (!state || !value) {
    return (
      <div className="flex min-h-screen w-full items-center justify-center">
        <Loading size="lg" label="正在读取账户状态" />
      </div>
    );
  }
  if (state.mode === 'no_accounts' && !skipped) return <WelcomePage onSkip={() => setSkipped(true)} />;
  if (state.mode === 'signed_out') return <SignInPage accounts={state.accounts} preselected={state.preselected} />;
  return <AccountContext.Provider value={value}>{children}</AccountContext.Provider>;
}

const STAGE_TEXT: Record<DatabaseFailure['stage'], string> = {
  open: '打开数据库文件时失败',
  migrate: '升级数据库结构时失败',
};

// FR-008 §5.2 / §10.3 (D098 item 9): show the error and the file location, repair nothing.
function DatabaseFailurePage({ failure }: { failure: DatabaseFailure }) {
  return (
    <AuthShell title="无法打开本机数据库">
      <p className="m-0 text-sm font-medium text-error-text">{STAGE_TEXT[failure.stage]}</p>
      <div>
        <p className="m-0 text-xs text-muted-foreground">原因</p>
        <p className="m-0 text-sm text-foreground wrap-anywhere">{failure.reason}</p>
      </div>
      <div>
        <p className="m-0 text-xs text-muted-foreground">数据库文件</p>
        <code className="block select-text text-sm text-foreground wrap-anywhere">{failure.path}</code>
      </div>
      <p className="m-0 text-sm text-muted-foreground">
        DGOS 没有修改这个文件，也不会自动修复。请先关闭应用，备份或移走该文件后再重新启动；移走后会以空数据库启动。
      </p>
    </AuthShell>
  );
}
