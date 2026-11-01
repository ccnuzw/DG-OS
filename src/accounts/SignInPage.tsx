import { useState, type FormEvent } from 'react';
import { Button, Input, RadioGroup, Stack } from '@dgos/ui';
import { usePageError } from '../pageError';
import { accountErrorText, signIn, type AccountSummary } from './accountApi';
import { AuthShell, InlineError } from './AuthShell';
import { reloadApp } from './reload';

// `signed_out`: pick one of the existing accounts and enter its password. There is no way to create an account
// here; the host refuses `create_account` while signed out (1008007).
export function SignInPage({ accounts, preselected }: { accounts: AccountSummary[]; preselected: string | null }) {
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
      setError('请选择用户');
      return;
    }
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
      return;
    }
    reloadApp();
  };

  return (
    <AuthShell title="登录 DGOS">
      <form onSubmit={submit} noValidate>
        <Stack gap={4}>
          <RadioGroup
            label="用户"
            options={accounts.map((account) => ({ value: account.username, label: account.username }))}
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
          <div>
            <Button type="submit" loading={busy}>
              登录
            </Button>
          </div>
        </Stack>
      </form>
    </AuthShell>
  );
}
