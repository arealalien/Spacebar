import { invoke } from '@tauri-apps/api/core';
import { FolderOpen, LayoutGrid, List, LoaderCircle, LockKeyhole, Orbit, Power, RotateCw, Search, Settings2, X } from 'lucide-react';
import { useEffect, useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from 'react';
import { AppIcon } from './AppIcon';
import { Calendar } from './Calendar';
import { StartSpotify } from './SpotifyWidgets';

interface StartApp { id: string; name: string; icon?: string }
interface StartPreferences { appLayout: 'list' | 'grid' }
type PowerAction = 'shutdown' | 'restart';
type StartAction = PowerAction | 'lock' | 'windows-settings' | 'spacebar-settings' | 'files';

const powerLabels = { shutdown: 'Shut down', restart: 'Restart' };
const appNames = new Intl.Collator(undefined, { sensitivity: 'base', numeric: true });

function groupApps(apps: StartApp[]) {
  const groups = new Map<string, StartApp[]>();
  for (const app of apps) {
    const letter = app.name.trim().normalize().match(/^\p{L}/u)?.[0].toLocaleUpperCase() ?? '#';
    const group = groups.get(letter);
    if (group) group.push(app);
    else groups.set(letter, [app]);
  }
  return [...groups].sort(([a], [b]) => {
    if (a === '#') return -1;
    if (b === '#') return 1;
    return appNames.compare(a, b);
  });
}

function useStartResource<T>(command: string) {
  const [state, setState] = useState<{ value?: T; error?: string }>({});
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    let active = true;
    void invoke<T>(command).then((value) => {
      if (active) setState({ value });
    }, (error) => {
      if (active) setState({ error: String(error) });
    });
    return () => { active = false; };
  }, [command, attempt]);
  return { ...state, retry: () => { setState({}); setAttempt((count) => count + 1); } };
}

function moveAppFocus(event: KeyboardEvent<HTMLDivElement>) {
  const { key, currentTarget } = event;
  const horizontal = key === 'ArrowLeft' || key === 'ArrowRight';
  const vertical = key === 'ArrowUp' || key === 'ArrowDown';
  const grid = currentTarget.dataset.layout === 'grid';
  if (!vertical && !(grid && horizontal)) return;
  const buttons = [...currentTarget.querySelectorAll<HTMLButtonElement>('button[data-start-app]:enabled')];
  const index = buttons.indexOf(event.target as HTMLButtonElement);
  if (index < 0) return;
  event.preventDefault();
  if (!grid || horizontal) {
    const direction = key === 'ArrowLeft' || key === 'ArrowUp' ? -1 : 1;
    buttons[index + direction]?.focus();
    return;
  }
  const from = buttons[index].getBoundingClientRect();
  const direction = key === 'ArrowUp' ? -1 : 1;
  const nearby = buttons.map((button) => {
    const box = button.getBoundingClientRect();
    return { button, row: (box.top - from.top) * direction, distance: Math.abs(box.left - from.left) };
  }).filter(({ row }) => row > 1);
  nearby.sort((a, b) => a.row - b.row || a.distance - b.distance);
  nearby[0]?.button.focus();
}

