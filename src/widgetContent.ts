import { invoke } from '@tauri-apps/api/core';
import { useEffect, useState } from 'react';

export interface AgendaEvent {
  id: string;
  title: string;
  startsAt: number;
  link?: string | null;
}

export interface QuickTask {
  id: string;
  title: string;
  done: boolean;
}

export interface WeatherLocation {
  name: string;
  detail: string;
  latitude: number;
  longitude: number;
}

export interface WeatherPreferences {
  location: WeatherLocation | null;
  fahrenheit: boolean;
}

interface WidgetDocuments {
  'quick-note': string;
  agenda: AgendaEvent[];
  'quick-tasks': QuickTask[];
  'color-picker': string[];
  weather: WeatherPreferences;
}

type ContentKind = keyof WidgetDocuments;
const writes = new Map<ContentKind, Promise<void>>();

export function writeWidgetContent<K extends ContentKind>(kind: K, data: WidgetDocuments[K]) {
  const prior = writes.get(kind);
  const request = (prior?.catch(() => {}) ?? Promise.resolve()).then(() =>
    invoke<void>('write_widget_content', { content: { kind, data } }));
  writes.set(kind, request);
  const release = () => { if (writes.get(kind) === request) writes.delete(kind); };
  void request.then(release, release);
  return request;
}

export function useWidgetContent<K extends ContentKind>(kind: K, onError: (error: unknown) => void) {
  const [value, setValue] = useState<WidgetDocuments[K]>();
  const [loading, setLoading] = useState(true);
  useEffect(() => {
    let alive = true;
    async function open() {
      try {
        await writes.get(kind)?.catch(() => {});
        if (!alive) return;
        const response = await invoke<{ data: WidgetDocuments[K] }>('read_widget_content', { kind });
        if (alive) setValue(response.data);
      } catch (error) {
        if (alive) onError(error);
      } finally {
        if (alive) setLoading(false);
      }
    }
    void open();
    return () => { alive = false; };
  }, [kind, onError]);
  return { value, setValue, loading };
}

export function useAgendaTime(live = true) {
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    if (!live) return;
    const timer = window.setInterval(() => setNow(Date.now()), 15_000);
    return () => window.clearInterval(timer);
  }, [live]);
  return now;
}
