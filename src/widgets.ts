import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useEffect, useRef, useState, type CSSProperties, type MouseEvent, type PointerEvent } from 'react';
import { desktop, normalizeHexColor, settingsStorageKey, type DockApp } from './model';
import { browserWidgetsChanged, dockStorageKey, readBrowserDock, writeBrowserDock } from './dockStorage';
import { isLayoutWidget, isStartWidget, repeatableWidget, type WidgetKind } from './widgetCatalog';
import type { ImageWidgetOptions } from './imageWidgetStorage';
export { widgetNames, type WidgetKind } from './widgetCatalog';

export interface WidgetItem extends Partial<AudioVisualizerOptions> {
  id: string;
  kind: WidgetKind | 'app';
  name?: string;
  children?: WidgetItem[];
  appId?: string;
  launchPath?: string;
  icon?: string;
  background?: boolean;
  frame?: boolean;
  spacerSize?: number;
  size?: WidgetSize;
  color?: string;
  colors?: Partial<Record<WidgetColorChannel, string>>;
  image?: ImageWidgetOptions;
}

type StoredWidgetItem = Omit<WidgetItem, 'kind' | 'children'> & {
  kind: WidgetItem['kind'] | 'clock-outline' | 'desk-pet';
  children?: StoredWidgetItem[];
};

export function migrateWidgetItems(items: StoredWidgetItem[]): WidgetItem[] {
  return items.flatMap(({ kind, children, ...item }) => {
    if (kind === 'desk-pet') return [];
    return [{
      ...item, kind: kind === 'clock-outline' ? 'clock-plain' : kind,
      frame: kind === 'clock-outline' ? item.frame ?? true : item.frame,
      children: children && migrateWidgetItems(children),
    }];
  });
}

type AppearanceOption = 'background' | 'frame';
export type WidgetSize = 'small' | 'normal' | 'large';
export interface AudioVisualizerOptions {
  audioAlignment: 'center' | 'bottom';
  audioSpeed: 'slow' | 'normal' | 'fast';
  audioSensitivity: 'low' | 'normal' | 'high';
}

export const audioDefaults: AudioVisualizerOptions = {
  audioAlignment: 'center', audioSpeed: 'normal', audioSensitivity: 'normal',
};
export const widgetScale = (size: WidgetSize | undefined) => size === 'small' ? 0.78 : size === 'large' ? 1.35 : 1;

export const defaultSpacerSize = 32;
export const minSpacerSize = 8;
export const maxSpacerSize = 240;

export function supportsWidgetSurface(kind: WidgetItem['kind']) {
  return !isStartWidget(kind) && !isLayoutWidget(kind) && kind !== 'image';
}

export const supportsWidgetSize = (kind: WidgetItem['kind']) => kind !== 'app' && kind !== 'spacer';

type WidgetColorChannel = 'cpu' | 'ram' | 'gpu' | 'download' | 'upload' | 'elapsed' | 'current' | 'upcoming' | 'hands' | 'dial';
type WidgetColorKey = WidgetColorChannel | 'accent';
interface WidgetColorOption {
  key: WidgetColorKey;
  label: string;
  defaultColor: string;
  legacyColorWeight?: number;
}

