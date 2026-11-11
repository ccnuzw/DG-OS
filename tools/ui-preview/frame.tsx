import { createRoot } from 'react-dom/client';
import { MemoryRouter } from 'react-router-dom';
import App from '../../src/App';
import { AccountContext } from '../../src/accounts/AccountContext';
import { ErrorBoundary, ModalHost, ToastViewport, initAppearance } from '../../src/ui';
import { LayoutScaleFeedback } from '../../src/ui/appearance/LayoutScaleFeedback';
import { setLocaleForTesting } from '../../src/i18n';
import { useChatStore } from '../../src/stores/chatStore';
import ComponentScenes from './ComponentScenes';
import { clock, diagnostics, locale, messages, replyFixture, scenario, sessions } from './host';
import './preview.css';

Date.now = () => clock;
setLocaleForTesting(locale);
await initAppearance();
const session = sessions[0];
const initialMessages = structuredClone(messages);
const view = { messages: initialMessages, stale: false, loaded: true, olderCursor: null, newerTrimmed: false, messagesLoading: false, messagesError: null, droppedTurns: {} };
useChatStore.setState({ sessions, sessionsLoaded: true, openSessionIds: sessions.map(s => s.id), activeSessionId: session.id, activeSession: session, messages: initialMessages, sessionViews: { [session.id]: view }, sessionDrafts: { [session.id]: locale === 'en-US' ? 'Editable preview draft — switch away and return.' : '可编辑预览草稿：切换后返回检查保留。' } });
if (replyFixture || ['chat-approval', 'chat-streaming', 'chat-recovery', 'chat-context'].includes(scenario)) {
  const notice = { tone: 'info' as const, text: locale === 'en-US' ? 'Isolated production-component fixture. No model, MCP tool, credential vault, or external network is used. Sources and approvals are synthetic.' : '真实组件的隔离预览，不调用模型、MCP 工具、凭据库或外部网络；来源与审批均为合成数据。' };
  useChatStore.setState({ notice, sessionNotices: { [session.id]: notice }, sessionUnknown: { [session.id]: replyFixture === 'unknown' } });
}
if (scenario === 'chat-error') {
  const actionError = locale === 'en-US' ? 'Preview failure: no request was sent. Draft retained.' : '预览失败：未发送请求，草稿保留。';
  const notice = { tone: 'warning' as const, text: locale === 'en-US' ? 'Result unknown. Confirm before trying again.' : '结果未确认，请核对后再试。' };
  useChatStore.setState({ actionError, notice, sessionActionErrors: { [session.id]: actionError }, sessionNotices: { [session.id]: notice } });
}
if (scenario === 'chat-approval' || scenario === 'chat-streaming' || scenario === 'chat-recovery') await useChatStore.getState().send(locale === 'en-US' ? 'Run the isolated preview stream.' : '运行隔离预览消息。');
const appScene = scenario.startsWith('settings') || scenario.startsWith('chat');
createRoot(document.getElementById('root')!).render(<ErrorBoundary><MemoryRouter initialEntries={[scenario === 'settings-appearance' ? '/settings/appearance' : scenario.startsWith('settings') ? '/settings/cli' : '/workbench']} future={{v7_startTransition:true,v7_relativeSplatPath:true}}><AccountContext.Provider value={{ state: { mode: 'signed_in', current: { username: 'preview-only', last_sign_in_at: null }, accounts: [{ username: 'preview-only', last_sign_in_at: null }], preselected: null }, refresh: async () => {} }}>{appScene ? <App/> : <main className="preview-components"><ComponentScenes scenario={scenario} locale={locale}/></main>}</AccountContext.Provider><ModalHost/><ToastViewport/><LayoutScaleFeedback/></MemoryRouter></ErrorBoundary>);
const mounted = () => {
  const selector = scenario === 'settings-appearance' ? '.appearance-editor input:not(:disabled)' : scenario.startsWith('settings') ? '.cli-settings input:not(:disabled)' : scenario.startsWith('chat') ? '.chat-composer textarea' : '[data-preview-controls], [data-preview-patterns]';
  if (document.querySelector(selector) || (scenario.startsWith('settings') && document.querySelector('.cli-feedback-error'))) { diagnostics.ready = true; return; }
  requestAnimationFrame(mounted);
};
requestAnimationFrame(mounted);
