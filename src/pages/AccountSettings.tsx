import { useState, type FormEvent } from 'react';
import { Button, Card, Input, Modal, Stack, toast } from '@dgos/ui';
import { useAccount } from '../accounts/AccountContext';
import {
  AccountCommandError,
  accountErrorText,
  changePassword,
  confirmProblem,
  deleteAccount,
  markCleanupNotice,
  passwordProblem,
} from '../accounts/accountApi';
import { InlineError } from '../accounts/AuthShell';
import { CreateAccountForm } from '../accounts/CreateAccountForm';
import { reloadApp } from '../accounts/reload';
import { usePageError } from '../pageError';
import { t, useLocale, useLocale as usePageLocale } from '../i18n';

// V1-FR-008 account page. Which user is signed in is the host's knowledge; this page sends passwords only and
// never a user id or token. Changing the password and deleting the account end the session on the host, so both
// finish with a full reload (the gate then shows the sign-in page).

function ChangePasswordCard() {
  usePageLocale();
  const reportError = usePageError('AccountSettings');
  const [current, setCurrent] = useState('');
  const [next, setNext] = useState('');
  const [confirm, setConfirm] = useState('');
  const [errors, setErrors] = useState<{ current?: string; next?: string; confirm?: string }>({});
  const [formError, setFormError] = useState<string>();
  const [busy, setBusy] = useState(false);

  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (busy) return;
    const problems = {
      current: current === '' ? t('account.currentPasswordRequired') : undefined,
      next: passwordProblem(next),
      confirm: confirmProblem(next, confirm),
    };
    setErrors(problems);
    setFormError(undefined);
    if (problems.current || problems.next || problems.confirm) return;
    setBusy(true);
    try {
      await changePassword(current, next);
    } catch (error) {
      setBusy(false);
      if (error instanceof AccountCommandError && error.code === 1008005) setFormError(t('account.currentPasswordWrong'));
      else {
        const text = accountErrorText(error);
        if (text) setFormError(text);
        else reportError(error);
      }
      return;
    }
    reloadApp();
  };

  return (
    <Card>
      <Card.Header>
        <h2 className="m-0 text-base font-semibold">{t('account.changePassword')}</h2>
      </Card.Header>
      <Card.Body>
        <form onSubmit={submit} noValidate>
          <Stack gap={4}>
            <p className="m-0 text-muted-foreground">{t('account.changePasswordHelp')}</p>
            <Input
              label={t('account.currentPassword')}
              type="password"
              autoComplete="current-password"
              value={current}
              error={errors.current}
              onChange={(event) => setCurrent(event.target.value)}
            />
            <Input
              label={t('account.newPassword')}
              type="password"
              autoComplete="new-password"
              value={next}
              error={errors.next}
              onChange={(event) => setNext(event.target.value)}
            />
            <Input
              label={t('account.confirmNewPassword')}
              type="password"
              autoComplete="new-password"
              value={confirm}
              error={errors.confirm}
              onChange={(event) => setConfirm(event.target.value)}
            />
            {formError ? <InlineError>{formError}</InlineError> : null}
            <div>
              <Button type="submit" loading={busy}>
                {t('account.changePassword')}
              </Button>
            </div>
          </Stack>
        </form>
      </Card.Body>
    </Card>
  );
}