const performanceColors: readonly WidgetColorOption[] = [
  { key: 'cpu', label: 'CPU', defaultColor: '#a3d8f4', legacyColorWeight: 1 },
  { key: 'ram', label: 'RAM', defaultColor: '#c7aafa', legacyColorWeight: 1 },
  { key: 'gpu', label: 'GPU', defaultColor: '#a0e0c0', legacyColorWeight: 1 },
];
const networkColors: readonly WidgetColorOption[] = [
  { key: 'download', label: 'Download', defaultColor: '#83d6f5', legacyColorWeight: 1 },
  { key: 'upload', label: 'Upload', defaultColor: '#b6a3f9', legacyColorWeight: .65 },
];
const clockColors: readonly WidgetColorOption[] = [
  { key: 'hands', label: 'Hour and minute hands', defaultColor: '#f7f7f9' },
  { key: 'dial', label: 'Dial accents', defaultColor: '#d0d9e7', legacyColorWeight: 1 },
];
const multiColorOptions: Partial<Record<WidgetKind, readonly WidgetColorOption[]>> = {
  performance: performanceColors,
  'system-rings': performanceColors.slice(0, 2),
  network: networkColors,
  'network-usage': networkColors,
  'day-progress': [
    { key: 'elapsed', label: 'Elapsed hours', defaultColor: '#f0f0f4', legacyColorWeight: .52 },
    { key: 'current', label: 'Current hour', defaultColor: '#ffc9a9', legacyColorWeight: 1 },
    { key: 'upcoming', label: 'Upcoming hours', defaultColor: '#ffffff' },
  ],
  'clock-analog': clockColors,
  'clock-twilight': [clockColors[0], { ...clockColors[1], defaultColor: '#ffbd97' }],
};
const singleColorOption: readonly WidgetColorOption[] = [{ key: 'accent', label: 'Color', defaultColor: '#f4f3f5' }];
const visualizerColor: readonly WidgetColorOption[] = [{ key: 'accent', label: 'Wave color', defaultColor: '#d7e5ff' }];

export function widgetColorOptions(kind: WidgetItem['kind']): readonly WidgetColorOption[] {
  if (kind === 'app' || kind === 'apps' || kind === 'spacer' || kind === 'image') return [];
  if (kind === 'audio-visualizer') return visualizerColor;
  return multiColorOptions[kind] ?? singleColorOption;
}

export function widgetColorValue(item: WidgetItem, { key, defaultColor, legacyColorWeight }: WidgetColorOption): string | undefined {
  const legacy = normalizeHexColor(item.color);
  if (key === 'accent') return legacy;
  const individual = normalizeHexColor(item.colors?.[key]);
  if (individual || !legacy || legacyColorWeight === undefined) return individual;
  if (legacyColorWeight === 1) return legacy;
  // Earlier versions blended the shared accent into some parts of the widget.
  const mix = (index: number) => {
    const from = parseInt(legacy.slice(index, index + 2), 16);
    const to = parseInt(defaultColor.slice(index, index + 2), 16);
    return Math.round(from * legacyColorWeight + to * (1 - legacyColorWeight)).toString(16).padStart(2, '0');
  };
  return '#' + [1, 3, 5].map(mix).join('');
}

export function widgetAppearance(item: WidgetItem) {
  if (!supportsWidgetSurface(item.kind)) return {};
  return { 'data-widget-background': item.background, 'data-widget-frame': item.frame };
}

export function widgetColorStyle(item?: WidgetItem): CSSProperties | undefined {
  if (!item) return;
  const styles: Record<string, string> = {};
  for (const option of widgetColorOptions(item.kind)) {
    const color = widgetColorValue(item, option);
    if (color) styles[option.key === 'accent' ? '--widget-color' : '--widget-color-' + option.key] = color;
  }
  return styles as CSSProperties;
}

export function hasWidgetSurface(item: WidgetItem, option: AppearanceOption) {
  if (!supportsWidgetSurface(item.kind)) return false;
  if (item[option] != null) return item[option];
  if (item.kind === 'apps' || item.kind === 'clock-plain') return false;
  return true;
}

export function toggleWidgetSurface(items: WidgetItem[], id: string, option: AppearanceOption) {
  return updateWidget(items, id, (item) => supportsWidgetSurface(item.kind) ? { ...item, [option]: !hasWidgetSurface(item, option) } : item);
}

export function setWidgetSize(items: WidgetItem[], id: string, size: WidgetSize) {
  return updateWidget(items, id, (item) => supportsWidgetSize(item.kind) && (item.size ?? 'normal') !== size ? { ...item, size } : item);
}

