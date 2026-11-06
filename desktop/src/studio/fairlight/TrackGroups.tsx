import { createContext, useContext, useEffect, useRef, useState, type ReactNode } from 'react';
import { Sheet } from '../../app/Sheet';
import { callEngine, type GroupControl, type Op, type Timeline, type TrackGroup } from '../engine';
import type { RoutingContext } from './BusFormat';
import { useFailureText, useFairlightText } from './words';
import { GROUP_CONTROLS, GROUP_DEFAULTS_KEY, allGroup, groupDefaults, groupKey, groupPatch, groupTracks } from './groupModel';
import { NameField } from '../media/NameField';
import { TRACK_COLORS } from './IndexTracks';
import { writing, inStudio } from '../keys';
import { CoveringMenu } from '../color/CoveringMenu';
import './groups.css';
type ContextValue = RoutingContext & {
  selected: readonly string[];
  select: (ids: string[]) => void;
  edit: (ops: Op[], revision?: number) => Promise<boolean>;
  open: (group?: TrackGroup, members?: string[]) => void;
};
const Context = createContext<ContextValue | null>(null);
export const useTrackGroups = () => useContext(Context);
export function TrackGroupsProvider({ timeline, bridge, session, revision, selected, select, children }: RoutingContext & {
  selected: readonly string[];
  select: (ids: string[]) => void;
  children: ReactNode;
}) {
  const t = useFairlightText(), [draft, setDraft] = useState<TrackGroup | null>(null), [problem, setProblem] = useState('');
  const failure = useFailureText();
  const [menu, setMenu] = useState<{
    x: number;
    y: number;
    members: string[];
  } | null>(null);
  const live = useRef(true);
  useEffect(() => {
    live.current = true;
    return () => {
      live.current = false;
    };
  }, []);
  const edit = async (ops: Op[], expected = revision) => {
    try {
      await callEngine(bridge, 'transaction.apply', { revision: expected, ops }, { expectedSession: session });
      return true;
    }
    catch (error) {
      if (live.current)
        setProblem(failure(error));
      return false;
    }
  };
  const open = (group?: TrackGroup, members?: string[]) => {
    setMenu(null);
    setProblem('');
    const ids = (members ?? selected).filter(id => timeline.tracks.some(track => track.id === id && track.kind === 'audio'));
    setDraft(group ? structuredClone(group) : {
      id: '', name: t('New Group'), members: [...new Set(ids)], linked: groupDefaults(), active: true, color: TRACK_COLORS[8]
    });
  };
  useEffect(() => {
    const down = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.repeat || event.isComposing || writing(event.target) || !inStudio(event.target) || document.querySelector('dialog[open]'))
        return;
      const ops = groupKey(timeline, event);
      if (ops.length) {
        event.preventDefault();
        void edit(ops);
      }
    };
    window.addEventListener('keydown', down);
    return () => window.removeEventListener('keydown', down);
  }, [timeline, revision, session]);
  return <Context.Provider value={{
    timeline, bridge, session, revision, selected, select, edit, open
  }}>

    <div className="track-groups-context" onContextMenuCapture={event => {
      const element = event.target instanceof Element ? event.target : null;
      const key = element?.matches('.mixer-strip,.mixer-name,.mixer-strip-color') ? element.closest<HTMLElement>('[data-mixer-strip]')?.dataset.mixerStrip : undefined;
      const id = key?.startsWith('track:') ? key.slice(6) : null;
      if (!id || !timeline.tracks.some(track => track.id === id && track.kind === 'audio') || writing(event.target))
        return;
      event.preventDefault();
      event.stopPropagation();
      setMenu({ x: event.clientX, y: event.clientY, members: selected.includes(id) ? [...selected] : [id] });
    }}>
      {children}
    </div>

    {problem && <p role="alert">{problem}<button type="button" onClick={() => setProblem('')}>{t('Dismiss')}</button></p>}

    {menu && <CoveringMenu onClose={() => setMenu(null)} menu={{ ...menu, label: t('Groups'), items: [{ id: 'create-group', label: t('Create Group'), run: () => open(undefined, menu.members) }] }} />}

    {draft && <GroupDialog
      key={draft.id || 'new'}
      initial={draft}
      context={{
        timeline, bridge, session, revision
      }}
      onClose={() => setDraft(null)}
    />}
  </Context.Provider>;
}
function GroupDialog({ initial, context, onClose }: {
  initial: TrackGroup;
  context: RoutingContext;
  onClose: () => void;
}) {
  const t = useFairlightText(), origin = useRef(context), current = useRef(context);
  const failure = useFailureText();
  current.current = context;
  const [value, setValue] = useState(initial), [add, setAdd] = useState<string[]>([]), [remove, setRemove] = useState<string[]>([]);
  const [problem, setProblem] = useState(''), [saving, setSaving] = useState(false);
  // Only a change of what the window edits makes it stale: this group, and the tracks it lists.
  // Another edit (a fader, another group) leaves it open, and it saves against the latest revision.
  const edited = (c: RoutingContext) => JSON.stringify([
    initial.id ? c.timeline.track_groups?.find(group => group.id === initial.id) ?? null
      : initial.all_tracks ? allGroup(c.timeline) : null,
    c.timeline.tracks.filter(track => track.kind === 'audio').map(track => [track.id, track.name, track.locked]),
  ]);
  const stale = context.session !== origin.current.session || context.timeline.id !== origin.current.timeline.id
    || (context.revision !== origin.current.revision && edited(context) !== edited(origin.current));
  const disabled = stale || saving, members = groupTracks(context.timeline, value);
  const audio = context.timeline.tracks.filter(track => track.kind === 'audio');
  const controls: Record<GroupControl, import('../../messages/fairlight').FairlightText> = {
    editing: 'Editing', fader: 'Fader', solo: 'solo', mute: 'mute', sends: 'Sends', panning: 'Panning', automation: 'Automation'
  };
  const save = async () => {
    if (disabled)
      return;
    setSaving(true);
    setProblem('');
    const { timeline, bridge, session, revision } = current.current;
    try {
      await callEngine(bridge, 'transaction.apply', { revision, ops: [groupPatch(timeline, initial, { ...value, name: value.name.trim() })] }, { expectedSession: session });
      if (current.current.session === session && current.current.timeline.id === timeline.id)
        onClose();
    }
    catch (error) {
      setProblem(failure(error));
      setSaving(false);
    }
  };
  return <Sheet
    title={t(initial.id || initial.all_tracks ? 'Modify Group' : 'Create Group')}
    wide
    onClose={() => {
      if (!saving)
        onClose();
    }}
    foot={<>

      <button
        type="button"
        disabled={disabled}
        onClick={() => {
          try {
            localStorage.setItem(GROUP_DEFAULTS_KEY, JSON.stringify(value.linked ?? GROUP_CONTROLS));
          }
          catch (error) {
            setProblem(failure(error));
          }
        }}
      >
        {t('Set as Default')}
      </button>

      <button type="button" data-first disabled={saving} onClick={onClose}>{t('Cancel')}</button>
      <button
        type="button"
        disabled={disabled || !value.name.trim() || !value.all_tracks && !members.length}
        onClick={() => void save()}
      >
        {t('Save')}
      </button>
    </>}
  >
    <div className="track-group-dialog">

      {stale && <p role="alert">{t('The project changed. Reopen this window before saving.')}</p>}
      {problem && <p role="alert">{problem}</p>}

      <fieldset disabled={disabled}>
        <label>
          {t('Name')}
          <input
            value={value.all_tracks ? t('All Group') : value.name}
            disabled={value.all_tracks}
            onChange={event => setValue({ ...value, name: event.target.value })}
          />
        </label>

        <label>
          {t('Color')}
          <select value={value.color ?? TRACK_COLORS[8]} onChange={event => setValue({ ...value, color: event.target.value })}>
            {TRACK_COLORS.map((color, i) => <option key={color} value={color}>{i + 1}</option>)}
          </select>
        </label>

        <fieldset>
          <legend>{t('Linked controls')}</legend>
          {GROUP_CONTROLS.map(control => <label key={control}>
            <input
              type="checkbox"
              checked={(value.linked ?? GROUP_CONTROLS).includes(control)}
              onChange={event => setValue({ ...value, linked: event.target.checked ? [...value.linked ?? GROUP_CONTROLS, control] : (value.linked ?? GROUP_CONTROLS).filter(c => c !== control) })}
            />
            {t(controls[control])}
          </label>)}
        </fieldset>

        {!value.all_tracks && <div className="track-group-members">
          <label>
            {t('Available tracks')}
            <select
              multiple
              value={add}
              onChange={event => setAdd([...event.target.selectedOptions].map(option => option.value))}
            >
              {audio.filter(track => !members.includes(track.id)).map(track => <option key={track.id} value={track.id}>{track.name}</option>)}
            </select>
          </label>

          <div>
            <button
              type="button"
              disabled={!add.length}
              onClick={() => {
                setValue({ ...value, members: [...members, ...add] });
                setAdd([]);
              }}
              aria-label={t('Add members')}
            >»</button>
            <button
              type="button"
              disabled={!remove.length}
              onClick={() => {
                setValue({ ...value, members: members.filter(id => !remove.includes(id)) });
                setRemove([]);
              }}
              aria-label={t('Remove members')}
            >«</button>
          </div>

          <label>
            {t('Group members')}
            <select
              multiple
              value={remove}
              onChange={event => setRemove([...event.target.selectedOptions].map(option => option.value))}
            >
              {audio.filter(track => members.includes(track.id)).map(track => <option key={track.id} value={track.id}>{track.name}</option>)}
            </select>
          </label>
        </div>}
      </fieldset>
    </div>
  </Sheet>;
}
function SelectionBox({ label, checked, mixed, onClick }: {
  label: string;
  checked: boolean;
  mixed: boolean;
  onClick: (shift: boolean) => void;
}) {
  const ref = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (ref.current)
      ref.current.indeterminate = mixed;
  }, [mixed]);
  return <input
    ref={ref}
    aria-label={label}
    type="checkbox"
    checked={checked}
    onChange={() => {
    }}
    onClick={event => onClick(event.shiftKey)}
  />;
}
function GroupName({ group, onToggle, onRename }: {
  group: TrackGroup;
  onToggle: () => void;
  onRename: (name: string) => Promise<boolean>;
}) {
  const t = useFairlightText(), [renaming, setRenaming] = useState(false), click = useRef<ReturnType<typeof setTimeout> | null>(null);
  const cancel = () => {
    if (click.current !== null)
      clearTimeout(click.current);
    click.current = null;
  };
  useEffect(() => cancel, []);
  if (renaming)
    return <NameField
      name={group.name}
      onCancel={() => setRenaming(false)}
      onCommit={async (name) => {
        const saved = await onRename(name);
        if (saved)
          setRenaming(false);
        return saved;
      }}
    />;
  return <button
    type="button"
    style={{ borderLeft: `4px solid ${group.color ?? TRACK_COLORS[8]}` }}
    aria-pressed={group.active !== false}
    onClick={event => {
      cancel();
      if (event.detail === 0)
        onToggle();
      else if (event.detail === 1)
        click.current = setTimeout(() => {
          click.current = null;
          onToggle();
        }, 500);
    }}
    onDoubleClick={() => {
      cancel();
      if (!group.all_tracks)
        setRenaming(true);
    }}
    onKeyDown={event => {
      if (event.key === 'F2' && !group.all_tracks) {
        event.preventDefault();
        cancel();
        setRenaming(true);
      }
    }}
  >
    {group.all_tracks ? t('All Group') : group.name}
  </button>;
}
export function TrackGroups() {
  const context = useTrackGroups(), t = useFairlightText();
  const [remove, setRemove] = useState<{
    group: TrackGroup;
    revision: number;
    session: number;
    timeline: string;
  } | null>(null);
  const [deleting, setDeleting] = useState(false);
  if (!context)
    return null;
  const { timeline, selected, select, edit, open } = context, groups = [allGroup(timeline), ...timeline.track_groups?.filter(group => !group.all_tracks) ?? []];
  const staleRemoval = remove !== null && (context.revision !== remove.revision || context.session !== remove.session || timeline.id !== remove.timeline);
  return <section className="track-groups" aria-label={t('Groups')}>
    <header><h2>{t('Groups')}</h2><button type="button" onClick={() => open()}>{t('Create Group')}</button></header>

    <label>
      <input
        type="checkbox"
        checked={!timeline.audio?.groups_bypassed}
        onChange={event => void edit([{ op: 'set_timeline_audio', timeline: timeline.id, patch: { groups_bypassed: !event.target.checked } }])}
      />
      {t('Groups Switch')}
    </label>

    {groups.map(group => {
      const members = groupTracks(timeline, group), count = members.filter(id => selected.includes(id)).length, name = group.all_tracks ? t('All Group') : group.name;
      return <div
        key={JSON.stringify([context.session, timeline.id, group.id])}
        className="track-group-row"
        data-active={group.active !== false && !timeline.audio?.groups_bypassed || undefined}
      >

        <SelectionBox
          label={`${t('Selected')} ${name}`}
          checked={members.length > 0 && count === members.length && group.active !== false}
          mixed={count > 0 && count < members.length}
          onClick={shift => {
            const active = !(count === members.length && group.active !== false);
            select(active ? [...new Set([...(shift ? selected : []), ...members])] : selected.filter(id => !members.includes(id)));
            if (group.active !== active || !group.id)
              void edit([groupPatch(timeline, group, { active })]);
          }}
        />

        <GroupName
          group={group}
          onToggle={() => void edit([groupPatch(timeline, group, { active: group.active === false })])}
          onRename={name => edit([groupPatch(timeline, group, { name })])}
        />

        <button
          type="button"
          aria-label={`${t('Modify Group')} ${name}`}
          onClick={() => open(group)}
        >⚙</button>
        {!group.all_tracks && <button
          type="button"
          aria-label={`${t('Delete Group')} ${name}`}
          onClick={() => setRemove({
            group, revision: context.revision, session: context.session, timeline: timeline.id
          })}
        >×</button>}
      </div>;
    })}

    {remove && <Sheet
      title={t('Delete Group')}
      onClose={() => {
        if (!deleting)
          setRemove(null);
      }}
      foot={<>

        <button
          type="button"
          data-first
          disabled={deleting}
          onClick={() => setRemove(null)}
        >{t('Cancel')}</button>

        <button
          type="button"
          disabled={deleting || staleRemoval}
          onClick={async () => {
            if (deleting || staleRemoval)
              return;
            setDeleting(true);
            if (await edit([{ op: 'remove_track_group', timeline: remove.timeline, id: remove.group.id }], remove.revision))
              setRemove(null);
            setDeleting(false);
          }}
        >
          {t('Delete')}
        </button>
      </>}
    >

      <p>{remove.group.name}</p>
      {staleRemoval && <p role="alert">{t('The project changed. Reopen this window before saving.')}</p>}
    </Sheet>}
  </section>;
}
export function GroupLabel({ track }: {
  track: string;
}) {
  const context = useTrackGroups(), t = useFairlightText(), [menu, setMenu] = useState<{
    x: number;
    y: number;
  } | null>(null);
  if (!context)
    return null;
  const { timeline, edit } = context, groups = [allGroup(timeline), ...timeline.track_groups?.filter(group => !group.all_tracks) ?? []];
  const name = (group: TrackGroup) => `${group.all_tracks ? t('All Group') : group.name}${group.active === false || timeline.audio?.groups_bypassed ? ' ○' : ' ●'}`;
  return <div className="track-group-label">
    <button
      type="button"
      aria-label={`${t('Groups')} ${timeline.tracks.find(each => each.id === track)?.name ?? track}`}
      aria-expanded={Boolean(menu)}
      onClick={event => {
        const box = event.currentTarget.getBoundingClientRect();
        setMenu({ x: box.left, y: box.bottom });
      }}
    >
      {groups.filter(group => !group.all_tracks && group.members.includes(track)).map(name).join(', ') || t('Groups')}
    </button>

    {menu && <CoveringMenu onClose={() => setMenu(null)} menu={{
      ...menu, label: t('Groups'), items: [
        ...groups.map(group => ({
          id: group.id || 'all', label: name(group), checked: Boolean(group.all_tracks || group.members.includes(track)), disabled: group.all_tracks, run: () => void edit([groupPatch(timeline, group, { members: group.members.includes(track) ? group.members.filter(id => id !== track) : [...group.members, track] })])
        })),
        { id: 'create-group', label: t('Create Group'), run: () => context.open(undefined, context.selected.includes(track) ? [...context.selected] : [track]) },
      ]
    }} />}
  </div>;
}
