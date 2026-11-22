import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useEffect, useState } from 'react';
import { desktop } from './model';

export function useSurfacePayload<T>(event: string, command: string, enabled = true) {
  const [payload, setPayload] = useState<T>();
  useEffect(() => {
    if (!desktop || !enabled) return;
    let alive = true;
    let changed = false;
    const subscription = listen<T | null>(event, ({ payload }) => {
      changed = true;
      if (alive) setPayload(payload ?? undefined);
    });
    void subscription.then(() => invoke<T | null>(command)).then((current) => {
      if (alive && !changed) setPayload(current ?? undefined);
    }).catch(console.error);
    return () => { alive = false; void subscription.then((unlisten) => unlisten()).catch(console.error); };
  }, [event, command, enabled]);
  return enabled ? payload : undefined;
}
