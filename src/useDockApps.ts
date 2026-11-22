import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useEffect, useState } from 'react';
import { desktop, type DockApp } from './model';

export function useDockApps(enabled = true) {
  const [apps, setApps] = useState<DockApp[]>([]);
  const [ready, setReady] = useState(false);
  const [error, setError] = useState('');
  useEffect(() => {
    if (!enabled || !desktop) return;
    let disposed = false;
    let timer: number;
    let request = 0;
    async function refresh() {
      const current = ++request;
      window.clearTimeout(timer);
      try {
        const next = await invoke<DockApp[]>('list_dock_apps');
        if (!disposed && current === request) { setApps((previous) => JSON.stringify(previous) === JSON.stringify(next) ? previous : next); setError(''); setReady(true); }
      } catch (error) {
        if (!disposed && current === request) { setError(String(error)); setReady(true); }
      } finally {
        if (!disposed && current === request) timer = window.setTimeout(refresh, 750);
      }
    }
    const changes = listen('dock-apps-changed', () => { if (!disposed) void refresh(); });
    void refresh();
    return () => { disposed = true; window.clearTimeout(timer); void changes.then((unlisten) => unlisten()).catch(console.error); };
  }, [enabled]);
  return { apps, ready, error };
}
