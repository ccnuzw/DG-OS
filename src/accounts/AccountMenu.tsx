import { useState, type FormEvent } from 'react';
import { NavLink } from 'react-router-dom';
import { Button, Input, Modal, RadioGroup, Stack, cn } from '@dgos/ui';
import { ROUTES } from '../appRoutes';
import { usePageError } from '../pageError';
import { useAccount } from './AccountContext';
import { AccountCommandError, accountErrorText, signIn, signOut, type AccountSummary } from './accountApi';
import { InlineError } from './AuthShell';
import { reloadApp } from './reload';

const LINK_CLASS = ({ isActive }: { isActive: boolean }) =>
  cn(
    'rounded-md px-3 py-2 text-sm no-underline transition-colors',
    'focus-visible:outline-2 focus-visible:outline-focus-ring',
    isActive ? 'bg-primary text-on-primary' : 'text-foreground hover:bg-secondary',
  );

// Switching users re-authenticates: a wrong password leaves the current session and the rendered app untouched.
function SwitchUserDialog({
  current,
  accounts,
  onClose,
  onSessionInvalid,
}: {
  current: string;
  accounts: AccountSummary[];
  onClose: () => void;
  onSessionInvalid: () => void;
}) {
  const reportError = usePageError('AccountMenu');
  const others = accounts.filter((account) => account.username !== current);
  const [selected, setSelected] = useState(others[0]?.username ?? '');
  const [password, setPassword] = useState('');
  const [error, setError] = useState<string>();
  const [busy, setBusy] = useState(false);

  const submit = async (event?: FormEvent<HTMLFormElement>) => {
    event?.preventDefault();
    if (busy || selected === '') return;
    if (password === '') {
      setError('请输入密码');
      return;
    }
    setBusy(true);
    setError(undefined);
    try {
      await signIn(selected, password);
    } catch (failure) {
      setBusy(false);
      setPassword('');
      const text = accountErrorText(failure);
      if (text) setError(text);
      else reportError(failure);
      if (failure instanceof AccountCommandError && failure.code === 1008007) onSessionInvalid();
      return;
    }
    reloadApp();
  };

  return (
    <Modal
      open
      onOpenChange={(open) => {
        if (!open && !busy) onClose();
      }}
      title="切换用户"
      footer={
        <>
          <Button variant="secondary" disabled={busy} onClick={onClose}>
            取消
          </Button>
          {others.length > 0 ? (
            <Button loading={busy} onClick={() => void submit()}>
              切换
            </Button>
          ) : null}
        </>
      }
    >
      {others.length === 0 ? (
        <p className="m-0 text-sm text-muted-foreground">没有其他账户可切换。可在“账户”页新建账户。</p>
      ) : (
        <form onSubmit={submit} noValidate>
          <Stack gap={4}>
            <RadioGroup
              label="切换到"
              options={others.map((account) => ({ value: account.username, label: account.username }))}
              value={selected}
              onChange={setSelected}
            />
            <Input
              label="密码"
              type="password"
              autoComplete="current-password"
              value={password}
              onChange={(event) => setPassword(event.target.value)}
            />
            {error ? <InlineError>{error}</InlineError> : null}
          </Stack>
        </form>
      )}
    </Modal>
  );
}

// Nav widget. Renders nothing when the app runs without AccountGate.
export default function AccountMenu() {
  const account = useAccount();
  const reportError = usePageError('AccountMenu');
  const [switching, setSwitching] = useState(false);
  const [busy, setBusy] = useState(false);

  if (!account) return null;
  const { state, refresh } = account;

  if (state.mode === 'no_accounts') {
    return (
      <div className="ml-auto flex items-center gap-2">
        <NavLink to={ROUTES.account} className={LINK_CLASS}>
          创建账户
        </NavLink>
      </div>
    );
  }
  if (state.mode !== 'signed_in' || !state.current) return null;

  const leave = async () => {
    if (busy) return;
    setBusy(true);
    try {
      await signOut();
    } catch (error) {
      setBusy(false);
      reportError(error);
      return;
    }
    reloadApp();
  };

  return (
    <div className="ml-auto flex items-center gap-2">
      <span className="text-sm text-muted-foreground">{state.current.username}</span>
      <NavLink to={ROUTES.account} className={LINK_CLASS}>
        账户
      </NavLink>
      <Button variant="ghost" size="sm" onClick={() => setSwitching(true)}>
        切换用户
      </Button>
      <Button variant="ghost" size="sm" loading={busy} onClick={() => void leave()}>
        注销
      </Button>
      {switching ? (
        <SwitchUserDialog
          current={state.current.username}
          accounts={state.accounts}
          onClose={() => setSwitching(false)}
          onSessionInvalid={() => {
            setSwitching(false);
            refresh().catch(reportError);
          }}
        />
      ) : null}
    </div>
  );
}
