export default {
  'commitDraft.title': 'Generate commit message',
  'commitDraft.generate': 'Send diff and generate',
  'commitDraft.disclosure': 'Send the staged diff below to this project’s configured model. The result only fills the commit draft; it does not commit code or send a conversation message.',
  'commitDraft.diffLimit': 'Stage changes first. Generation accepts up to 64 KiB; reduce the commit scope.',
  'commitDraft.providerRequired': 'Configure a Provider and model for this project first.',
  'commitDraft.failed': 'Commit message generation failed ({code}). No automatic retry was made. Check the model configuration before trying again.',
} as const;