function AppsPane({ layout, layoutControl, busy, launching, onLaunch }: {
  layout: StartPreferences['appLayout'];
  layoutControl: ReactNode;
  busy: boolean;
  launching?: string;
  onLaunch: (id: string) => void;
}) {
  const { value: apps, error, retry } = useStartResource<StartApp[]>('read_start_apps');
  const [query, setQuery] = useState('');
  const search = useRef<HTMLInputElement>(null);
  const list = useRef<HTMLDivElement>(null);
  useEffect(() => { search.current?.focus(); }, []);

  const term = query.trim().toLocaleLowerCase();
  const sorted = useMemo(() => apps?.slice().sort((a, b) => appNames.compare(a.name, b.name)), [apps]);
  const groups = useMemo(() => sorted && groupApps(sorted.filter((app) => app.name.toLocaleLowerCase().includes(term))), [sorted, term]);
  const firstApp = groups?.[0]?.[1][0];

  return <section className="start-apps-pane" aria-labelledby="start-apps-title">
    <div className="start-section-heading">
      <h2 id="start-apps-title">Apps</h2>
      {layoutControl}
    </div>
    <form className="start-search" onSubmit={(event) => {
      event.preventDefault();
      if (term && firstApp && !busy) onLaunch(firstApp.id);
    }}>
      <Search size={16} aria-hidden="true" />
      <input ref={search} aria-label="Search apps" placeholder="Search apps" aria-controls="start-app-results"
        value={query} autoComplete="off" onChange={(event) => {
          setQuery(event.currentTarget.value);
          if (list.current) list.current.scrollTop = 0;
        }} onKeyDown={(event) => {
          if (event.key !== 'ArrowDown') return;
          event.preventDefault();
          list.current?.querySelector<HTMLButtonElement>('button[data-start-app]:enabled')?.focus();
        }} />
      {query && <button type="button" aria-label="Clear search" title="Clear search" onClick={() => {
        setQuery('');
        search.current?.focus();
      }}><X size={15} /></button>}
    </form>
    <div ref={list} id="start-app-results" className="start-apps" data-layout={layout}
      aria-busy={!apps && !error} onKeyDown={moveAppFocus}>
      {error ? <div className="start-message">
        <p className="system-panel-note" role="status" title={error}>Couldn’t load apps.</p>
        <button type="button" onClick={retry}>Try again</button>
      </div> : !groups ? <>
        <p className="system-panel-note" role="status">Loading apps…</p>
        <div className="start-app-skeleton" aria-hidden="true">
          {Array.from({ length: layout === 'grid' ? 12 : 9 }, (_, index) =>
            <div key={index}><i /><span /></div>)}
        </div>
      </> : groups.length === 0 ? <p className="system-panel-note" role="status">
        {term ? 'No apps match your search.' : 'No apps found.'}
      </p> : groups.map(([letter, entries]) =>
        <section key={letter} className="start-app-group">
          <h3>{letter}</h3>
          <ul className="start-app-items">{entries.map((app) => {
            const opening = launching === app.id;
            return <li key={app.id}><button type="button" data-start-app data-launching={opening || undefined}
              aria-busy={opening || undefined} disabled={busy} title={app.name} onClick={() => onLaunch(app.id)}>
              <span className="start-app-icon" aria-hidden="true"><AppIcon app={app} /></span>
              <span className="start-app-name">{app.name}</span>
              {opening && <LoaderCircle className="start-app-spinner" size={15} aria-hidden="true" />}
            </button></li>;
          })}</ul>
        </section>)}
    </div>
  </section>;
}

