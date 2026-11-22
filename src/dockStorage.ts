import type { AppAppearance, Settings } from './model';
import type { WidgetItem, WidgetLayout } from './widgets';

export interface StoredDock {
  id: string;
  name: string;
  settings: Settings;
  items: WidgetItem[];
}

interface BrowserDock {
  settings?: Settings;
  appAppearance?: AppAppearance;
  widgets?: WidgetLayout;
  savedDocks?: StoredDock[];
  activeDock?: string | null;
}

export const dockStorageKey = 'spacebar.dock';
export const browserWidgetsChanged = 'spacebar:widgets-changed';

export function readBrowserDock(): BrowserDock {
  try { return JSON.parse(localStorage.getItem(dockStorageKey) ?? 'null') ?? {}; }
  catch { return {}; }
}

export function writeBrowserDock(change: Partial<BrowserDock>) {
  localStorage.setItem(dockStorageKey, JSON.stringify({ ...readBrowserDock(), ...change }));
  if (change.widgets) window.dispatchEvent(new Event(browserWidgetsChanged));
}
