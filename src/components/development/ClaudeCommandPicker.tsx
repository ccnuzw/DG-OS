import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Button, Popover } from '@dgos/ui';
import { Terminal } from 'lucide-react';
import { useLocale } from '../../i18n';
import { useAgentRunner, type AgentSession } from '../../stores/agentRunnerStore';
import { useDevelopment } from '../../stores/developmentStore';
import { contextGeneration, useContextSelection } from '../../stores/developmentSelection';
import type { NativeObservedSkill } from '../../types/nativeExtensions';

type Observation = { session_id: string; generation: string; engine: string; domain: string; incomplete: boolean; skills: NativeObservedSkill[] };
type Props = { projectId: string | null; session?: AgentSession; disabled: boolean; active: boolean; onCommandSelect?: (command: string) => void };

// SDK name rows take precedence over aliases. Builtin precedence applies only
// to actual names; aliases shared by multiple rows have no verified target.
function commandReason(commands: NativeObservedSkill[], command: NativeObservedSkill, name: string) {
  const exact = commands.filter(row => row.name === name);
  if (exact.some(row => row.builtin === true || row.classification === 'builtin_command')) return 'agentSkills.commands.builtin';
  const matches = exact.length ? exact : commands.filter(row => row.aliases?.includes(name));
  if (matches.length !== 1 || matches[0] !== command) return 'agentSkills.commands.ambiguous';
  if (command.builtin === true || command.classification === 'builtin_command') return 'agentSkills.commands.builtin';
  return null;
}

export function ClaudeCommandPicker({ projectId, session, disabled, active, onCommandSelect }: Props) {
  const { t } = useLocale();
  const selectDraft = useRef(onCommandSelect); selectDraft.current = onCommandSelect;
  const epoch = useDevelopment(state => state.epoch);
  const { generation, pending } = useContextSelection();
  const owner = JSON.stringify([epoch, generation, projectId, session?.id, session?.generation, session?.status, session?.active_run, session?.pending, active, pending, disabled]);
  const latest = useRef(owner); latest.current = owner;
  const mounted = useRef(true);
  const sequence = useRef(0);
  const selected = useRef(false);
  const [panel, setPanel] = useState<string | null>(null);
  const [read, setRead] = useState<{ owner: string; commands: NativeObservedSkill[]; error: boolean; changed?: boolean } | null>(null);
  const [loading, setLoading] = useState<string | null>(null);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; sequence.current += 1; }; }, []);
  useEffect(() => { sequence.current += 1; setPanel(null); setRead(null); setLoading(null); }, [owner]);
  const writable = active && !pending && !disabled && session?.engine === 'claude' && session.status === 'idle' && !session.active_run && !session.pending.length;

  function current() {
    const runner = useAgentRunner.getState(), live = runner.session;
    return mounted.current && latest.current === owner && writable && session && epoch === useDevelopment.getState().epoch && generation === contextGeneration() && runner.selected === session.id && live?.id === session.id && live.generation === session.generation && live.engine === 'claude' && !runner.busy && !runner.acting && live.status === 'idle' && !live.active_run && !live.pending.length;
  }
  async function observe(selection?: { command: NativeObservedSkill; name: string; commands: NativeObservedSkill[] }) {
    if (!current()) return;
    const token = ++sequence.current;
    setRead(null); setLoading(owner);
    try {
      const value = await invoke<Observation>('observe_native_session', { input: { session_id: session!.id, generation: session!.generation, domain: 'skills' } });
      if (!current() || sequence.current !== token) return;
      if (value.session_id !== session!.id || value.generation !== session!.generation || value.engine !== 'claude' || value.domain !== 'skills' || value.incomplete) {
        setRead({ owner, commands: [], error: true });
        return;
      }
      setRead({ owner, commands: value.skills, error: false });
      if (selection) {
        const target = value.skills.find(command => command.name === selection.command.name && command.builtin === selection.command.builtin);
        if (JSON.stringify(value.skills) !== JSON.stringify(selection.commands) || !target || commandReason(value.skills, target, selection.name)) {
          setRead({ owner, commands: value.skills, error: false, changed: true });
          return;
        }
        selected.current = true;
        setPanel(null);
        // Only the draft changes. Sending remains a separate explicit action.
        selectDraft.current?.(`/${selection.name}`);
      }
    } catch {
      if (current() && sequence.current === token) setRead({ owner, commands: [], error: true });
    } finally {
      if (mounted.current && latest.current === owner && sequence.current === token) setLoading(null);
    }
  }
  function choose(command: NativeObservedSkill, name: string) {
    if (!current() || !onCommandSelect || loading === owner || read?.owner !== owner || read.error || !read.commands.includes(command) || commandReason(read.commands, command, name)) return;
    // supportedCommands() follows SDK commands_changed pushes. Re-read through
    // the existing observation path before using a previously displayed token.
    void observe({ command, name, commands: read.commands });
  }
  return <Popover
    label={t('agentSkills.commands.title')}
    side="top"
    align="start"
    open={active && !pending && panel === owner}
    onOpenChange={open => {
      setPanel(open ? owner : null);
      if (open) { selected.current = false; void observe(); }
      else { sequence.current += 1; setLoading(null); }
    }}
    onCloseAutoFocus={event => { if (selected.current || !current()) event.preventDefault(); }}
    trigger={<Button size="xs" variant="ghost" disabled={!active || pending} aria-label={t('agentSkills.commands.title')}><Terminal size={14} aria-hidden="true" /><span>{t('agentSkills.commands.title')}</span></Button>}
    content={<div className="agent-control-popover">
      <h3>{t('agentSkills.commands.title')}</h3>
      <p>{t(session ? 'agentSkills.commands.boundary' : 'agentSkills.commands.beforeStart')}</p>
      {session ? <>
        <p>{t('agentSkills.commands.sourceUnknown')}</p>
        {!onCommandSelect && <p>{t('agentSkills.commands.selectionUnavailable')}</p>}
        <Button size="xs" variant="secondary" disabled={!writable || loading === owner} onClick={() => void observe()}>{t('agentSkills.commands.refresh')}</Button>
        {read?.owner === owner && read.changed && <p role="alert">{t('agentSkills.commands.changed')}</p>}
        {loading === owner ? <p role="status">{t('agentSkills.commands.loading')}</p> : read?.owner === owner ? read.error ? <p role="alert">{t('agentSkills.commands.unavailable')}</p> : !read.commands.length ? <p>{t('agentSkills.commands.empty')}</p> : <div className="flex min-w-0 flex-col gap-2" role="group" aria-label={t('agentSkills.commands.available')}>
          {read.commands.flatMap((command, index) => [...new Set([command.name, ...(command.aliases ?? [])])].map(name => {
            const reason = commandReason(read.commands, command, name);
            return <div key={JSON.stringify([command.name, index, name])} className="flex min-w-0 flex-col gap-1">
              <Button size="xs" variant="ghost" disabled={!writable || !onCommandSelect || Boolean(reason)} onClick={() => choose(command, name)} aria-label={`/${name}`}><span className="break-all whitespace-normal">/{name}</span></Button>
              <small className="text-ink-secondary">{name !== command.name && <>{t('agentSkills.commands.aliasOf', { name: `/${command.name}` })} · </>}{t(reason ?? 'agentSkills.commands.addToDraft')}</small>
            </div>;
          }))}
        </div> : null}
      </> : null}
    </div>}
  />;
}