export function StartLauncher({ onClose, onError }: { onClose: () => void; onError: (error: unknown) => void }) {
  const configuration = useStartResource<StartPreferences>('read_start_preferences');
  const [saved, setSaved] = useState<StartPreferences>();
  const preferences = saved ?? configuration.value;
  const [saving, setSaving] = useState(false);
  const [pane, setPane] = useState<'apps' | 'widgets'>('apps');
  const [powerOpen, setPowerOpen] = useState(false);
  const [confirm, setConfirm] = useState<PowerAction>();
  const [pending, setPending] = useState<{ app?: string }>();

  async function run(command: string, args: Record<string, unknown>, app?: string) {
    if (pending) return;
    setPending({ app });
    try { await invoke(command, args); }
    catch (error) { onError(error); }
    finally { setPending(undefined); }
  }

  function launch(id: string) { void run('launch_start_app', { id }, id); }
  function action(action: StartAction, confirmed = false) { void run('start_action', { action, confirmed }); }

  async function changeLayout(appLayout: StartPreferences['appLayout']) {
    if (!preferences || preferences.appLayout === appLayout || saving) return;
    setSaving(true);
    try {
      setSaved(await invoke<StartPreferences>('save_start_preferences', { preferences: { appLayout } }));
    } catch (error) { onError(error); }
    finally { setSaving(false); }
  }

  return <div className="start-launcher" onKeyDown={(event) => {
    if (event.key !== 'Escape' || !(confirm || powerOpen)) return;
    event.stopPropagation();
    if (confirm) setConfirm(undefined);
    else setPowerOpen(false);
  }}>
    <header className="start-header">
      <h1 id="system-panel-title">Spacebar Start</h1>
      <button type="button" className="start-close" aria-label="Close Start" title="Close Start" onClick={onClose}><X size={17} /></button>
    </header>
    <div className="start-pane-switch" role="group" aria-label="Start menu panels">
      <button type="button" aria-pressed={pane === 'apps'} onClick={() => setPane('apps')}>Apps</button>
      <button type="button" aria-pressed={pane === 'widgets'} onClick={() => setPane('widgets')}>Widgets</button>
    </div>
    <div className="start-workspace" data-pane={pane}>
      <AppsPane layout={preferences?.appLayout ?? 'list'} busy={!!pending} launching={pending?.app} onLaunch={launch}
        layoutControl={configuration.error && !preferences
          ? <button type="button" className="start-layout-retry" title={configuration.error} onClick={configuration.retry}>Retry layout</button>
          : <div className="start-layout-switch" role="group" aria-label="App layout" aria-busy={saving}>
            <button type="button" aria-label="List view" title="List view" disabled={!preferences}
              aria-pressed={(preferences?.appLayout ?? 'list') === 'list'} onClick={() => void changeLayout('list')}><List size={16} /></button>
            <button type="button" aria-label="Grid view" title="Grid view" disabled={!preferences}
              aria-pressed={preferences?.appLayout === 'grid'} onClick={() => void changeLayout('grid')}><LayoutGrid size={15} /></button>
          </div>} />
      <section className="start-widgets-pane" aria-labelledby="start-widgets-title">
        <div className="start-section-heading"><h2 id="start-widgets-title">Widgets</h2></div>
        <div className="start-widgets-scroll">
          <div className="start-calendar"><Calendar /></div>
          <StartSpotify onError={onError} />
        </div>
      </section>
    </div>
    {confirm ? <div className="start-power-options">
      <span>{powerLabels[confirm]} this PC?</span>
      <button type="button" disabled={!!pending} onClick={() => setConfirm(undefined)}>Cancel</button>
      <button type="button" className="start-power-confirm" disabled={!!pending} onClick={() => action(confirm, true)}>{powerLabels[confirm]}</button>
    </div> : powerOpen && <div className="start-power-options">
      <button type="button" disabled={!!pending} onClick={() => action('lock')}><LockKeyhole size={15} />Lock</button>
      <button type="button" disabled={!!pending} onClick={() => setConfirm('restart')}><RotateCw size={15} />Restart</button>
      <button type="button" disabled={!!pending} onClick={() => setConfirm('shutdown')}><Power size={15} />Shut down</button>
    </div>}
    <footer className="start-shortcuts" aria-label="Start shortcuts">
      <button type="button" disabled={!!pending} onClick={() => action('windows-settings')}><Settings2 size={16} />Windows settings</button>
      <button type="button" disabled={!!pending} onClick={() => action('spacebar-settings')}><Orbit size={16} />Dock settings</button>
      <button type="button" disabled={!!pending} onClick={() => action('files')}><FolderOpen size={16} />Files</button>
      <button type="button" disabled={!!pending} aria-expanded={powerOpen} onClick={() => {
        setConfirm(undefined);
        setPowerOpen(!powerOpen);
      }}><Power size={16} />Power</button>
    </footer>
  </div>;
}
