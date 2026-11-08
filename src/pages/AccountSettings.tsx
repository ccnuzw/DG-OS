import { useId, useRef, useState, type FormEvent } from 'react';
import { Button, Modal, Stack, toast } from '@dgos/ui';
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
import { PasswordField } from '../accounts/PasswordField';
import { reloadApp } from '../accounts/reload';
import { usePageError } from '../pageError';
import { t, useLocale, useLocale as usePageLocale } from '../i18n';
import './account-settings.css';

// V1-FR-008 account page. Which user is signed in is the host's knowledge; this page sends passwords only and
// never a user id or token. Changing the password and deleting the account end the session on the host, so both
// finish with a full reload (the gate then shows the sign-in page).

function ChangePasswordSection() {
  usePageLocale();
  const reportError = usePageError('AccountSettings');
  const passwordHintId = useId();
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
    <section className="account-settings-section" aria-labelledby="account-security-title">
      <header className="account-section-heading">
        <h2 id="account-security-title">{t('account.security')}</h2>
        <p>{t('account.changePasswordHelp')}</p>
      </header>
      <form className="account-password-form" onSubmit={submit} noValidate aria-busy={busy}>
        <PasswordField
          label={t('account.currentPassword')}
          autoComplete="current-password"
          value={current}
          error={errors.current}
          onChange={(event) => setCurrent(event.target.value)}
        />
        <div className="account-password-pair">
          <PasswordField
            label={t('account.newPassword')}
            autoComplete="new-password"
            aria-describedby={passwordHintId}
            value={next}
            error={errors.next}
            onChange={(event) => setNext(event.target.value)}
          />
          <PasswordField
            label={t('account.confirmNewPassword')}
            autoComplete="new-password"
            value={confirm}
            error={errors.confirm}
            onChange={(event) => setConfirm(event.target.value)}
          />
        </div>
        <p className="account-settings-help" id={passwordHintId}>{t('account.passwordHint')}</p>
        {formError ? <InlineError>{formError}</InlineError> : null}
        <div className="account-form-actions">
          <Button type="submit" loading={busy}>
            {t('account.changePassword')}
          </Button>
        </div>
      </form>
    </section>
  );
}

function DeleteAccountDialog({ username, onClose, onSessionInvalid }: { username: string; onClose: () => void; onSessionInvalid: () => void }) {
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
      description={t('account.deleteDescription', { username })}
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
          <PasswordField
            label={t('account.currentPassword')}
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

function CreateAccountDialog({ onClose, onCreated }: { onClose: () => void; onCreated: () => void }) {
  const { t } = useLocale();
  const busy = useRef(false);
  const close = () => {
    if (!busy.current) onClose();
  };

  return (
    <Modal
      open
      kind="form"
      title={t('account.addAnother')}
      description={t('account.addAnotherHelp')}
      onOpenChange={(open) => { if (!open) close(); }}
      footer={<></>}
    >
      <CreateAccountForm
        submitLabel={t('account.create')}
        onCreated={onCreated}
        onBusyChange={(pending) => { busy.current = pending; }}
        onCancel={close}
      />
    </Modal>
  );
}

export default function AccountSettings() {
  const { t } = useLocale();
  usePageLocale();
  const account = useAccount();
  const reportError = usePageError('AccountSettings');
  const [deleting, setDeleting] = useState(false);
  const [creating, setCreating] = useState(false);

  if (!account) {
    return (
      <div className="account-settings-scroll">
        <div className="settings-content settings-sections account-settings-content">
          <h1 className="account-settings-title">{t('account.title')}</h1>
          <p className="account-settings-help">{t('account.unavailable')}</p>
        </div>
      </div>
    );
  }
  const { state, refresh } = account;

  if (state.mode === 'no_accounts') {
    return (
      <div className="account-settings-scroll">
        <div className="settings-content settings-sections account-settings-content">
          <h1 className="account-settings-title">{t('account.title')}</h1>
          <section className="account-settings-section" aria-labelledby="account-first-title">
            <header className="account-section-heading">
              <h2 id="account-first-title">{t('account.first')}</h2>
              <p>{t('account.firstHelp')}</p>
            </header>
            <div className="account-create-form">
              <CreateAccountForm submitLabel={t('account.create')} onCreated={reloadApp} />
            </div>
          </section>
        </div>
      </div>
    );
  }
  if (state.mode !== 'signed_in' || !state.current) return null;

  return (
    <div className="account-settings-scroll">
      <div className="settings-content settings-sections account-settings-content">
        <h1 className="account-settings-title">{t('account.title')}</h1>
        <section className="account-current-summary" aria-labelledby="account-current-title">
          <span className="account-current-mark" aria-hidden="true">{Array.from(state.current.username)[0]?.toLocaleUpperCase()}</span>
          <div className="account-current-identity">
            <h2 id="account-current-title">{t('account.current')}</h2>
            <p className="account-current-username">{state.current.username}</p>
            <div className="account-current-meta">
              <span>{t('account.localAccount')}</span>
              <span className="account-signed-in">{t('account.signedIn')}</span>
            </div>
          </div>
        </section>
        <ChangePasswordSection />
        <section className="account-settings-section account-action-section" aria-labelledby="account-create-title">
          <header className="account-section-heading">
            <h2 id="account-create-title">{t('account.addAnother')}</h2>
            <p>{t('account.addAnotherHelp')}</p>
          </header>
          <Button variant="secondary" onClick={() => setCreating(true)}>{t('account.addAnother')}</Button>
        </section>
        <section className="account-settings-section account-action-section account-delete-section" aria-labelledby="account-delete-title">
          <header className="account-section-heading">
            <h2 id="account-delete-title">{t('account.deleteAccount')}</h2>
            <p>{t('account.deleteHelp')}</p>
          </header>
          <Button variant="ghost" className="account-delete-trigger" onClick={() => setDeleting(true)}>
            {t('account.deleteAccount')}
          </Button>
        </section>
        {creating ? (
          <CreateAccountDialog
            onClose={() => setCreating(false)}
            onCreated={() => {
              setCreating(false);
              toast.success(t('account.created'));
              refresh().catch(reportError);
            }}
          />
        ) : null}
        {deleting ? (
          <DeleteAccountDialog
            username={state.current.username}
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
