import { useState, type FormEvent } from 'react';
import { Button, Input, Stack } from '@dgos/ui';
import { usePageError } from '../pageError';
import {
  accountErrorText,
  confirmProblem,
  createAccount,
  passwordProblem,
  usernameProblem,
  type AccountState,
} from './accountApi';
import { InlineError } from './AuthShell';

interface FieldErrors {
  username?: string;
  password?: string;
  confirm?: string;
}

// Username, password and confirmation. The confirmation is checked here only and never sent; the length rules are
// pre-checks of the host's, which stays the authority and answers 1008001/1008002/1008003 for anything this misses.
export function CreateAccountForm({
  submitLabel,
  onCreated,
}: {
  submitLabel: string;
  onCreated: (state: AccountState) => void;
}) {
  const reportError = usePageError('CreateAccountForm');
  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');
  const [confirm, setConfirm] = useState('');
  const [errors, setErrors] = useState<FieldErrors>({});
  const [formError, setFormError] = useState<string>();
  const [busy, setBusy] = useState(false);

  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (busy) return;
    const next: FieldErrors = {
      username: usernameProblem(username),
      password: passwordProblem(password),
      confirm: confirmProblem(password, confirm),
    };
    setErrors(next);
    setFormError(undefined);
    if (next.username || next.password || next.confirm) return;

    setBusy(true);
    let state: AccountState;
    try {
      state = await createAccount(username, password);
    } catch (error) {
      setBusy(false);
      const text = accountErrorText(error);
      if (text) setFormError(text);
      else reportError(error);
      return;
    }
    setUsername('');
    setPassword('');
    setConfirm('');
    setBusy(false);
    onCreated(state);
  };

  return (
    <form onSubmit={submit} noValidate>
      <Stack gap={4}>
        <Input
          label="用户名"
          autoComplete="username"
          value={username}
          error={errors.username}
          onChange={(event) => setUsername(event.target.value)}
        />
        <Input
          label="密码"
          type="password"
          autoComplete="new-password"
          value={password}
          error={errors.password}
          onChange={(event) => setPassword(event.target.value)}
        />
        <Input
          label="确认密码"
          type="password"
          autoComplete="new-password"
          value={confirm}
          error={errors.confirm}
          onChange={(event) => setConfirm(event.target.value)}
        />
        {formError ? <InlineError>{formError}</InlineError> : null}
        <div>
          <Button type="submit" loading={busy}>
            {submitLabel}
          </Button>
        </div>
      </Stack>
    </form>
  );
}
