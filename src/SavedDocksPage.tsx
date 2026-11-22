import { Pencil, RotateCcw, Save, Trash2 } from 'lucide-react';
import { useEffect, useState } from 'react';
import { readSavedDocks, type SavedDockEdit, type SavedDockList } from './savedDocks';

export function SavedDocksPage({ ready, busy, onEdit }: {
  ready: boolean;
  busy: boolean;
  onEdit: (edit: SavedDockEdit) => Promise<SavedDockList>;
}) {
  const [list, setList] = useState<SavedDockList>();
  const [name, setName] = useState('');
  const [renaming, setRenaming] = useState<{ id: string; name: string }>();
  const [deleting, setDeleting] = useState<string>();
  const [status, setStatus] = useState('');
  const [error, setError] = useState('');
  useEffect(() => {
    let alive = true;
    void readSavedDocks()
      .then((docks) => { if (alive) setList(docks); })
      .catch((error) => { if (alive) setError(String(error)); });
    return () => { alive = false; };
  }, []);

  function run(edit: SavedDockEdit, message: string, done?: () => void) {
    setError('');
    setStatus('');
    void onEdit(edit).then((next) => {
      setList(next);
      setStatus(message);
      done?.();
    }).catch((error) => setError(String(error)));
  }

  return <section className="saved-docks-page" aria-label="Manage saved docks" aria-busy={busy}>
    <div className="saved-docks-panel">
      <form className="save-dock-form" onSubmit={(event) => {
        event.preventDefault();
        const label = name.trim();
        if (label) run({ type: 'save', id: crypto.randomUUID(), name: label }, `Saved ${label}.`, () => setName(''));
      }}>
        <div><h2>Save current dock</h2><p>Save your layout, appearance, motion, widgets, and app order together.</p></div>
        <div className="saved-dock-actions">
          <input aria-label="Dock name" placeholder="Dock name" value={name} maxLength={64} required autoComplete="off"
            disabled={busy} onChange={(event) => setName(event.currentTarget.value)} />
          <button className="preset-button primary" type="submit" disabled={!ready || !list || busy || !name.trim()}><Save size={15} />Save dock</button>
        </div>
      </form>
      {list && list.docks.length > 0 && <ul className="saved-dock-list">
        {list.docks.map((dock) => {
          const current = list.activeId === dock.id;
          return <li key={dock.id} className="saved-dock-row" aria-current={current || undefined}>
            {renaming?.id === dock.id ? <form className="saved-dock-inline" onSubmit={(event) => {
              event.preventDefault();
              const label = renaming.name.trim();
              if (label) run({ type: 'rename', id: dock.id, name: label }, `Renamed to ${label}.`, () => setRenaming(undefined));
            }}>
              <input aria-label={`Rename ${dock.name}`} value={renaming.name} maxLength={64} autoFocus required disabled={busy}
                onChange={(event) => setRenaming({ ...renaming, name: event.currentTarget.value })} />
              <button className="preset-button primary" type="submit" disabled={busy || !renaming.name.trim()}>Rename</button>
              <button className="preset-button" type="button" disabled={busy} onClick={() => setRenaming(undefined)}>Cancel</button>
            </form> : deleting === dock.id ? <div className="saved-dock-inline">
              <span>Delete “{dock.name}”?</span>
              <button className="preset-button danger" disabled={busy} onClick={() => run({ type: 'delete', id: dock.id }, `Deleted ${dock.name}.`, () => setDeleting(undefined))}>Delete</button>
              <button className="preset-button" disabled={busy} onClick={() => setDeleting(undefined)}>Cancel</button>
            </div> : <>
              <div className="saved-dock-info">
                <div><strong>{dock.name}</strong>{current && <span className="saved-dock-current">Current</span>}</div>
                <small>{dock.edge} edge · {dock.material} · {dock.widgetCount} {dock.widgetCount === 1 ? 'widget' : 'widgets'}</small>
              </div>
              <div className="saved-dock-actions">
                {current && <button className="preset-button" disabled={busy} onClick={() => run({ type: 'update', id: dock.id }, `Saved changes to ${dock.name}.`)}>Save changes</button>}
                <button className="preset-button" disabled={busy} aria-label={current ? `Restore ${dock.name}` : `Use ${dock.name}`}
                  title={current ? 'Restore saved version' : undefined} onClick={() => run({ type: 'load', id: dock.id }, `Using ${dock.name}.`)}>
                  {current ? <RotateCcw size={15} /> : 'Use dock'}
                </button>
                <button className="preset-button" disabled={busy} aria-label={`Rename ${dock.name}`} title="Rename saved dock"
                  onClick={() => { setDeleting(undefined); setRenaming({ id: dock.id, name: dock.name }); }}><Pencil size={15} /></button>
                <button className="preset-button" disabled={busy} aria-label={`Delete ${dock.name}`} title="Delete saved dock"
                  onClick={() => { setRenaming(undefined); setDeleting(dock.id); }}><Trash2 size={15} /></button>
              </div>
            </>}
          </li>;
        })}
      </ul>}
    </div>
    {error ? <p className="saved-dock-feedback" role="alert">{error}</p> : <p className="saved-dock-feedback" role="status">
      {busy ? 'Saving…' : status || (!list ? 'Loading saved docks…' : list.docks.length ? 'Use a saved dock to edit it. Save changes when you’re done.' : '')}
    </p>}
  </section>;
}
