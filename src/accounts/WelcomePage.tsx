import { Button } from '@dgos/ui';
import { AuthShell } from './AuthShell';
import { CreateAccountForm } from './CreateAccountForm';
import { reloadApp } from './reload';

// First launch (`no_accounts`). Until the first account exists the host runs as the default user without a gate
// (D087), so the page offers to continue without one; that choice is not stored and lasts for this page load.
export function WelcomePage({ onSkip }: { onSkip: () => void }) {
  return (
    <AuthShell title="欢迎使用 DGOS">
      <p className="m-0 text-sm text-muted-foreground">
        创建账户会为 DGOS 增加密码保护。这台设备上已有的本地数据将归入你创建的第一个账户。
      </p>
      <CreateAccountForm submitLabel="创建账户" onCreated={reloadApp} />
      <div>
        <Button variant="ghost" onClick={onSkip}>
          暂不创建，继续使用
        </Button>
      </div>
    </AuthShell>
  );
}
