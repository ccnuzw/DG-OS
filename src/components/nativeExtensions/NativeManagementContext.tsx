import { createContext, useContext } from 'react';
import type { NativeSelection } from '../../types/nativeExtensions';

export interface NativeManagementEntry {
  kind: 'skills' | 'mcp';
  selection: NativeSelection;
  sessionId?: string;
}

// Presentation-only routing inside the resident CLI workbench. Host commands
// independently authorize the account, project and exact workspace.
export const NativeManagementContext = createContext<((entry: NativeManagementEntry) => void) | null>(null);
export const useNativeManagement = () => useContext(NativeManagementContext);
