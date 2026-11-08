import { useState, type FormEvent } from 'react';
import { NavLink, useNavigate } from 'react-router-dom';
import { Button, DropdownMenu, Icon, Input, Modal, RadioGroup, Stack, cn } from '@dgos/ui';
import { FOCUS_RING } from '@dgos/ui/lib/focus';
import { ROUTES } from '../appRoutes';
import { usePageError } from '../pageError';
import { useAccount } from './AccountContext';
import { AccountCommandError, accountErrorText, signIn, signOut, type AccountSummary } from './accountApi';
import { InlineError } from './AuthShell';
import { reloadApp } from './reload';
import { useLocale } from '../i18n';

const LINK_CLASS = ({ isActive }: { isActive: boolean }) =>
  cn(
    'rounded-md px-3 py-2 text-sm no-underline transition-colors',
    FOCUS_RING,
    isActive ? 'bg-accent-default text-accent-on' : 'text-ink-primary hover:bg-surface-inset',
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
  const { t } = useLocale();
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
      setError(t('auth.enterPassword'));
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
      title={t('account.switch')}
      footer={
        <>
          <Button variant="secondary" disabled={busy} onClick={onClose}>
            {t('account.cancel')}
          </Button>
          {others.length > 0 ? (
            <Button loading={busy} onClick={() => void submit()}>
              {t('account.switchConfirm')}
            </Button>
          ) : null}
        </>
      }
    >
      {others.length === 0 ? (
        <p className="m-0 text-sm text-ink-secondary">{t('account.switchUnavailable')}</p>
      ) : (
        <form onSubmit={submit} noValidate>
          <Stack gap={4}>
            <RadioGroup
              label={t('account.select')}
              options={others.map((account) => ({ value: account.username, label: account.username }))}
              value={selected}
              onChange={setSelected}
            />
            <Input
              label={t('account.password')}
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
export default function AccountMenu({ collapsed = false }: { collapsed?: boolean }) {
  const { t } = useLocale();
  const navigate = useNavigate();
  const account = useAccount();
  const reportError = usePageError('AccountMenu');
  const [switching, setSwitching] = useState(false);
  const [busy, setBusy] = useState(false);

  if (!account) return null;
  const { state, refresh } = account;

  if (state.mode === 'no_accounts') {
    return (
      <div className="workspace-account-widget"><NavLink to={ROUTES.account} title={t('account.create')} aria-label={t('account.create')} className={LINK_CLASS}><span className="workspace-account-avatar"><Icon name="home" size="sm" /></span>{collapsed ? null : <span>{t('account.create')}</span>}</NavLink></div>
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
    <div className="workspace-account-widget">
      <DropdownMenu label={state.current.username} trigger={<Button variant="ghost" size="sm" className={collapsed ? 'justify-center' : 'justify-start'} disabled={busy} title={state.current.username} aria-label={state.current.username}><span className="workspace-account-avatar">{state.current.username.slice(0, 1)}</span>{collapsed ? null : <span className="workspace-account-name">{state.current.username}</span>}</Button>} items={[
        { kind: 'label', text: state.current.username },
        { kind: 'item', id: 'account', label: t('account.title'), icon: 'home', onSelect: () => navigate(ROUTES.account) },
        { kind: 'item', id: 'switch', label: t('account.switch'), onSelect: () => setSwitching(true) },
        { kind: 'item', id: 'signout', label: t('account.signOut'), danger: true, onSelect: () => void leave() },
      ]} />
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
