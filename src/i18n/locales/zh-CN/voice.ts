export default {
  "voice.title": "语音输入",
  "voice.help": "最多录制60秒、25 MiB。点击“转写到草稿”后才将音频发送到所选服务。转写先进入可编辑草稿，不自动发送；音频不保存到聊天历史。",
  "voice.provider": "转写服务",
  "voice.model": "转写模型",
  "voice.chooseProvider": "选择服务",
  "voice.chooseModel": "选择转写模型",
  "voice.record": "开始录音",
  "voice.stop": "结束录音",
  "voice.transcribe": "转写到草稿",
  "voice.playback": "录音回放",
  "voice.permission": "等待麦克风授权…",
  "voice.recording": "正在录音 · 最长60秒",
  "voice.transcribing": "正在转写…",
  "voice.error": "转写服务不可用，请检查所选服务和模型。",
  "voice.permissionError": "无法录音，请检查麦克风权限和设备。",
  "voice.limit": "录音超过25 MiB，已丢弃。",
  "voice.unknown": "转写未完成，结果未知，服务可能已计费。音频仍可预览，不会自动重试。"
} as const;