function DeleteAccountDialog({ onClose, onSessionInvalid }: { onClose: () => void; onSessionInvalid: () => void }) {
  const { t } = useLocale();
  const reportError = usePageError('AccountSettings');
  const [password, setPassword] = useState('');
  const [error, setError] = useState<string>();
  const [busy, setBusy] = useState(false);

  const submit = async (event?: FormEvent<HTMLFormElement>) => {
    event?.preventDefault();
    if (busy) return;
    if (password === '') {
      setError(t('account.confirmDeletePassword'));
      return;
    }
    setBusy(true);
    setError(undefined);
    let cleanupPending: boolean;
    try {
      cleanupPending = (await deleteAccount(password)).credential_cleanup_pending;
    } catch (failure) {
      setBusy(false);
      setPassword('');
      const text = accountErrorText(failure);
      if (text) setError(text);
      else reportError(failure);
      if (failure instanceof AccountCommandError && failure.code === 1008007) onSessionInvalid();
      return;
    }
    if (cleanupPending) markCleanupNotice();
    reloadApp();
  };

  return (
    <Modal
      open
      onOpenChange={(open) => {
        if (!open && !busy) onClose();
      }}
      title={t('account.deleteTitle')}
      icon="warning"
      description={t('account.deleteDescription')}
      footer={
        <>
          <Button variant="secondary" disabled={busy} onClick={onClose}>
            {t('account.cancel')}
          </Button>
          <Button variant="danger" loading={busy} onClick={() => void submit()}>
            {t('account.confirmDelete')}
          </Button>
        </>
      }
    >
      <form onSubmit={submit} noValidate>
        <Stack gap={3}>
          <Input
            label={t('account.currentPassword')}
            type="password"
            autoComplete="current-password"
            value={password}
            onChange={(event) => setPassword(event.target.value)}
          />
          {error ? <InlineError>{error}</InlineError> : null}
        </Stack>
      </form>
    </Modal>
  );
}

export default function AccountSettings() {
  const { t } = useLocale();
  usePageLocale();
  const account = useAccount();
  const reportError = usePageError('AccountSettings');
  const [deleting, setDeleting] = useState(false);

  if (!account) {
    return (
      <div className="mx-auto flex max-w-2xl flex-col gap-6 p-6">
        <h1 className="m-0 text-2xl font-semibold text-foreground">{t('account.title')}</h1>
        <p className="m-0 text-sm text-muted-foreground">{t('account.unavailable')}</p>
      </div>
    );
  }
  const { state, refresh } = account;

  if (state.mode === 'no_accounts') {
    return (
      <div className="h-full overflow-y-auto">
        <div className="mx-auto flex max-w-2xl flex-col gap-6 p-6">
          <h1 className="m-0 text-2xl font-semibold text-foreground">{t('account.title')}</h1>
          <Card>
            <Card.Header>
              <h2 className="m-0 text-base font-semibold">{t('account.first')}</h2>
            </Card.Header>
            <Card.Body>
              <Stack gap={4}>
                <p className="m-0 text-muted-foreground">
                  {t('account.firstHelp')}
                </p>
                <CreateAccountForm submitLabel={t('account.create')} onCreated={reloadApp} />
              </Stack>
            </Card.Body>
          </Card>
        </div>
      </div>
    );
  }
  if (state.mode !== 'signed_in' || !state.current) return null;

  return (
    <div className="h-full overflow-y-auto">
      <div className="mx-auto flex max-w-2xl flex-col gap-6 p-6">
        <h1 className="m-0 text-2xl font-semibold text-foreground">{t('account.title')}</h1>
        <Card>
          <Card.Header>
            <h2 className="m-0 text-base font-semibold">{t('account.current')}</h2>
          </Card.Header>
          <Card.Body>
            <p className="m-0">
              {t('account.signedInAs')}<strong>{state.current.username}</strong>
            </p>
          </Card.Body>
        </Card>
        <Card>
          <Card.Header>
            <h2 className="m-0 text-base font-semibold">{t('account.addAnother')}</h2>
          </Card.Header>
          <Card.Body>
            <CreateAccountForm
              submitLabel={t('account.create')}
              onCreated={() => {
                toast.success(t('account.created'));
                refresh().catch(reportError);
              }}
            />
          </Card.Body>
        </Card>
        <ChangePasswordCard />
        <Card>
          <Card.Header>
            <h2 className="m-0 text-base font-semibold text-error-text">{t('account.deleteAccount')}</h2>
          </Card.Header>
          <Card.Body>
            <Stack gap={4}>
              <p className="m-0 text-muted-foreground">{t('account.deleteHelp')}</p>
              <div>
                <Button variant="danger" onClick={() => setDeleting(true)}>
                  {t('account.deleteAccount')}
                </Button>
              </div>
            </Stack>
          </Card.Body>
        </Card>
        {deleting ? (
          <DeleteAccountDialog
            onClose={() => setDeleting(false)}
            onSessionInvalid={() => {
              setDeleting(false);
              refresh().catch(reportError);
            }}
          />
        ) : null}
      </div>
    </div>
  );
}