export function setWidgetColor(items: WidgetItem[], id: string, key: WidgetColorKey, color: string | undefined) {
  return updateWidget(items, id, (item) => {
    const options = widgetColorOptions(item.kind);
    const option = options.find((option) => option.key === key);
    if (!option || widgetColorValue(item, option) === color) return item;
    if (key === 'accent') return { ...item, color };

    const colors: NonNullable<WidgetItem['colors']> = {};
    for (const entry of options) {
      if (entry.key === 'accent') continue;
      const value = entry.key === key ? color : widgetColorValue(item, entry);
      if (value) colors[entry.key] = value;
    }
    return { ...item, color: undefined, colors: Object.keys(colors).length ? colors : undefined };
  });
}

export function setAudioOption<K extends keyof AudioVisualizerOptions>(items: WidgetItem[], id: string, key: K, value: AudioVisualizerOptions[K]) {
  const fallback = audioDefaults[key];
  return updateWidget(items, id, (item) => item.kind === 'audio-visualizer' && (item[key] ?? fallback) !== value
    ? { ...item, [key]: value === fallback ? undefined : value } : item);
}

function updateWidget(items: WidgetItem[], id: string, update: (item: WidgetItem) => WidgetItem): WidgetItem[] {
  const index = items.findIndex((item) => item.id === id || item.children?.some((child) => child.id === id));
  if (index === -1) return items;
  const item = items[index];
  const children = item.id !== id && item.children ? updateWidget(item.children, id, update) : undefined;
  const next = children ? (children === item.children ? item : { ...item, children }) : update(item);
  if (next === item) return items;
  const updated = items.slice();
  updated[index] = next;
  return updated;
}

export function resizeSpacer(items: WidgetItem[], id: string, size: number) {
  if (!Number.isFinite(size)) return items;
  const spacerSize = Math.min(maxSpacerSize, Math.max(minSpacerSize, Math.round(size)));
  return updateWidget(items, id, (item) => item.kind === 'spacer' && (item.spacerSize ?? defaultSpacerSize) !== spacerSize ? { ...item, spacerSize } : item);
}

export function configureImageWidget(items: WidgetItem[], id: string | undefined, image: ImageWidgetOptions) {
  if (!id) return [...items, { id: crypto.randomUUID(), kind: 'image', image } satisfies WidgetItem];
  const next = updateWidget(items, id, (item) => item.kind === 'image' ? { ...item, image } : item);
  if (next === items) throw new Error('This image widget is no longer on the dock.');
  return next;
}

export function dockPopupAnchor(element: HTMLElement) {
  const { left, top, right, bottom } = element.getBoundingClientRect();
  const body = element.closest('.dock-body');
  if (!body) return { left, top, right, bottom };
  const strip = body.getBoundingClientRect();
  switch (body.parentElement?.dataset.edge) {
    case 'top': return { left, top, right, bottom: strip.bottom };
    case 'left': return { left, top, right: strip.right, bottom };
    case 'right': return { left: strip.left, top, right, bottom };
    case 'bottom': return { left, top: strip.top, right, bottom };
    default: return { left, top, right, bottom };
  }
}

export interface WidgetLayout { revision: number; items: WidgetItem[] }
export interface WidgetTarget { folder?: string; before?: string }
export type WidgetDragSource = { type: 'new'; kind: WidgetKind } | { type: 'item'; id: string }
  | { type: 'app'; app: Pick<DockApp, 'id' | 'name' | 'icon' | 'launchPath'> };
export interface WidgetDrag extends WidgetTarget { source: WidgetDragSource; x: number; y: number; inside: boolean; finished: boolean; cancelled: boolean }

export const initialWidgets: WidgetItem[] = [{ id: 'apps-row', kind: 'apps' }];
const storageKey = 'spacebar.widgets';

