import { useEffect, useRef, useState } from 'react';
import { Button, Icon, Select } from '@dgos/ui';
import { useLocale } from '../../i18n';
import { useChatStore } from '../../stores/chatStore';
import type { Model, Provider } from '../../stores/providerStore';
import { assetCall, fileBase64 } from './Attachments';

type Phase = 'idle' | 'permission' | 'recording' | 'ready' | 'transcribing';
export default function VoiceCapture({ sessionId, disabled, onBusy, onClose }: { sessionId: string; disabled: boolean; onBusy: (busy: boolean) => void; onClose: () => void }) {
  const { t } = useLocale();
  const [providers, setProviders] = useState<Provider[]>([]);
  const [models, setModels] = useState<Model[]>([]);
  const [providerId, setProviderId] = useState('');
  const [modelId, setModelId] = useState('');
  const [phase, setPhase] = useState<Phase>('idle');
  const [audio, setAudio] = useState<Blob | null>(null);
  const [url, setUrl] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const version = useRef(0);
  const currentPhase = useRef<Phase>('idle');
  const recorder = useRef<MediaRecorder | null>(null);
  const stream = useRef<MediaStream | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const pending = useRef<string | null>(null);
  const stopTracks = () => { if (timer.current) clearTimeout(timer.current); stream.current?.getTracks().forEach(track => track.stop()); stream.current = null; };
  const changePhase = (value: Phase) => { currentPhase.current = value; setPhase(value); onBusy(value !== 'idle' && value !== 'ready'); };
  const release = () => {
    version.current++;
    if (recorder.current?.state === 'recording') recorder.current.stop();
    stopTracks(); recorder.current = null;
    if (pending.current) void assetCall('cancel_chat_transcription', { requestId: pending.current }).catch(() => undefined);
    pending.current = null;
  };
  useEffect(() => {
    const unsubscribe = useChatStore.subscribe(state => {
      if (state.activeSessionId === sessionId && state.sessions.some(session => session.id === sessionId)) return;
      release(); setAudio(null); changePhase('idle'); onClose();
    });
    return () => { unsubscribe(); release(); onBusy(false); };
  }, [sessionId]);
  useEffect(() => {
    if (!disabled || currentPhase.current === 'idle' || currentPhase.current === 'ready') return;
    release(); changePhase(audio ? 'ready' : 'idle');
  }, [disabled]);
  useEffect(() => { if (!audio) { setUrl(null); return; } const objectUrl = URL.createObjectURL(audio); setUrl(objectUrl); return () => URL.revokeObjectURL(objectUrl); }, [audio]);
  async function load() {
    setError(null);
    const token = ++version.current;
    try {
      const list = await assetCall<Provider[]>('list_providers', {});
      if (token === version.current) setProviders(list.filter(provider => provider.enabled && provider.credential_status === 'stored' && provider.provider_type === 'openai_compatible'));
    } catch { if (token === version.current) setError(t('voice.error')); }
  }
  useEffect(() => { void load(); }, []);
  async function selectProvider(id: string) {
    release(); changePhase('idle'); setAudio(null); setProviderId(id); setModelId(''); setModels([]);
    const token = version.current;
    try {
      const list = await assetCall<Model[]>('list_models', { providerId: id });
      if (token === version.current) setModels(list.filter(model => model.model_type === 'speech-transcription'));
    } catch { if (token === version.current) setError(t('voice.error')); }
  }
  async function record() {
    if (disabled || !providerId || !modelId || currentPhase.current !== 'idle') return;
    if (!navigator.mediaDevices?.getUserMedia) { setError(t('voice.error')); return; }
    const token = ++version.current; setError(null); setAudio(null); changePhase('permission');
    try {
      const microphone = await navigator.mediaDevices.getUserMedia({ audio: true });
      if (token !== version.current) { microphone.getTracks().forEach(track => track.stop()); return; }
      stream.current = microphone;
      const mime = ['audio/webm', 'audio/mp4', 'audio/ogg'].find(type => MediaRecorder.isTypeSupported(type));
      if (!mime) throw new Error('recording_unsupported');
      const capture = new MediaRecorder(microphone, { mimeType: mime });
      recorder.current = capture;
      const chunks: Blob[] = []; let size = 0;
      capture.ondataavailable = event => {
        if (token !== version.current) return;
        size += event.data.size;
        if (size > 25 * 1024 * 1024) { release(); setAudio(null); changePhase('idle'); setError(t('voice.limit')); return; }
        chunks.push(event.data);
      };
      capture.onerror = () => { if (token === version.current) { release(); changePhase('idle'); setError(t('voice.error')); } };
      capture.onstop = () => {
        if (token !== version.current) return;
        stopTracks();
        if (!size) { changePhase('idle'); setError(t('voice.error')); return; }
        setAudio(new Blob(chunks, { type: capture.mimeType })); changePhase('ready');
      };
      capture.start(500); changePhase('recording');
      timer.current = setTimeout(() => { if (capture.state === 'recording') capture.stop(); }, 60_000);
    } catch { if (token === version.current) { stopTracks(); changePhase('idle'); setError(t('voice.permissionError')); } }
  }
  async function transcribe() {
    if (disabled || !audio || currentPhase.current !== 'ready') return;
    const token = ++version.current; const requestId = crypto.randomUUID(); pending.current = requestId;
    changePhase('transcribing'); setError(null);
    try {
      const base64 = await fileBase64(audio);
      if (token !== version.current) return;
      const result = await assetCall<{ text: string }>('transcribe_chat_audio', { input: { request_id: requestId, provider_id: providerId, model_id: modelId, mime: audio.type, base64 } });
      if (token !== version.current || !useChatStore.getState().sessions.some(session => session.id === sessionId)) return;
      const state = useChatStore.getState();
      state.setDraft(sessionId, [state.sessionDrafts[sessionId], result.text].filter(Boolean).join('\n'));
      setAudio(null); changePhase('idle'); onClose();
    } catch { if (token === version.current) { setError(t('voice.unknown')); changePhase('ready'); } }
    finally { if (token === version.current) pending.current = null; }
  }
  const locked = phase !== 'idle';
  return <section aria-label={t('voice.title')} className="chat-voice-menu">
      <header className="chat-picker-header"><strong>{t('voice.title')}</strong><Button size="xxs" variant="ghost" iconOnly aria-label={t('common.close')} title={t('common.close')} onClick={onClose}><Icon name="close" size="sm" /></Button></header>
      <p className="chat-picker-help">{t('voice.help')}</p>
      <Select label={t('voice.provider')} placeholder={t('voice.chooseProvider')} value={providerId} onChange={id => void selectProvider(id)} disabled={disabled || locked} options={providers.map(provider => ({ value: provider.id, label: `${provider.name} · ${new URL(provider.base_url).host}` }))} />
      <Select label={t('voice.model')} placeholder={t('voice.chooseModel')} value={modelId} onChange={setModelId} disabled={disabled || locked} options={models.map(model => ({ value: model.model_id, label: model.model_name }))} />
      <div className="chat-voice-actions">{phase === 'idle' && <Button size="sm" disabled={disabled || !providerId || !modelId} onClick={() => void record()}>{t('voice.record')}</Button>}
        {phase === 'recording' && <Button size="sm" onClick={() => recorder.current?.stop()}>{t('voice.stop')}</Button>}
        {phase === 'ready' && <Button size="sm" disabled={disabled} onClick={() => void transcribe()}>{t('voice.transcribe')}</Button>}
        {phase !== 'idle' && <Button size="sm" variant="secondary" onClick={() => { release(); setAudio(null); changePhase('idle'); }}>{t('common.cancel')}</Button>}
      </div>
      {phase === 'permission' || phase === 'recording' || phase === 'transcribing' ? <p role="status" className="chat-voice-status">{t(`voice.${phase}`)}</p> : null}
      {url && <audio controls src={url} aria-label={t('voice.playback')} className="chat-voice-playback" />}
      {error && <p role="alert" className="chat-media-error">{error}</p>}
    </section>
  ;
}
