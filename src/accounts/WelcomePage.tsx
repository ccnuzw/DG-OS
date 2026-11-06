import { Button } from '@dgos/ui';
import { AuthShell } from './AuthShell';
import { CreateAccountForm } from './CreateAccountForm';
import { reloadApp } from './reload';
import { useLocale } from '../i18n';

// First launch (`no_accounts`). Until the first account exists the host runs as the default user without a gate
// (D087), so the page offers to continue without one; that choice is not stored and lasts for this page load.
export function WelcomePage({ onSkip }: { onSkip: () => void }) {
  const { t } = useLocale();
  return (
    <AuthShell title={t('auth.welcome')}>
      <p className="m-0 text-sm text-ink-secondary">
        {t('auth.welcomeText')}
      </p>
      <CreateAccountForm submitLabel={t('auth.create')} onCreated={reloadApp} />
      <div>
        <Button variant="ghost" onClick={onSkip}>
          {t('auth.skip')}
        </Button>
      </div>
    </AuthShell>
  );
}
