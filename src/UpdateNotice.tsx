import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { ArrowDownToLine, RotateCw } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { desktop } from './model';

type Update = {
  version: string;
  phase: 'available' | 'downloading' | 'ready' | 'installing';
};

const copy = {
  available: { title: 'Update available', action: 'Download' },
  downloading: { title: 'Downloading update', action: 'Downloading…' },
  ready: { title: 'Update downloaded', action: 'Install update' },
  installing: { title: 'Installing update', action: 'Installing…' },
};

export function UpdateNotice() {
  const [update, setUpdate] = useState<Update | null>(null);
  const [error, setError] = useState('');
  const actionPending = useRef(false);

  useEffect(() => {
    if (!desktop) return;
    let disposed = false;
    let changed = false;
    const subscription = listen<Update>('spacebar-update', ({ payload }) => {
      changed = true;
      if (!disposed) setUpdate(payload);
    });
    void subscription.then(() => invoke<Update | null>('read_update')).then((initial) => {
      if (!changed && !disposed) setUpdate(initial);
    }).catch(console.error);
    return () => { disposed = true; void subscription.then((unlisten) => unlisten()).catch(console.error); };
  }, []);

  async function apply(next: 'available' | 'ready') {
    if (actionPending.current) return;
    actionPending.current = true;
    setError('');
    try {
      await invoke(next === 'available' ? 'download_update' : 'install_update');
    } catch {
      setError(next === 'available' ? 'Download failed. Try again.' : 'Couldn’t start the installer. Try again.');
    } finally {
      actionPending.current = false;
    }
  }

  if (!desktop) return null;

  const phase = update?.phase;
  const action = phase === 'available' || phase === 'ready' ? phase : undefined;
  const Icon = phase === 'ready' || phase === 'installing' ? RotateCw : ArrowDownToLine;

  return <div className={update ? 'update-notice' : undefined}>
    <div role="status">
      {update && <><strong>{copy[update.phase].title}</strong><span>Version {update.version}</span></>}
    </div>
    {update && <>
      {update.phase === 'ready' && <p>Spacebar will restart after installation.</p>}
      <button type="button" disabled={!action} onClick={() => { if (action) void apply(action); }}>
        <Icon size={14} aria-hidden="true" />{copy[update.phase].action}
      </button>
      {error && <p className="update-error" role="alert">{error}</p>}
    </>}
  </div>;
}
