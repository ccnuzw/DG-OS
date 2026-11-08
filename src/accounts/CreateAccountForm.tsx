import { useRef, useState, type FormEvent } from 'react';
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
import { t, useLocale } from '../i18n';
import { PasswordField } from './PasswordField';

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
  onBusyChange,
  onCancel,
}: {
  submitLabel: string;
  onCreated: (state: AccountState) => void;
  onBusyChange?: (busy: boolean) => void;
  onCancel?: () => void;
}) {
  useLocale();
  const reportError = usePageError('CreateAccountForm');
  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');
  const [confirm, setConfirm] = useState('');
  const [errors, setErrors] = useState<FieldErrors>({});
  const [formError, setFormError] = useState<string>();
  const [busy, setBusy] = useState(false);
  const pending = useRef(false);

  const setPending = (next: boolean) => {
    pending.current = next;
    setBusy(next);
    onBusyChange?.(next);
  };

  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (pending.current) return;
    const next: FieldErrors = {
      username: usernameProblem(username),
      password: passwordProblem(password),
      confirm: confirmProblem(password, confirm),
    };
    setErrors(next);
    setFormError(undefined);
    if (next.username || next.password || next.confirm) return;

    setPending(true);
    let state: AccountState;
    try {
      state = await createAccount(username, password);
    } catch (error) {
      setPending(false);
      const text = accountErrorText(error);
      if (text) setFormError(text);
      else reportError(error);
      return;
    }
    setUsername('');
    setPassword('');
    setConfirm('');
    setPending(false);
    onCreated(state);
  };

  return (
    <form onSubmit={submit} noValidate>
      <Stack gap={4}>
        <Input
          label={t('account.username')}
          autoComplete="username"
          value={username}
          error={errors.username}
          onChange={(event) => setUsername(event.target.value)}
        />
        <PasswordField
          label={t('account.passwordLabel')}
          autoComplete="new-password"
          value={password}
          error={errors.password}
          onChange={(event) => setPassword(event.target.value)}
        />
        <PasswordField
          label={t('account.confirmPassword')}
          autoComplete="new-password"
          value={confirm}
          error={errors.confirm}
          onChange={(event) => setConfirm(event.target.value)}
        />
        {formError ? <InlineError>{formError}</InlineError> : null}
        <div className="flex flex-wrap gap-2">
          {onCancel ? <Button variant="secondary" disabled={busy} onClick={onCancel}>{t('account.cancel')}</Button> : null}
          <Button type="submit" loading={busy}>
            {submitLabel}
          </Button>
        </div>
      </Stack>
    </form>
  );
}
