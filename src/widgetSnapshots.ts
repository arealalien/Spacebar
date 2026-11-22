import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { useSyncExternalStore } from 'react';
import { desktop } from './model';

const idleSubscription = () => () => {};

export function widgetSnapshot<T>(command: string, interval = 1100, changeEvent?: string) {
  const listeners = new Set<() => void>();
  let value: T | undefined;
  let generation = 0;
  let request: Promise<void> | undefined;
  let timer: number | undefined;
  let subscription: Promise<UnlistenFn> | undefined;

  function publish(next: T | undefined) {
    generation++;
    value = next;
    for (const listener of listeners) listener();
  }

  function refresh() {
    if (request) return request;
    const current = generation;
    request = invoke<T>(command).then((next) => {
      if (generation === current) publish(next);
    }).catch(() => {
      if (generation === current && value !== undefined) publish(undefined);
    }).finally(() => { request = undefined; });
    return request;
  }

  function subscribe(listener: () => void) {
    listeners.add(listener);
    if (listeners.size === 1) {
      void refresh();
      timer = window.setInterval(() => void refresh(), interval);
      if (changeEvent) subscription = listen(changeEvent, () => {
        // A playback update can arrive while the previous snapshot is still resolving.
        void Promise.resolve(request).then(() => { if (listeners.size) return refresh(); });
      });
    }
    return () => {
      listeners.delete(listener);
      if (!listeners.size) {
        window.clearInterval(timer);
        void subscription?.then((unlisten) => unlisten()).catch(console.error);
      }
    };
  }

  return {
    useValue(live: boolean) {
      const enabled = desktop && live;
      return useSyncExternalStore(enabled ? subscribe : idleSubscription, () => enabled ? value : undefined);
    },
    refresh,
    update(transform: (current: T | undefined) => T) { publish(transform(value)); },
  };
}