export function browserLayout(): WidgetLayout {
  const saved = readBrowserDock().widgets;
  if (saved) return { ...saved, items: migrateWidgetItems(saved.items) };
  let layout: WidgetLayout = { revision: 0, items: initialWidgets };
  try {
    const legacy: WidgetLayout | null = JSON.parse(localStorage.getItem(storageKey) ?? 'null');
    if (legacy) layout = { ...legacy, items: migrateWidgetItems(legacy.items) };
    if (JSON.parse(localStorage.getItem(settingsStorageKey) ?? 'null')?.widgetSize !== 'large') return layout;
    let updated = false;
    function migrate(item: WidgetItem): WidgetItem {
      const assigned = !item.size && !isStartWidget(item.kind) && !['app', 'apps', 'spacer', 'divider'].includes(item.kind);
      if (assigned) updated = true;
      const size = assigned ? 'large' : item.size;
      return { ...item, size, children: item.children?.map(migrate) };
    }
    const items = layout.items.map(migrate);
    if (updated) {
      layout = { revision: layout.revision + 1, items };
      writeBrowserDock({ widgets: layout });
    }
  } catch { /* Keep the layout already read, even if the migration cannot be saved. */ }
  return layout;
}

function currentLayout(): Promise<WidgetLayout> {
  return desktop ? invoke('read_widgets') : Promise.resolve(browserLayout());
}

export function allWidgets(items: WidgetItem[]) { return items.flatMap((item) => [item, ...(item.children ?? [])]); }

export function useWidgets() {
  const [items, setItems] = useState(initialWidgets);
  const [ready, setReady] = useState(false);
  useEffect(() => {
    let alive = true;
    let newest = -1;
    function apply(layout: WidgetLayout) {
      if (alive && layout.revision >= newest) { newest = layout.revision; setItems(layout.items); setReady(true); }
    }
    const onLocalChange = () => { void currentLayout().then(apply); };
    const subscription = desktop ? listen<WidgetLayout>('widgets-changed', ({ payload }) => apply(payload)) : undefined;
    const onStorage = (event: StorageEvent) => { if (event.key === dockStorageKey) onLocalChange(); };
    if (!desktop) {
      window.addEventListener(browserWidgetsChanged, onLocalChange);
      window.addEventListener('storage', onStorage);
    }
    void Promise.resolve(subscription).then(currentLayout).then(apply).catch(console.error);
    return () => {
      alive = false;
      window.removeEventListener(browserWidgetsChanged, onLocalChange);
      window.removeEventListener('storage', onStorage);
      void subscription?.then((unlisten) => unlisten()).catch(console.error);
    };
  }, []);
  return { items, ready };
}

let pendingEdits = Promise.resolve();
export const settledWidgetEdits = () => pendingEdits;

export function editWidgets(edit: (items: WidgetItem[]) => WidgetItem[]) {
  const saved = pendingEdits.then(() => saveWidgetEdit(edit));
  pendingEdits = saved.catch(() => {});
  return saved;
}

async function saveWidgetEdit(edit: (items: WidgetItem[]) => WidgetItem[]) {
  if (!desktop) {
    const current = browserLayout();
    const items = edit(current.items);
    if (items !== current.items) {
      writeBrowserDock({ widgets: { revision: current.revision + 1, items } });
    }
    return;
  }
  for (let attempt = 0; attempt < 4; attempt++) {
    const current = await currentLayout();
    const items = edit(current.items);
    if (items === current.items) return;
    if (await invoke<boolean>('save_widgets', { revision: current.revision, items })) return;
  }
  throw new Error('The dock changed while editing. Try again.');
}

function without(items: WidgetItem[], id: string) {
  return items.filter((item) => item.id !== id).map((item) => item.children?.some((child) => child.id === id)
    ? { ...item, children: item.children.filter((child) => child.id !== id) } : item);
}

function insert(items: WidgetItem[], item: WidgetItem, before?: string) {
  const index = before ? items.findIndex((entry) => entry.id === before) : -1;
  return index < 0 ? [...items, item] : [...items.slice(0, index), item, ...items.slice(index)];
}

