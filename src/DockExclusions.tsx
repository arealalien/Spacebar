import { invoke } from '@tauri-apps/api/core';
import { Plus } from 'lucide-react';
import { useEffect, useState } from 'react';

interface ExcludedApp {
  name: string;
  path: string;
}

export function DockExclusions() {
  const [apps, setApps] = useState<ExcludedApp[]>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');

  useEffect(() => {
    let alive = true;
    void invoke<ExcludedApp[]>('read_dock_exclusions')
      .then((apps) => { if (alive) setApps(apps); })
      .catch((error) => { if (alive) setError(String(error)); });
    return () => { alive = false; };
  }, []);

  async function edit(path?: string) {
    setBusy(true);
    setError('');
    try {
      const next = path === undefined
        ? await invoke<ExcludedApp[]>('add_dock_exclusion')
        : await invoke<ExcludedApp[]>('remove_dock_exclusion', { path });
      setApps(next);
    } catch (error) {
      setError(String(error));
    } finally {
      setBusy(false);
    }
  }

  return <section className="setting-group" aria-labelledby="dock-exclusions">
    <h2 id="dock-exclusions">Excluded apps</h2>
    <div className="settings-list">
      <div className="setting-row">
        <div>
          <p className="dock-exclusions-detail">Hide the dock whenever a listed program is active.</p>
          {apps?.length === 0 && <p className="setting-detail">No excluded apps</p>}
        </div>
        <button className="preset-button" type="button" disabled={busy || (!apps && !error)} onClick={() => void edit()}>
          <Plus size={15} aria-hidden="true" />Add program…
        </button>
      </div>
      {!!apps?.length && <ul className="dock-exclusions-list" aria-label="Programs that hide the dock">
        {apps.map(({ name, path }) => <li key={path} className="setting-row">
          <div>
            <div className="setting-label dock-exclusion-name" title={name}>{name}</div>
            <div className="setting-detail dock-exclusion-path" title={path}>{path}</div>
          </div>
          <button className="preset-button" type="button" disabled={busy} aria-label={`Remove ${name}`}
            onClick={() => void edit(path)}>Remove</button>
        </li>)}
      </ul>}
      {error && <p className="dock-exclusions-error" role="alert">{error}</p>}
    </div>
  </section>;
}
