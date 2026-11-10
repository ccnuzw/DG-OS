import { useRef, useState } from 'react';
import { Button, Modal } from '@dgos/ui';
import { ImagePlus, X } from 'lucide-react';
import { useLocale } from '../../i18n';
import { addAgentImages, removeAgentImage, useAgentImages } from '../../stores/agentImages';

/** Explicitly selected private draft bytes; never accepts paths or remote URLs. */
export function AgentImages({ owner, disabled = false, active = true }: { owner: string; disabled?: boolean; active?: boolean }) {
  const { t } = useLocale();
  const picker = useRef<HTMLInputElement>(null);
  const draft = useAgentImages(state => state.drafts[owner]);
  const pickerOwner = useRef(owner);
  const [preview, setPreview] = useState<{ owner: string; id: string } | null>(null);
  const images = draft?.images ?? [];
  const selected = active && preview?.owner === owner ? images.find(image => image.id === preview.id) : undefined;
  const busy = Boolean(draft?.busy);
  return <section aria-label={t('agentImages.title')} aria-busy={busy} className="flex min-w-0 flex-wrap items-center gap-2" data-testid="agent-image-list">
    <input ref={picker} data-testid="agent-image-picker" type="file" accept="image/png,image/jpeg,image/webp" multiple hidden disabled={!active || disabled || busy || images.length >= 4} onClick={() => { pickerOwner.current = owner; }} onChange={event => {
      const files = Array.from(event.target.files ?? []);
      event.target.value = '';
      if (active && pickerOwner.current === owner && !disabled && !busy) void addAgentImages(owner, files);
    }} />
    <Button type="button" size="xs" variant="ghost" data-testid="agent-image-add" disabled={!active || disabled || busy || images.length >= 4} onClick={() => { pickerOwner.current = owner; picker.current?.click(); }} title={t('agentImages.limits')}><ImagePlus size={16} aria-hidden="true" />{t('agentImages.add')}</Button>
    {images.map((image, index) => <div key={image.id} className="flex min-w-0 max-w-full items-center gap-1 rounded border border-border-subtle px-1 py-1">
      <Button type="button" size="xs" variant="ghost" data-testid={`agent-image-preview-${index}`} disabled={!active} aria-label={t('agentImages.preview', { name: image.name })} onClick={() => setPreview({ owner, id: image.id })}>
        <img src={`data:${image.input.mime};base64,${image.input.base64}`} alt="" className="h-8 w-8 shrink-0 rounded object-cover" />
        <span className="max-w-32 truncate">{image.name}</span><span className="text-xs text-ink-secondary">{t('agentImages.size', { kib: Math.ceil(image.bytes / 1024) })}</span>
      </Button>
      <Button type="button" size="xs" variant="ghost" iconOnly data-testid={`agent-image-remove-${index}`} disabled={!active || disabled || busy} aria-label={t('agentImages.remove', { name: image.name })} title={t('agentImages.remove', { name: image.name })} onClick={() => removeAgentImage(owner, image.id)}><X size={14} aria-hidden="true" /></Button>
    </div>)}
    {busy && <p role="status" className="m-0 text-xs text-ink-secondary">{t('agentImages.reading')}</p>}
    {draft?.error && <p role="alert" className="m-0 w-full text-sm text-status-error">{t(`agentImages.error.${draft.error}`)}</p>}
    <Modal kind="alert" title={selected?.name ?? t('agentImages.title')} open={Boolean(selected)} onOpenChange={open => { if (!open) setPreview(null); }}>
      {selected && <div data-testid="agent-image-dialog"><img src={`data:${selected.input.mime};base64,${selected.input.base64}`} alt={selected.name} className="max-h-96 max-w-full object-contain" /><p className="text-xs text-ink-secondary">{t('agentImages.privacy')}</p></div>}
    </Modal>
  </section>;
}