export function placeWidget(items: WidgetItem[], source: WidgetDragSource, { folder, before }: WidgetTarget = {}): WidgetItem[] {
  const existing = allWidgets(items);
  const target = folder ? items.find((item) => item.id === folder && item.kind === 'folder') : undefined;
  if (folder && !target) return items;
  let entry: WidgetItem | undefined;
  if (source.type === 'item') entry = existing.find((item) => item.id === source.id);
  if (source.type === 'app') {
    if (!target) return items;
    const { id: appId, name, icon, launchPath } = source.app;
    entry = existing.find((item) => item.kind === 'app' && item.appId === appId) ?? { id: crypto.randomUUID(), kind: 'app', appId, name, icon, launchPath };
  }
  if (source.type === 'new') {
    entry = source.kind === 'folder'
      ? { id: crypto.randomUUID(), kind: 'folder', name: 'Folder', size: 'normal', children: [] }
      : (repeatableWidget(source.kind) ? undefined : existing.find((item) => item.kind === source.kind))
        ?? { id: crypto.randomUUID(), kind: source.kind, size: supportsWidgetSize(source.kind) ? 'normal' : undefined };
  }
  if (!entry || before === entry.id || (target && (entry.kind === 'folder' || isLayoutWidget(entry.kind)))) return items;
  const next = without(items, entry.id);
  if (target) return next.map((item) => item.id === target.id ? { ...item, children: insert(item.children ?? [], entry, before) } : item);
  return entry.kind === 'app' ? next : insert(next, entry, before);
}

export function removeWidget(items: WidgetItem[], id: string): WidgetItem[] {
  const entry = allWidgets(items).find((item) => item.id === id);
  if (!entry) return items;
  return entry.kind === 'folder' ? items.flatMap((item) => item.id === id ? (item.children ?? []).filter((child) => child.kind !== 'app') : [item]) : without(items, id);
}

export function renameFolder(items: WidgetItem[], id: string, name: string) {
  return items.map((item) => item.id === id && item.kind === 'folder' ? { ...item, name: name.trim().slice(0, 48) || 'Folder' } : item);
}

export function rowApps(apps: DockApp[], items: WidgetItem[]) {
  const grouped = new Set(allWidgets(items).filter((item) => item.kind === 'app').map((item) => item.appId));
  return apps.filter((app) => !grouped.has(app.id));
}

export function useWidgetDrag(source: WidgetDragSource | ((event: PointerEvent<HTMLElement>) => WidgetDragSource), enabled = true) {
  const suppressClick = useRef(false);
  const dragCleanup = useRef<() => void>(undefined);
  useEffect(() => () => dragCleanup.current?.(), []);
  function onPointerDown(event: PointerEvent<HTMLElement>) {
    if (!desktop || !enabled || event.button !== 0 || (event.target as HTMLElement).closest('[data-no-drag]')) return;
    suppressClick.current = false;
    const dragged = typeof source === 'function' ? source(event) : source;
    dragCleanup.current?.();
    const { clientX, clientY, pointerId, currentTarget } = event;
    function cleanup() {
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', cleanup);
      window.removeEventListener('pointercancel', cleanup);
      dragCleanup.current = undefined;
    }
    function move(next: globalThis.PointerEvent) {
      if (next.pointerId !== pointerId || Math.hypot(next.clientX - clientX, next.clientY - clientY) < 6) return;
      cleanup();
      suppressClick.current = true;
      try { currentTarget.setPointerCapture(pointerId); } catch { /* Native tracking continues across windows. */ }
      const preview = currentTarget.querySelector('[data-drag-image]')
        ?? (dragged.type === 'item' && currentTarget.matches('.dock-slot') ? currentTarget.closest('.dock-app-row') : null)
        ?? currentTarget.firstElementChild ?? currentTarget;
      const { width, height } = preview.getBoundingClientRect();
      void invoke('begin_widget_drag', { source: dragged, size: { width, height } }).catch(console.error);
    }
    dragCleanup.current = cleanup;
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', cleanup);
    window.addEventListener('pointercancel', cleanup);
  }
  function onClickCapture(event: MouseEvent) {
    if (!suppressClick.current) return;
    suppressClick.current = false;
    event.preventDefault();
    event.stopPropagation();
  }
  return { onPointerDown, onClickCapture };
}
