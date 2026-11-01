import { useEffect, useRef, useState, type FormEvent } from 'react';
import { Button } from '@dgos/ui/components/Button';
import type { LanApi, LanSessionStatus } from './api';
import { TEXT_INPUT } from './classes';
import { ACCESS_CODE_LENGTH, normalizeAccessCode } from './ids';
import { pairErrorText } from './messages';

export interface PairingViewProps {
  api: LanApi;
  /** Code read from the URL fragment; prefilled. */
  initialCode?: string;
  /** Submit `initialCode` once on mount. */
  autoSubmit?: boolean;
  /** Why the user is back here (disconnected, expired, ...). */
  notice?: string;
  onPaired: (status: LanSessionStatus) => void;
}

export function PairingView({ api, initialCode = '', autoSubmit = false, notice, onPaired }: PairingViewProps) {
  const [value, setValue] = useState(initialCode);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const autoSubmitted = useRef(false);

  async function submit(raw: string) {
    const code = normalizeAccessCode(raw);
    if (code.length !== ACCESS_CODE_LENGTH) {
      setError(`访问凭据应为 ${ACCESS_CODE_LENGTH} 位字符`);
      return;
    }
    setBusy(true);
    setError('');
    try {
      onPaired(await api.pair(code));
    } catch (failure) {
      setError(pairErrorText(failure));
      setBusy(false);
    }
  }

  useEffect(() => {
    // The ref survives React's dev-mode effect replay, so a code is submitted once: pairing codes are single use.
    if (!autoSubmit || autoSubmitted.current) return;
    autoSubmitted.current = true;
    void submit(initialCode);
  }, []);

  function onSubmit(event: FormEvent) {
    event.preventDefault();
    if (!busy) void submit(value);
  }

  return (
    <main className="mx-auto flex w-full max-w-md flex-1 flex-col justify-center gap-6 p-6">
      <header className="flex flex-col gap-2">
        <h1 className="text-2xl font-semibold">连接到 DG-OS</h1>
        <p className="text-muted-foreground">在主机的“设置 → 局域网协同”中生成访问凭据，输入后即可在本设备上聊天。</p>
      </header>
      {notice ? (
        <p role="status" className="rounded-md bg-warning-bg px-3 py-2 text-warning-text">
          {notice}
        </p>
      ) : null}
      <form className="flex flex-col gap-4" onSubmit={onSubmit}>
        <div className="flex flex-col gap-1">
          <label htmlFor="lan-access-code" className="text-sm font-medium">
            访问凭据
          </label>
          <input
            id="lan-access-code"
            className={TEXT_INPUT}
            value={value}
            onChange={(event) => setValue(event.target.value)}
            autoComplete="off"
            autoCapitalize="characters"
            autoCorrect="off"
            spellCheck={false}
            inputMode="text"
            placeholder="XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XX"
            disabled={busy}
            aria-invalid={error ? true : undefined}
            aria-describedby={error ? 'lan-access-code-error' : undefined}
          />
          {error ? (
            <p id="lan-access-code-error" role="alert" className="text-sm text-error-text">
              {error}
            </p>
          ) : null}
        </div>
        <Button type="submit" size="lg" loading={busy}>
          连接
        </Button>
      </form>
    </main>
  );
}
