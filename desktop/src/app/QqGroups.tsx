import { useEffect, useState } from 'react';
import { Eraser, Eye, EyeOff } from 'lucide-react';
import { useWords } from './locale';
import { failureOf, groupsOf, type Group } from './work';

/**
 * The groups of QQ that the bot was asked in, under the assistant in QQ: the name that each is
 * known by, and its memory, which can be read and forgotten. Without the shell, as in a replay,
 * and before the bot was asked in any group, there is no list.
 */
export function QqGroups() {
  const words = useWords();
  const said = words.label.work.groups;
  const told = words.hint.work.groups;
  const bridge = window.columbina?.qqGroups ? window.columbina : undefined;
  const [groups, setGroups] = useState<Group[] | null>(null);
  const [names, setNames] = useState<Record<string, string>>({});
  // The memory that is shown, of one group at a time.
  const [shown, setShown] = useState<{ id: string; memory: string } | null>(null);
  const [asking, setAsking] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState<string | null>(null);

  const asked = async (what: () => Promise<unknown>): Promise<unknown> => {
    if (!bridge) return null;
    setBusy(true);
    setFailed(null);
    try {
      const answer = await what();
      const listed = groupsOf(await bridge.qqGroups({ what: 'list' }));
      setGroups(listed);
      setNames(Object.fromEntries(listed.map(group => [group.id, group.name ?? ''])));
      return answer;
    } catch (problem) {
      setFailed(`${told.failed} ${failureOf(problem)}`);
      return null;
    } finally {
      setBusy(false);
    }
  };
  useEffect(() => {
    // Where the program cannot be asked, as where it is not there, there is no list, and nothing to say of it.
    void bridge?.qqGroups({ what: 'list' }).then(answer => {
      const listed = groupsOf(answer);
      setGroups(listed);
      setNames(Object.fromEntries(listed.map(group => [group.id, group.name ?? ''])));
    }).catch(() => {});
  }, []);
  if (!bridge || groups === null) return null;

  const show = async (group: Group) => {
    if (shown?.id === group.id) { setShown(null); return; }
    const answer = await asked(() => bridge.qqGroups({ what: 'memory', id: group.id })) as { memory?: unknown } | null;
    if (answer && typeof answer.memory === 'string') setShown({ id: group.id, memory: answer.memory });
  };
  const forget = async (id: string) => {
    setAsking(null);
    await asked(() => bridge.qqGroups({ what: 'forget', id }));
    if (shown?.id === id) setShown({ id, memory: '' });
  };

  return <div className="qq-groups" data-test="qq-groups">
    <div className="overline">{said.title}</div>
    {groups.length === 0 && <p className="caption">{told.none}</p>}
    {groups.map(group => {
      const named = group.name ?? said.unnamed;
      const typed = names[group.id] ?? '';
      return <div className="work-item qq-group" key={group.id} data-test="qq-group">
        <div className="work-words">
          <div className="setting-row">
            <input
              className="field setting-grow"
              value={typed}
              maxLength={64}
              placeholder={said.unnamed}
              aria-label={said.name}
              disabled={busy}
              onChange={event => setNames(current => ({ ...current, [group.id]: event.target.value }))}
              data-test="qq-group-name"
            />
            <button
              type="button"
              className="btn"
              disabled={busy || typed.trim() === (group.name ?? '')}
              onClick={() => void asked(() => bridge.qqGroups({ what: 'name', id: group.id, name: typed.trim() }))}
              data-test="qq-group-save"
            >
              {said.save}
            </button>
          </div>
          <span className="caption"><span className="mono">{group.id}</span> · {said.memory(group.memory, group.most)}</span>
          {shown?.id === group.id && <pre className="block" tabIndex={0} data-test="qq-group-memory">{shown.memory || told.empty}</pre>}
          {asking === group.id && <div className="turn-ask" role="group">
            <p className="caption">{told.forgetAsk(named)}</p>
            <button type="button" className="btn" disabled={busy} onClick={() => void forget(group.id)} data-test="qq-group-forget-yes">{said.forget}</button>
            <button type="button" className="btn btn-quiet" onClick={() => setAsking(null)}>{words.label.work.keep}</button>
          </div>}
        </div>
        <button
          type="button"
          className="icon-btn"
          disabled={busy}
          aria-label={shown?.id === group.id ? said.hide : said.show}
          title={shown?.id === group.id ? said.hide : said.show}
          onClick={() => void show(group)}
          data-test="qq-group-show"
        >
          {shown?.id === group.id ? <EyeOff size={14} /> : <Eye size={14} />}
        </button>
        <button
          type="button"
          className="icon-btn"
          disabled={busy || group.memory === 0}
          aria-label={said.forget}
          title={said.forget}
          onClick={() => setAsking(group.id)}
          data-test="qq-group-forget"
        >
          <Eraser size={14} />
        </button>
      </div>;
    })}
    {failed && <p className="caption" data-tone="warn" role="alert" data-test="qq-groups-failed">{failed}</p>}
  </div>;
}
