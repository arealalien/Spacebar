import { invoke } from '@tauri-apps/api/core';
import { readBrowserDock, writeBrowserDock, type StoredDock } from './dockStorage';
import { browserSettings, desktop, normalizeSettings, type Settings } from './model';
import { browserLayout, migrateWidgetItems, settledWidgetEdits } from './widgets';

interface SavedDockSummary {
  id: string;
  name: string;
  edge: Settings['edge'];
  material: Settings['material'];
  widgetCount: number;
}

export interface SavedDockList {
  activeId: string | null;
  docks: SavedDockSummary[];
}

export type SavedDockEdit =
  | { type: 'save' | 'rename'; id: string; name: string }
  | { type: 'load' | 'update' | 'delete'; id: string };

interface SavedDockResult {
  list: SavedDockList;
  settings?: Settings | null;
}

function summaries(docks: StoredDock[], activeId: string | null): SavedDockList {
  return { activeId, docks: docks.map(({ id, name, settings, items }) => ({
    id, name, edge: settings.edge, material: settings.material, widgetCount: items.length,
  })) };
}

function migrateSavedDocks(docks: StoredDock[]) {
  return docks.map((dock) => ({ ...dock, items: migrateWidgetItems(dock.items) }));
}

export async function readSavedDocks(): Promise<SavedDockList> {
  if (desktop) return invoke('read_saved_docks');
  const { savedDocks = [], activeDock = null } = readBrowserDock();
  return summaries(migrateSavedDocks(savedDocks), activeDock);
}

export async function editSavedDocks(edit: SavedDockEdit): Promise<SavedDockResult> {
  await settledWidgetEdits();
  if (desktop) return invoke('edit_saved_docks', { edit });

  const { savedDocks = [], activeDock = null } = readBrowserDock();
  let active = activeDock;
  const docks = migrateSavedDocks(savedDocks);
  const index = docks.findIndex((dock) => dock.id === edit.id);
  const name = 'name' in edit ? edit.name.trim() : undefined;
  if (name !== undefined && (name.length < 1 || Array.from(name).length > 64)) {
    throw new Error('Enter a dock name between 1 and 64 characters.');
  }
  if (edit.type !== 'save' && index < 0) throw new Error('This saved dock is no longer available.');

  switch (edit.type) {
    case 'save':
      docks.unshift({ id: edit.id, name: name!, settings: browserSettings(), items: browserLayout().items });
      active = edit.id;
      break;
    case 'load': {
      const dock = docks[index];
      const settings = normalizeSettings(dock.settings);
      writeBrowserDock({ settings, widgets: { revision: browserLayout().revision + 1, items: dock.items }, savedDocks: docks, activeDock: edit.id });
      return { list: summaries(docks, edit.id), settings };
    }
    case 'update':
      if (active !== edit.id) throw new Error('Load this dock before saving changes to it.');
      docks[index] = { ...docks[index], settings: browserSettings(), items: browserLayout().items };
      break;
    case 'rename':
      docks[index] = { ...docks[index], name: name! };
      break;
    case 'delete':
      docks.splice(index, 1);
      if (active === edit.id) active = null;
      break;
  }
  writeBrowserDock({ savedDocks: docks, activeDock: active });
  return { list: summaries(docks, active) };
}
