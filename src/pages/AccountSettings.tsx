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

// V1-FR-008 account page. Which user is signed in is the host's knowledge; this page sends passwords only and
// never a user id or token. Changing the password and deleting the account end the session on the host, so both
// finish with a full reload (the gate then shows the sign-in page).

function ChangePasswordCard() {
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
      current: current === '' ? '请输入当前密码' : undefined,
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
      if (error instanceof AccountCommandError && error.code === 1008005) setFormError('当前密码错误');
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
        <h2 className="m-0 text-base font-semibold">修改密码</h2>
      </Card.Header>
      <Card.Body>
        <form onSubmit={submit} noValidate>
          <Stack gap={4}>
            <p className="m-0 text-muted-foreground">修改成功后会退出登录，请用新密码重新登录。</p>
            <Input
              label="当前密码"
              type="password"
              autoComplete="current-password"
              value={current}
              error={errors.current}
              onChange={(event) => setCurrent(event.target.value)}
            />
            <Input
              label="新密码"
              type="password"
              autoComplete="new-password"
              value={next}
              error={errors.next}
              onChange={(event) => setNext(event.target.value)}
            />
            <Input
              label="确认新密码"
              type="password"
              autoComplete="new-password"
              value={confirm}
              error={errors.confirm}
              onChange={(event) => setConfirm(event.target.value)}
            />
            {formError ? <InlineError>{formError}</InlineError> : null}
            <div>
              <Button type="submit" loading={busy}>
                修改密码
              </Button>
            </div>
          </Stack>
        </form>
      </Card.Body>
    </Card>
  );
}

function DeleteAccountDialog({ onClose, onSessionInvalid }: { onClose: () => void; onSessionInvalid: () => void }) {
  const reportError = usePageError('AccountSettings');
  const [password, setPassword] = useState('');
  const [error, setError] = useState<string>();
  const [busy, setBusy] = useState(false);

  const submit = async (event?: FormEvent<HTMLFormElement>) => {
    event?.preventDefault();
    if (busy) return;
    if (password === '') {
      setError('请输入密码以确认删除');
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
      title="删除账户？"
      icon="warning"
      description="此账户的全部数据将被永久删除，且无法恢复。"
      footer={
        <>
          <Button variant="secondary" disabled={busy} onClick={onClose}>
            取消
          </Button>
          <Button variant="danger" loading={busy} onClick={() => void submit()}>
            确认删除
          </Button>
        </>
      }
    >
      <form onSubmit={submit} noValidate>
        <Stack gap={3}>
          <Input
            label="当前密码"
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
  const account = useAccount();
  const reportError = usePageError('AccountSettings');
  const [deleting, setDeleting] = useState(false);

  if (!account) {
    return (
      <div className="mx-auto flex max-w-2xl flex-col gap-6 p-6">
        <h1 className="m-0 text-2xl font-semibold text-foreground">账户</h1>
        <p className="m-0 text-sm text-muted-foreground">账户功能当前不可用。</p>
      </div>
    );
  }
  const { state, refresh } = account;

  if (state.mode === 'no_accounts') {
    return (
      <div className="h-full overflow-y-auto">
        <div className="mx-auto flex max-w-2xl flex-col gap-6 p-6">
          <h1 className="m-0 text-2xl font-semibold text-foreground">账户</h1>
          <Card>
            <Card.Header>
              <h2 className="m-0 text-base font-semibold">创建第一个账户</h2>
            </Card.Header>
            <Card.Body>
              <Stack gap={4}>
                <p className="m-0 text-muted-foreground">
                  创建账户会为 DGOS 增加密码保护。这台设备上已有的本地数据将归入你创建的第一个账户。
                </p>
                <CreateAccountForm submitLabel="创建账户" onCreated={reloadApp} />
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
        <h1 className="m-0 text-2xl font-semibold text-foreground">账户</h1>
        <Card>
          <Card.Header>
            <h2 className="m-0 text-base font-semibold">当前账户</h2>
          </Card.Header>
          <Card.Body>
            <p className="m-0">
              当前登录：<strong>{state.current.username}</strong>
            </p>
          </Card.Body>
        </Card>
        <Card>
          <Card.Header>
            <h2 className="m-0 text-base font-semibold">新建账户</h2>
          </Card.Header>
          <Card.Body>
            <CreateAccountForm
              submitLabel="新建账户"
              onCreated={() => {
                toast.success('账户已创建');
                refresh().catch(reportError);
              }}
            />
          </Card.Body>
        </Card>
        <ChangePasswordCard />
        <Card>
          <Card.Header>
            <h2 className="m-0 text-base font-semibold text-error-text">删除账户</h2>
          </Card.Header>
          <Card.Body>
            <Stack gap={4}>
              <p className="m-0 text-muted-foreground">删除后，此账户的全部数据将被永久删除，且无法恢复。</p>
              <div>
                <Button variant="danger" onClick={() => setDeleting(true)}>
                  删除账户
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
