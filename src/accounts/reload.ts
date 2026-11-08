import { useChatStore } from '../stores/chatStore';
import { useTaskRunner } from '../stores/taskRunnerStore';
import { useAgentRunner } from '../stores/agentRunnerStore';
import { useDevelopment } from '../stores/developmentStore';
import { moduleRuntimeHost } from '../modules/moduleRuntimeHost';
import { useDesktopPreferences } from '../workspace/desktopPreferences';
import { disposeAppearance } from '../ui/appearance';
// A full page load is how a session change reaches every page and the module host (V1-FR-008): each of them
// re-reads the current user's data on start. Kept in its own module so tests can replace it.
export function reloadApp(): void {
  disposeAppearance();
  useChatStore.getState().reset();
  useTaskRunner.getState().reset();
  useDevelopment.getState().reset();
  useAgentRunner.getState().reset();
  useDesktopPreferences.getState().reset();
  moduleRuntimeHost.resetViews();
  window.location.reload();
}
