import { useState, type FormEvent } from 'react';
import { Button, Input, RadioGroup, Stack } from '@dgos/ui';
import { usePageError } from '../pageError';
import { accountErrorText, signIn, type AccountSummary } from './accountApi';
import { AuthShell, InlineError } from './AuthShell';
import { reloadApp } from './reload';
import { useLocale } from '../i18n';

// `signed_out`: pick one of the existing accounts and enter its password. There is no way to create an account
// here; the host refuses `create_account` while signed out (1008007).
export function SignInPage({ accounts, preselected }: { accounts: AccountSummary[]; preselected: string | null }) {
  const { t } = useLocale();
  const reportError = usePageError('SignInPage');
  const initial = accounts.some((account) => account.username === preselected) ? preselected : accounts[0]?.username;
  const [selected, setSelected] = useState(initial ?? '');
  const [password, setPassword] = useState('');
  const [error, setError] = useState<string>();
  const [busy, setBusy] = useState(false);

  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (busy) return;
    if (selected === '') {
       setError(t('auth.selectUser'));
      return;
    }
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
      return;
    }
    reloadApp();
  };

  return (
    <AuthShell title={t('auth.signIn')}>
      <form onSubmit={submit} noValidate>
        <Stack gap={4}>
          <RadioGroup
            label={t('auth.user')}
            options={accounts.map((account) => ({ value: account.username, label: account.username }))}
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
          <div>
            <Button type="submit" loading={busy}>
              {t('auth.signInButton')}
            </Button>
          </div>
        </Stack>
      </form>
    </AuthShell>
  );
}
