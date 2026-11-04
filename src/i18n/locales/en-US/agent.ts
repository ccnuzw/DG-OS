export default {
  'agent.engine.codex': 'Codex', 'agent.engine.claude': 'Claude',
  'agent.you': 'You', 'agent.assistant': 'Agent', 'agent.finished': 'Turn result', 'agent.activity': 'Tools and progress',
  'agent.truncated': 'Only recent conversation content is shown. The local agent retains the full history.',
  'agent.title': 'Local agent sessions', 'agent.subtitle': 'Codex and Claude run independently with their own context and tools.',
  'agent.engine': 'Agent', 'agent.workspace': 'Working directory', 'agent.settings': 'Runtime settings', 'agent.executable': 'Absolute executable path (optional)', 'agent.model': 'Model (optional; uses native configuration)',
  'agent.codexPolicy': 'Codex: workspace writes with native approval policy.', 'agent.claudePolicy': 'Claude: native project settings and tool permission policy.',
  'agent.trust': 'I trust the selected directory and directories of sessions I resume, including their instructions, hooks, skills and MCP configuration.',
  'agent.new': 'New session', 'agent.sessions': 'Saved sessions', 'agent.messages': 'Conversation', 'agent.details': 'Details', 'agent.empty': 'Send a message to begin.', 'agent.choose': 'Choose a directory to create a session, or resume a saved session.',
  'agent.prompt': 'Message', 'agent.send': 'Send', 'agent.interrupt': 'Interrupt task', 'agent.close': 'Close runtime', 'agent.approval': 'Pending request', 'agent.allow': 'Allow once', 'agent.deny': 'Deny', 'agent.answer': 'Submit answer',
  'agent.failure': 'The operation did not complete. Check the local agent installation, authentication and configuration. Verify uncertain results before submitting again.',
  'agent.state.starting': 'Connecting', 'agent.state.idle': 'Ready', 'agent.state.running': 'Running', 'agent.state.waiting': 'Waiting for you', 'agent.state.closed': 'Closed; can resume', 'agent.state.error': 'Failed', 'agent.state.unknown': 'Disconnected; verify outcome',
} as const;
