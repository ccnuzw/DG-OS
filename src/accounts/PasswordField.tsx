import { useId, useState } from 'react';
import { Eye, EyeOff } from 'lucide-react';
import { Button, Input, type SingleLineInputProps } from '@dgos/ui';
import { useLocale } from '../i18n';
import './password-field.css';

type PasswordFieldProps = Omit<SingleLineInputProps, 'type' | 'multiline' | 'label' | 'aria-label' | 'aria-labelledby'> & {
  label: string;
};

export function PasswordField({ label, id, ...props }: PasswordFieldProps) {
  const { t } = useLocale();
  const autoId = useId();
  const inputId = id ?? autoId;
  const [visible, setVisible] = useState(false);

  return (
    <div className="account-password-field">
      <Input {...props} id={inputId} label={label} type={visible ? 'text' : 'password'} />
      <Button
        type="button"
        variant="secondary"
        iconOnly
        className="col-start-2 row-start-2"
        aria-label={t(visible ? 'account.hidePassword' : 'account.showPassword', { field: label })}
        aria-controls={inputId}
        aria-pressed={visible}
        onClick={() => setVisible((previous) => !previous)}
      >
        {visible ? <EyeOff size={16} aria-hidden="true" /> : <Eye size={16} aria-hidden="true" />}
      </Button>
    </div>
  );
}
