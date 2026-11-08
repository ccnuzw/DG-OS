export default {
  "voice.title": "Voice input",
  "voice.help": "Record up to 60 seconds / 25 MiB. Audio is sent only after you choose Transcribe, to the selected service. The editable transcript is never sent automatically; audio is not saved in chat history.",
  "voice.provider": "Transcription service",
  "voice.model": "Transcription model",
  "voice.chooseProvider": "Choose service",
  "voice.chooseModel": "Choose transcription model",
  "voice.record": "Record audio",
  "voice.stop": "Finish recording",
  "voice.transcribe": "Transcribe to draft",
  "voice.playback": "Recorded audio playback",
  "voice.permission": "Waiting for microphone permission…",
  "voice.recording": "Recording · maximum 60 seconds",
  "voice.transcribing": "Transcribing…",
  "voice.error": "Transcription service unavailable. Check the selected service and model.",
  "voice.permissionError": "Could not record audio. Check microphone permission and device availability.",
  "voice.limit": "Recording exceeded 25 MiB and was discarded.",
  "voice.unknown": "Transcription did not complete. The result is unknown and the service may have charged. Audio remains available; no automatic retry."
} as const;
