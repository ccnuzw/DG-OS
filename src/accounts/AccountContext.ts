import { createContext, useContext } from 'react';
import type { AccountState } from './accountApi';

export interface AccountContextValue {
  state: AccountState;
  /** Re-reads the account state from the host. Rejects when the read fails. */
  refresh: () => Promise<void>;
}

/** `null` when the app is rendered without `AccountGate` (existing tests, isolated pages). */
export const AccountContext = createContext<AccountContextValue | null>(null);

export function useAccount(): AccountContextValue | null {
  return useContext(AccountContext);
}
