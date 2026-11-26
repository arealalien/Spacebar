import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { Minus, Orbit, Settings2 } from 'lucide-react';
import { useEffect, useLayoutEffect, useMemo, useRef, useState, type ComponentPropsWithRef, type CSSProperties, type PointerEvent, type ReactNode } from 'react';
import { defaults, demoApps, readSettings, subscribeSettings, verticalDock, type DockApp, type Settings } from './model';
import { followPopupMotion, magnification } from './dockMotion';
import { AppIcon, appIconStyle, DockAppIcon, NotificationBadge, notificationLabel } from './AppIcon';
import { DockEffects, DockSnowCap } from './DockEffects';
import { DockSnow } from './dockSnow';
import { useDockAppearance } from './dockAppearance';
import { appMenuTarget, dockPopupAnchor, middleClick, openDockMenu, widgetMenuTarget } from './DockOverlays';
import { preferredWidgetWidth, WidgetFace } from './WidgetFaces';
import { useDockApps } from './useDockApps';
import { isLayoutWidget, isSpotifyKind, isStartWidget, isWidgetContainer, widgetCatalog } from './widgetCatalog';
import {
  containerLayout, defaultSpacerSize, editWidgets, hasWidgetSurface, placeWidget, removeWidget, rowApps,
  useWidgetDrag, useWidgets, widgetAppearance, widgetColorStyle, widgetNames, widgetScale,
  type WidgetDrag, type WidgetItem, type WidgetKind, type WidgetTarget,
} from './widgets';

function dockMetrics(settings: Settings, items: WidgetItem[], count: number, available = Infinity) {
  const displayed = items.filter((item) => item.kind !== 'apps' || count > 0);
  const vertical = verticalDock(settings.edge);
  const clear = settings.material === 'clear';
  const gear = !settings.hideSettingsIcon || (settings.replaceTaskbar && !displayed.length);
  const divider = !clear && gear && displayed.length > 0;
  const row = displayed.find((item) => item.kind === 'apps');
  const size = settings.iconSize;
  const appSize = size * widgetScale(row?.size);
  const padding = settings.padding;
  const clearGrowth = clear ? 15 : 0;
  const cardHeight = (item: WidgetItem) => size * widgetScale(item.size) + 7 + clearGrowth;
  const isSquare = (kind: WidgetItem['kind']) => kind !== 'app' && widgetCatalog[kind].width === 0;
  const widgetLength = (item: WidgetItem) => {
    const { kind } = item;
    const scale = widgetScale(item.size);
    if (kind === 'spacer') return item.spacerSize ?? defaultSpacerSize;
    if (kind === 'divider') return 8 * scale;
    if (isStartWidget(kind)) return size * scale;
    const height = cardHeight(item);
    if (kind === 'image') return height * (vertical ? 1 : item.image?.crop.ratio ?? 1);
    if (isSquare(kind)) return height;
    if (vertical) return isSpotifyKind(kind) || kind === 'network'
      ? Math.max(65 * scale, height * 1.22) : height;
    return preferredWidgetWidth(kind, size) * height / (size + 7);
  };
  const chrome = 2 * padding + (gear ? 31 : 0) + Number(divider);
  const elements = displayed.length + Number(gear) + Number(divider);
  const gaps = Math.max(0, elements - 1) + (row ? Math.max(0, count - 1) : 0);
  const cards = displayed.filter((item) => item.kind !== 'apps' && !isLayoutWidget(item.kind));
  const room = Math.max(0, available - chrome);
  const spacing = Math.min(settings.spacing, room / Math.max(1, gaps) / 4);
  function cardDepth(item: WidgetItem, factor: number) {
    const { kind } = item;
    if (isStartWidget(kind)) return widgetLength(item) * factor;
    const height = cardHeight(item) * factor;
    if (kind === 'image') return height * (vertical ? item.image?.crop.ratio ?? 1 : 1);
    if (!vertical || isSquare(kind)) return height;
    const scale = widgetScale(item.size);
    if (kind === 'spotify') return Math.max(96 * scale, 112 * scale * factor, height);
    if (kind === 'clock') return Math.max(82 * scale, 100 * scale * factor, height);
    const width = kind === 'app' ? 0 : widgetCatalog[kind].width;
    return Math.max(Math.min(100, width) * scale, Math.min(156, width * .83) * scale * factor, height);
  }
  function containerBox(item: WidgetItem, factor: number) {
    const layout = containerLayout(item);
    const scale = layout.scale * widgetScale(item.size);
    const gap = layout.spacing * widgetScale(item.size) * factor;
    const padding = layout.padding * widgetScale(item.size) * factor;
    const children = (item.children ?? []).filter((child) => child.kind !== 'apps' || count > 0).map((child) => {
      const iconSize = size * widgetScale(child.size) * factor * scale;
      const slotExtra = 7 * scale * factor;
      const apps = child.kind === 'apps';
      const along = apps ? count * iconSize + Math.max(0, count - 1) * gap : widgetLength(child) * factor * scale;
      const across = apps ? iconSize + slotExtra : isLayoutWidget(child.kind) ? 0 : cardDepth(child, factor) * scale;
      return { item: child, along, across, iconSize, slotExtra };
    });
    const innerDepth = children.reduce((depth, child) => Math.max(depth, child.across), 0)
      || cardHeight(item) * layout.scale * factor;
    const fitted = children.map((child) => {
      const stretch = child.item.noPadding && child.across > 0 && child.item.kind !== 'apps';
      return { ...child, along: stretch ? child.along * innerDepth / child.across : child.along,
        across: stretch || !child.across ? innerDepth : child.across };
    });
    const length = fitted.reduce((sum, child) => sum + child.along, 0) + Math.max(0, fitted.length - 1) * gap;
    return { gap, padding, children: fitted, depth: innerDepth + 2 * padding,
      length: (fitted.length ? length : Math.max(innerDepth, widgetCatalog[item.kind as WidgetKind].width * factor)) + 2 * padding };
  }
  function measure(factor: number) {
    const iconSize = appSize * factor;
    const baseDepth = iconSize + (row || (gear && !vertical) ? 7 : 0);
    const containers = new Map(cards.filter((item) => isWidgetContainer(item.kind)).map((item) => [item.id, containerBox(item, factor)]));
    const depthFor = (item: WidgetItem) => containers.get(item.id)?.depth ?? cardDepth(item, factor);
    const contentDepth = cards.reduce((depth, item) => Math.max(depth, depthFor(item)), baseDepth);
    const height = contentDepth + 2 * padding;
    const lengthFor = (item: WidgetItem) => {
      const length = containers.get(item.id)?.length ?? widgetLength(item) * factor;
      return item.noPadding && !isLayoutWidget(item.kind) ? length * height / depthFor(item) : length;
    };
    const length = displayed.reduce((sum, item) => sum + (item.kind === 'apps' ? count * iconSize : lengthFor(item)), 0);
    return { iconSize, contentDepth, height, lengthFor, depthFor, containers, width: chrome + length + spacing * gaps };
  }
  let factor = 1;
  let metrics = measure(factor);
  if (metrics.width > available) {
    // A flush widget's length depends on the final dock depth, so fit both axes together.
    let lower = .12;
    let upper = 1;
    for (let attempt = 0; attempt < 16; attempt++) {
      const candidate = (lower + upper) / 2;
      if (measure(candidate).width <= available) lower = candidate;
      else upper = candidate;
    }
    factor = lower;
    metrics = measure(factor);
  }
  const { iconSize, contentDepth, height, lengthFor, depthFor, containers, width } = metrics;
  function widgetSizeFor(item: WidgetItem) {
    const along = lengthFor(item);
    const depth = isLayoutWidget(item.kind) ? contentDepth : depthFor(item);
    const across = item.noPadding ? height : depth;
    const unit = Math.max(1, Math.min(along, across) - (isStartWidget(item.kind) ? 0 : 7));
    return { width: vertical ? across : along, height: vertical ? along : across, unit };
  }
  function containerSizeFor(item: WidgetItem) {
    const box = containers.get(item.id);
    if (!box) return;
    const scale = item.noPadding ? height / box.depth : 1;
    return { gap: box.gap * scale, padding: box.padding * scale, children: box.children.map((child) => ({
      item: child.item, iconSize: child.iconSize * scale, slotExtra: child.slotExtra * scale,
      width: (vertical ? child.across : child.along) * scale,
      height: (vertical ? child.along : child.across) * scale,
      unit: Math.max(1, Math.min(child.along, child.across) * scale - child.slotExtra * scale),
    })) };
  }
  return {
    displayed, gear, divider, spacing, iconSize,
    width: elements ? width : 0,
    height, widgetSizeFor, containerSizeFor,
  };
}

function AppButton({ app, rowId, preview, editing, ...props }: ComponentPropsWithRef<'button'> & { app: DockApp; rowId: string; preview: boolean; editing: boolean }) {
  const drag = useWidgetDrag((event) => event.shiftKey || editing ? { type: 'item', id: rowId }
    : { type: 'app', app: { id: app.id, name: app.name, icon: app.icon, launchPath: app.launchPath } }, !preview);
  return <button {...props} {...drag} />;
}

function RemoveBadge({ item, onError }: { item: WidgetItem; onError: (error: unknown) => void }) {
  const name = item.name ?? widgetNames[item.kind as WidgetKind];
  const label = isWidgetContainer(item.kind) ? `Ungroup ${name.toLowerCase()}` : `Remove ${name}`;
  return <button type="button" data-no-drag className="dock-remove-widget" aria-label={label} title={label}
    onClick={() => void editWidgets((items) => removeWidget(items, item.id)).catch(onError)}><Minus size={11} strokeWidth={3} /></button>;
}

function WidgetBlock({ item, apps, width, height, unit, shift, preview, active, editing, highlight, snow, content, onError }: {
  item: WidgetItem; apps: DockApp[]; width: number; height: number; unit: number; shift: number; preview: boolean; active: boolean;
  editing: boolean; highlight: string; snow?: DockSnow; content?: ReactNode; onError: (error: unknown) => void;
}) {
  const drag = useWidgetDrag({ type: 'item', id: item.id }, !preview);
  return <div className={`dock-widget dock-widget-${item.kind} ${highlight}`} data-dock-widget={item.id} data-kind={item.kind}
    data-fill-dock={item.noPadding} data-widget-size={item.size ?? 'normal'}
    style={{ ...widgetColorStyle(item), width, height, '--widget-shift': `${shift}px`, '--widget-unit': `${unit}px` } as CSSProperties}
    {...widgetAppearance(item)} {...drag}
    onPointerDown={(event) => {
      if ((event.target as HTMLElement).closest('[data-dock-widget]') === event.currentTarget) drag.onPointerDown(event);
    }}
    onContextMenu={preview ? undefined : (event) => openDockMenu(event, widgetMenuTarget(item), onError)}>
    {content ?? <WidgetFace item={item} apps={apps} specimen={preview} active={active} onError={onError} onFolder={(button) => {
      if (preview) return;
      void invoke('open_folder', { id: item.id, anchor: dockPopupAnchor(button) }).catch(onError);
    }} />}
    {snow && (hasWidgetSurface(item, 'background') || hasWidgetSurface(item, 'frame')) &&
      <DockSnowCap id={'widget:' + item.id} snow={snow} />}
    {editing && <RemoveBadge item={item} onError={onError} />}
  </div>;
}

function previewAnchor(icon: HTMLElement, button: HTMLElement, body: HTMLElement, edge: Settings['edge']) {
  let { left, top, right, bottom } = icon.getBoundingClientRect();
  const slot = button.getBoundingClientRect();
  const bar = body.getBoundingClientRect();
  switch (edge) {
    case 'bottom': top = Math.min(top, bar.top); bottom = Math.max(bottom, slot.bottom); break;
    case 'top': bottom = Math.max(bottom, bar.bottom); top = Math.min(top, slot.top); break;
    case 'left': right = Math.max(right, bar.right); left = Math.min(left, slot.left); break;
    case 'right': left = Math.min(left, bar.left); right = Math.max(right, slot.right); break;
  }
  return { left, top, right, bottom };
}

interface AppDrop {
  anchor: string;
  before: boolean;
}

type DockDropTarget = WidgetTarget & { app?: AppDrop };

const clickEffectDuration = { press: 170, stars: 390, squish: 260, bounce: 320 } as const;
type ClickEffect = keyof typeof clickEffectDuration;
interface ClickPulse {
  id: string;
  effect: ClickEffect;
  sequence: number;
  duration: number;
  squishDuration: number;
}

const starFlights = [
  [-.7, -.32, -65], [-.51, -.66, 75], [-.23, -.9, -94], [.06, -.97, 78],
  [.35, -.82, 99], [.58, -.54, -78], [.71, -.28, 52],
];

function LaunchStars() {
  return <span className="dock-launch-stars" aria-hidden="true">{starFlights.map(([x, y, spin]) =>
    <i key={x} style={{ '--flight-x': x, '--flight-y': y, '--flight-spin': `${spin}deg` } as CSSProperties} />)}</span>;
}

function appDropTarget(row: HTMLElement | undefined, drag: WidgetDrag, id: string, vertical: boolean): AppDrop | undefined {
  if (!row) return;
  const bounds = row.getBoundingClientRect();
  const withinRow = drag.x >= bounds.left - 10 && drag.x <= bounds.right + 10
    && drag.y >= bounds.top - 10 && drag.y <= bounds.bottom + 10;
  if (!withinRow) return;
  const neighbors = [...row.querySelectorAll<HTMLElement>(':scope > [data-app]')].filter((node) => node.dataset.app !== id);
  const leading = neighbors.find((node) => vertical
    ? drag.y < bounds.top + node.offsetTop + node.offsetHeight / 2
    : drag.x < bounds.left + node.offsetLeft + node.offsetWidth / 2);
  const anchor = (leading ?? neighbors.at(-1))?.dataset.app;
  if (anchor) return { anchor, before: !!leading };
}

function dropTarget(body: HTMLElement, drag: WidgetDrag, vertical: boolean): DockDropTarget | undefined {
  const blocksIn = (element: HTMLElement) => [...element.querySelectorAll<HTMLElement>(':scope > [data-dock-widget]')]
    .map((node) => ({ node, bounds: node.getBoundingClientRect() }));
  const blocks = blocksIn(body);
  const source = drag.source;
  const all = [...body.querySelectorAll<HTMLElement>('[data-dock-widget]')];
  const kind = source.type === 'new' ? source.kind
    : source.type === 'item' ? all.find((node) => node.dataset.dockWidget === source.id)?.dataset.kind ?? '' : 'app';
  const main = vertical ? drag.y : drag.x;
  const cross = vertical ? drag.x : drag.y;
  const start = (bounds: DOMRect) => vertical ? bounds.top : bounds.left;
  const length = (bounds: DOMRect) => vertical ? bounds.height : bounds.width;
  const hit = (bounds: DOMRect) => {
    const inset = Math.min(8, length(bounds) * .18);
    return main >= start(bounds) + inset && main <= start(bounds) + length(bounds) - inset
      && cross >= (vertical ? bounds.left : bounds.top) - 8 && cross <= (vertical ? bounds.right : bounds.bottom) + 8;
  };
  const group = blocks.find(({ node, bounds }) => isWidgetContainer(node.dataset.kind ?? '') && hit(bounds));
  const groupFace = group?.node.querySelector<HTMLElement>(':scope > .widget-container-face');
  const children = groupFace ? blocksIn(groupFace) : [];
  const folder = kind !== 'folder' && !isLayoutWidget(kind) && !isWidgetContainer(kind)
    && [...children, ...blocks].find(({ node, bounds }) => node.dataset.kind === 'folder' && hit(bounds));
  if (folder) return { folder: folder.node.dataset.dockWidget };
  if (source.type === 'app') {
    const row = all.find((node) => node.dataset.kind === 'apps');
    const app = appDropTarget(row, drag, source.app.id, vertical);
    if (app) return { app };
    return;
  }
  const intoGroup = group && !isWidgetContainer(kind) && kind !== 'app';
  const before = (intoGroup ? children : blocks).find(({ bounds }) => main < start(bounds) + length(bounds) / 2);
  return { container: intoGroup ? group.node.dataset.dockWidget : undefined, before: before?.node.dataset.dockWidget };
}

export function Dock({ settings, apps, allApps, items, drag, preview = false, editing = false, contextOpen = false, revealed = true, onSettings, onActivate, onDrop, onMoveApp, onMeasure, onError }: {
  settings: Settings;
  apps: DockApp[];
  allApps: DockApp[];
  items: WidgetItem[];
  drag?: WidgetDrag;
  preview?: boolean;
  editing?: boolean;
  contextOpen?: boolean;
  revealed?: boolean;
  onSettings: () => void;
  onActivate: (id: string) => void;
  onDrop?: (drag: WidgetDrag, target: DockDropTarget | undefined) => void;
  onMoveApp?: (id: string, target: AppDrop) => void;
  onMeasure?: (height: number) => void;
  onError: (error: unknown) => void;
}) {
  const [hover, setHover] = useState<{ id: string; position: number }>();
  const [overSettings, setOverSettings] = useState(false);
  const [failedPreviewId, setFailedPreviewId] = useState<string>();
  const [motionAllowed, setMotionAllowed] = useState(() => !matchMedia('(prefers-reduced-motion: reduce)').matches);
  const [available, setAvailable] = useState(0);
  const [demoVisible, setDemoVisible] = useState(false);
  const [clickPulse, setClickPulse] = useState<ClickPulse>();
  const snow = useMemo(() => settings.dockSnow ? new DockSnow() : undefined, [settings.dockSnow]);
  const buttons = useRef(new Map<string, HTMLButtonElement>());
  const frame = useRef<HTMLDivElement>(null);
  const body = useRef<HTMLDivElement>(null);
  const appRow = useRef<HTMLDivElement>(null);
  const positions = useRef(new Map<string, number>());
  const priorOrder = useRef('');
  const { tint, material, ...appearance } = useDockAppearance(settings);
  const clickedApp = useRef<string>(undefined);
  const visible = preview ? !settings.autoHide || demoVisible : revealed;
  const resting = !visible || !!drag || editing || contextOpen;
  const hoverId = overSettings || resting ? undefined : hover?.id;
  const hovered = apps.find((app) => app.id === hoverId);
  const windowSignature = hovered?.windows.length ? JSON.stringify(hovered.windows) : '';
  const vertical = verticalDock(settings.edge);
  const usable = available;
  const { displayed, gear, divider, width, height, spacing, iconSize, widgetSizeFor, containerSizeFor } = dockMetrics(settings, items, apps.length, usable || Infinity);
  const zoom = displayed.some((item) => item.kind === 'apps') && settings.magnify && motionAllowed ? settings.zoom : 1;
  const room = Math.max(0, usable - width);
  const motion = magnification(apps.length, resting ? undefined : hover?.position, iconSize, zoom, settings.falloff, room, room / 2, settings.alignment);
  const playingClick = motionAllowed && visible && !resting && clickPulse?.effect === settings.appIconClickEffect ? clickPulse : undefined;
  const expanded = (hover !== undefined && !resting) || playingClick?.effect === 'stars' || playingClick?.effect === 'bounce';
  const hasDock = width > 0 || !!drag;
  const rowIndex = displayed.findIndex((item) => item.kind === 'apps');
  const target = drag?.inside && body.current ? dropTarget(body.current, drag, vertical) : undefined;
  const draggedApp = drag?.source.type === 'app' ? drag.source.app.id : undefined;
  const highlight = (id: string) => (target?.folder === id || target?.container === id) ? 'drop-folder' : target?.before === id ? 'drop-before' : '';

  useLayoutEffect(() => { onMeasure?.(height); }, [height, onMeasure]);

  useLayoutEffect(() => {
    if (!body.current) return;
    const nodes = [...body.current.querySelectorAll<HTMLElement>(':scope > [data-dock-widget], .dock-app-row > [data-app]')];
    const id = (node: HTMLElement) => node.dataset.app ?? node.dataset.dockWidget!;
    const order = nodes.map(id).join('|');
    const offset = (node: HTMLElement) => vertical ? node.offsetTop : node.offsetLeft;
    const current = new Map(nodes.map((node) => [id(node), offset(node)]));
    if (motionAllowed && priorOrder.current && order !== priorOrder.current) {
      for (const node of nodes) {
        const previous = positions.current.get(id(node));
        if (previous !== undefined && Math.abs(previous - offset(node)) > 1) {
          const delta = `${previous - offset(node)}px`;
          node.animate([{ translate: vertical ? `0 ${delta}` : `${delta} 0` }, { translate: '0px 0' }], { duration: 210, easing: 'cubic-bezier(.2,.75,.25,1)' });
        }
      }
    }
    positions.current = current;
    priorOrder.current = order;
  });

  useEffect(() => {
    if (drag?.finished) onDrop?.(drag, !drag.cancelled && drag.inside && body.current ? dropTarget(body.current, drag, vertical) : undefined);
  }, [drag]);

  useEffect(() => {
    if (visible) return;
    clickedApp.current = undefined;
    setOverSettings(false);
    setHover(undefined);
    setClickPulse(undefined);
  }, [visible]);

  useEffect(() => {
    const query = matchMedia('(prefers-reduced-motion: reduce)');
    function update() { setMotionAllowed(!query.matches); }
    query.addEventListener('change', update);
    return () => query.removeEventListener('change', update);
  }, []);

  useEffect(() => {
    if (!clickPulse) return;
    const timer = window.setTimeout(() => setClickPulse((current) => current === clickPulse ? undefined : current),
      clickPulse.duration);
    return () => window.clearTimeout(timer);
  }, [clickPulse]);

  useEffect(() => {
    if (!frame.current) return;
    const observer = new ResizeObserver(([entry]) => setAvailable(vertical ? entry.contentRect.height : entry.contentRect.width));
    observer.observe(frame.current);
    return () => observer.disconnect();
  }, [hasDock, vertical]);

  useEffect(() => {
    if (preview) return;
    const updateRegion = () => void invoke('set_dock_expanded', { expanded }).catch(console.error);
    if (expanded) { updateRegion(); return; }
    const timer = window.setTimeout(updateRegion, motionAllowed ? settings.duration : 0);
    return () => window.clearTimeout(timer);
  }, [expanded, motionAllowed, preview, settings.duration]);

  useEffect(() => {
    if (preview || !settings.previews || !hoverId || !windowSignature || !visible) return;
    const button = buttons.current.get(hoverId);
    const icon = button?.querySelector<HTMLElement>('.dock-icon-motion');
    const bar = body.current;
    if (!button || !icon || !bar) return;
    const token = crypto.randomUUID();
    const measure = () => ({ anchor: previewAnchor(icon, button, bar, settings.edge) });
    const initial = measure();
    let leaveAt = Infinity;
    let opened = false;
    const leave = () => void invoke('hide_window_previews', { token }).catch(console.error);

    setFailedPreviewId(undefined);
    void invoke('show_window_previews', { token, appId: hoverId, ...initial }).then(() => {
      opened = true;
      if (leaveAt !== Infinity) leave();
      followPopupMotion(
        () => performance.now() < leaveAt && icon.isConnected && bar.isConnected ? measure() : undefined,
        (geometry) => invoke<boolean>('move_window_previews', { token, ...geometry }),
        initial,
      );
    }, () => { if (leaveAt === Infinity) setFailedPreviewId(hoverId); });
    return () => {
      // Keep the anchor moving as magnification settles while the cursor crosses to the live preview.
      leaveAt = performance.now() + Math.max(settings.duration, 210) + 32;
      if (opened) leave();
    };
  }, [hoverId, windowSignature, settings.previews, settings.edge, settings.duration, preview, visible]);

  function trackPointer(event: PointerEvent<HTMLDivElement>) {
    if (resting) { setHover(undefined); return; }
    const hit = (event.target as HTMLElement).closest<HTMLButtonElement>('button');
    if (hit?.classList.contains('dock-settings')) { clickedApp.current = undefined; setOverSettings(true); return; }
    setOverSettings(false);
    const region = (event.target as HTMLElement).closest('[data-dock-widget]');
    if (region && region !== appRow.current) { clickedApp.current = undefined; setHover(undefined); return; }
    let target = hit;
    if (!target) {
      let closest = iconSize + spacing;
      for (const button of buttons.current.values()) {
        const bounds = button.getBoundingClientRect();
        const distance = Math.abs(vertical ? event.clientY - bounds.top - bounds.height / 2 : event.clientX - bounds.left - bounds.width / 2);
        if (distance < closest) { closest = distance; target = button; }
      }
    }
    const id = target?.dataset.app;
    if (!id) { setHover(undefined); return; }
    if (clickedApp.current === id) return;
    clickedApp.current = undefined;
    const first = buttons.current.get(apps[0]?.id);
    const last = buttons.current.get(apps.at(-1)?.id ?? '');
    if (!first || !last || !appRow.current) return;
    // Offset geometry describes the resting row and is unaffected by hover transforms.
    const rowBounds = appRow.current.getBoundingClientRect();
    const rowStart = vertical ? rowBounds.top : rowBounds.left;
    const center = (node: HTMLElement) => vertical ? node.offsetTop + node.offsetHeight / 2 : node.offsetLeft + node.offsetWidth / 2;
    const firstCenter = center(first);
    const lastCenter = center(last);
    const step = (lastCenter - firstCenter) / Math.max(1, apps.length - 1) || 1;
    const position = Math.max(0, Math.min(apps.length - 1, ((vertical ? event.clientY : event.clientX) - rowStart - firstCenter) / step));
    setHover((current) => current?.id === id && Math.abs(current.position - position) < 0.008
      ? current : { id, position });
  }

  function activate(id: string, action = () => onActivate(id)) {
    clickedApp.current = id;
    setHover(undefined);
    const effect = settings.appIconClickEffect ?? 'none';
    if (motionAllowed && effect !== 'none') {
      const rate = (settings.appIconClickSpeed ?? defaults.appIconClickSpeed) / 100;
      setClickPulse((previous) => ({
        id, effect, sequence: (previous?.sequence ?? 0) + 1,
        duration: Math.round(clickEffectDuration[effect] / rate),
        squishDuration: Math.round(clickEffectDuration.squish / rate),
      }));
    }
    action();
  }

  if (!hasDock) return null;
  const bounce = settings.bounceAmount / 100;
  const revealCurve = visible && settings.revealAnimation === 'bounce' && bounce > 0
    ? `linear(0, 0.64 25%, ${1 + bounce * 0.18} 58%, ${1 - bounce * 0.05} 80%, 1)`
    : 'cubic-bezier(.2,.75,.25,1)';
  const style = {
    ...appIconStyle(settings),
    '--dock-tint': tint, '--icon-size': `${iconSize}px`, '--dock-gap': `${spacing}px`,
    '--dock-align': settings.alignment === 'start' ? 'flex-start' : settings.alignment === 'end' ? 'flex-end' : 'center',
    '--dock-padding': `${settings.padding}px`,
    '--dock-duration': `${settings.duration}ms`, '--dock-offset': `${settings.offset}px`, '--end-shift': `${motion.end}px`,
    '--dock-snow-headroom': `${snow && !preview ? Math.min(24, settings.offset) : 0}px`,
    '--surface-shift': `${(motion.end - motion.start) / 2}px`,
    '--surface-scale': width ? 1 + (motion.start + motion.end) / width : 1,
    '--reveal-duration': `${settings.revealDuration}ms`, '--reveal-curve': revealCurve,
  } as CSSProperties;

  function renderAppRow(item: WidgetItem, compactStyle?: CSSProperties) {
    return (<div className={`dock-app-row ${highlight(item.id)}`} key={item.id} ref={appRow} data-dock-widget={item.id} data-kind="apps"
          data-fill-dock={item.noPadding} style={compactStyle ?? (item.noPadding ? (vertical ? { width: height } : { height }) : undefined)} {...widgetAppearance(item)}
          onContextMenu={preview ? undefined : (event) => openDockMenu(event, widgetMenuTarget(item), onError)}>
        {apps.map((app, index) => {
          const { scale, shift } = motion.icons[index];
          const pulse = playingClick?.id === app.id ? playingClick : undefined;
          const insertion = target?.app?.anchor === app.id ? (target.app.before ? 'before' : 'after') : undefined;
          return (
            <AppButton key={app.id} app={app} rowId={item.id} preview={preview} editing={editing} className="dock-slot"
              aria-label={notificationLabel(app.name, app.notification, settings.notificationBadges)} data-app={app.id}
              data-app-insertion={insertion} data-dragged={draggedApp === app.id}
              aria-keyshortcuts={onMoveApp ? (vertical ? 'Alt+ArrowUp Alt+ArrowDown' : 'Alt+ArrowLeft Alt+ArrowRight') : undefined}
              ref={(node) => { if (node) buttons.current.set(app.id, node); else buttons.current.delete(app.id); }}
              onFocus={(event) => {
                if (event.currentTarget.matches(':focus-visible')) {
                  setOverSettings(false);
                  setHover({ id: app.id, position: index });
                }
              }}
              onClick={() => activate(app.id)}
              onKeyDown={(event) => {
                if (!onMoveApp || !event.altKey || event.shiftKey || event.ctrlKey || event.metaKey) return;
                const previous = event.key === (vertical ? 'ArrowUp' : 'ArrowLeft');
                const next = event.key === (vertical ? 'ArrowDown' : 'ArrowRight');
                if (!previous && !next) return;
                event.preventDefault();
                const neighbor = apps[index + (previous ? -1 : 1)];
                if (neighbor && !event.repeat) onMoveApp(app.id, { anchor: neighbor.id, before: previous });
              }}
              {...(!preview && middleClick(() => activate(app.id, () =>
                void invoke('launch_app', { id: app.id }).catch(onError))))}
              onContextMenu={preview ? undefined : (event) => openDockMenu(event, appMenuTarget(app), onError)}
              style={{ '--icon-scale': scale, '--icon-shift': `${shift}px`, '--icon-rise': `${(scale - 1) * 5}px`, zIndex: pulse ? 3 : hover?.id === app.id ? 2 : 1 } as CSSProperties}>
              <span className="dock-icon-motion">
                <span className="dock-click-motion" key={pulse?.sequence ?? 0} data-click-effect={pulse?.effect}
                  style={pulse && { '--app-click-time': `${pulse.duration}ms`, '--app-squish-time': `${pulse.squishDuration}ms` } as CSSProperties}>
                  <span className="dock-click-art">
                    <DockAppIcon app={app} />
                    {snow && draggedApp !== app.id && <DockSnowCap id={'app:' + app.id} snow={snow} />}
                    <NotificationBadge notification={app.notification} mode={settings.notificationBadges} />
                  </span>
                  {pulse?.effect === 'stars' && <LaunchStars />}
                </span>
              </span>
              {settings.indicators && app.windows.length > 0 && <span className={`running-dot ${app.active ? 'active' : ''}`} />}
              {hoverId === app.id && settings.names && (!settings.previews || !app.windows.length || failedPreviewId === app.id) && <span className="dock-tooltip">{app.name}</span>}
              {preview && settings.previews && app.windows.length > 0 && (
                <span className="sample-preview-group" data-open={visible && hoverId === app.id} aria-hidden="true">
                  {app.windows.map((window) => <span key={window.id} className="sample-thumbnail"><span>{window.title}</span><span className={`thumbnail-canvas ${app.id}`}><AppIcon app={app} /></span></span>)}
                </span>
              )}
            </AppButton>
          );
        })}
        {editing && <RemoveBadge item={item} onError={onError} />}
        </div>);
  }

  function renderWidget(item: WidgetItem, shift: number) {
    if (item.kind === 'apps') return renderAppRow(item);
    const group = containerSizeFor(item);
    const props = { apps: allApps, preview, active: visible, editing, snow, onError };
    return <WidgetBlock key={item.id} item={item} {...props} {...widgetSizeFor(item)} shift={shift} highlight={highlight(item.id)}
      content={group && <div className="widget-face widget-container-face" role="group" tabIndex={preview ? undefined : 0}
        aria-label={widgetNames[item.kind as WidgetKind]} style={{ gap: group.gap, padding: group.padding }}>
        {group.children.length ? group.children.map(({ item: child, iconSize, slotExtra, ...box }) => child.kind === 'apps'
          ? renderAppRow(child, { width: box.width, height: box.height, '--icon-size': iconSize + 'px',
            '--dock-gap': group.gap + 'px', '--app-slot-extra': slotExtra + 'px' } as CSSProperties)
          : <WidgetBlock key={child.id} item={child} {...props} {...box} shift={0} highlight={highlight(child.id)} />)
          : <span className="widget-container-empty">Drop widgets here</span>}
      </div>} />;
  }

  return (
    <div ref={frame} className={`dock-frame ${preview ? 'dock-sample' : 'dock-desktop'}`} style={style} data-edge={settings.edge} data-material={material} {...appearance} data-vertical={vertical} data-span={settings.fullSpan} onPointerLeave={() => setDemoVisible(false)}>
      {snow &&
        <DockEffects frame={frame} body={body} settings={settings} visible={visible} motion={motionAllowed} preview={preview} snow={snow} />}
      {preview && settings.autoHide && <button className="demo-edge" aria-label="Reveal dock preview" onPointerEnter={() => setDemoVisible(true)} onFocus={() => setDemoVisible(true)} onClick={() => setDemoVisible(true)} />}
      <div ref={body} className={`dock-body${visible ? '' : ' dock-hidden'}${target && !target.before && !target.folder && !target.container && !target.app ? ' drop-end' : ''}`} inert={!visible} aria-label={preview ? 'Interactive dock preview' : 'Spacebar dock'} data-editing={editing}
        onPointerMove={trackPointer} onPointerLeave={() => { clickedApp.current = undefined; setOverSettings(false); setHover(undefined); }}
        onContextMenu={preview ? undefined : (event) => openDockMenu(event, { type: 'dock' }, onError)}
        onBlur={(event) => { if (!event.currentTarget.contains(event.relatedTarget)) { setOverSettings(false); setHover(undefined); } }}>
        {material !== 'clear' && <span className={`dock-surface material-${material}`} aria-hidden="true">
          {snow && <DockSnowCap id="dock" snow={snow} />}
        </span>}
        {displayed.map((item, index) => renderWidget(item, rowIndex < 0 ? 0 : index < rowIndex ? -motion.start : motion.end))}
        {!width && drag && <span className="dock-empty-drop">Drop a widget here</span>}
        {divider && <span className="dock-divider" />}
        {gear && <button className="dock-settings" onFocus={() => setOverSettings(true)} onClick={onSettings} title="Spacebar settings" aria-label="Spacebar settings">
          {preview ? <Orbit size={22} /> : <Settings2 size={21} />}
        </button>}
      </div>
    </div>
  );
}

export function DesktopDock() {
  const [settings, setSettings] = useState(defaults);
  const { apps, ready, error: appError } = useDockApps();
  const { items, ready: widgetsReady } = useWidgets();
  const [error, setError] = useState('');
  const [drag, setDrag] = useState<WidgetDrag>();
  const [revealed, setRevealed] = useState(false);
  const [editing, setEditing] = useState(false);
  const [contextOpen, setContextOpen] = useState(false);
  const [layoutHeight, setLayoutHeight] = useState<number>();
  const errorTimer = useRef<number>(undefined);
  function reportError(error: unknown) {
    setError(String(error));
    window.clearTimeout(errorTimer.current);
    errorTimer.current = window.setTimeout(() => setError(''), 4200);
  }

  useEffect(() => {
    let disposed = false;
    let changed = false;
    const unlisten = subscribeSettings((value) => { changed = true; if (!disposed) setSettings(value); });
    let revealChanged = false;
    const revealListener = listen<boolean>('dock-revealed', ({ payload }) => { revealChanged = true; if (!disposed) setRevealed(payload); });
    const dragListener = listen<WidgetDrag>('widget-drag', ({ payload }) => { if (!disposed) setDrag(payload); });
    let editChanged = false;
    const editListener = listen<boolean>('widget-editing', ({ payload }) => { editChanged = true; if (!disposed) setEditing(payload); });
    const contextListener = listen<boolean>('context-menu-state', ({ payload }) => { if (!disposed) setContextOpen(payload); });
    void revealListener.then(() => invoke<boolean>('read_dock_revealed')).then((value) => { if (!disposed && !revealChanged) setRevealed(value); }).catch(console.error);
    void editListener.then(() => invoke<boolean>('read_widget_editing')).then((value) => { if (!disposed && !editChanged) setEditing(value); }).catch(console.error);
    void unlisten.then(() => readSettings()).then((value) => { if (!changed && !disposed) setSettings(value); }).catch(console.error);
    return () => { disposed = true; window.clearTimeout(errorTimer.current); for (const listener of [unlisten, revealListener, dragListener, editListener, contextListener]) void listener.then((fn) => fn()).catch(console.error); };
  }, []);

  const visibleApps = rowApps(apps, items);
  const hasAppRow = items.some((item) => item.kind === 'apps'
    || (isWidgetContainer(item.kind) && item.children?.some((child) => child.kind === 'apps')));
  const count = hasAppRow ? visibleApps.length : 0;
  const { width: layoutWidth, iconSize: layoutIconSize } = dockMetrics(settings, items, count);

  useEffect(() => {
    if (!ready || !widgetsReady) return;
    const place = () => void invoke('place_dock', { count, layoutWidth, layoutHeight, layoutIconSize }).catch(reportError);
    place();
    const timer = window.setInterval(place, 2500);
    return () => window.clearInterval(timer);
  }, [count, layoutWidth, layoutHeight, layoutIconSize, settings, ready, widgetsReady]);

  function moveApp(id: string, { anchor, before }: AppDrop) {
    void invoke('move_dock_app', { id, anchor, before }).catch(reportError);
  }

  function finishDrop(drag: WidgetDrag, target: DockDropTarget | undefined) {
    setDrag(undefined);
    if (!target) return;
    const dragged = drag.source;
    if (target.app && dragged.type === 'app') {
      moveApp(dragged.app.id, target.app);
      return;
    }
    const source = dragged.type === 'app' ? { ...dragged, app: apps.find((app) => app.id === dragged.app.id) ?? dragged.app } : dragged;
    void editWidgets((layout) => placeWidget(layout, source, target)).catch(reportError);
  }

  return <>
    {(error || appError) && <p className="dock-error" role="alert">{error || appError}</p>}
    <Dock settings={settings} apps={visibleApps} allApps={apps} items={items} drag={drag} editing={editing} contextOpen={contextOpen} revealed={revealed} onSettings={() => void invoke('open_settings').catch(reportError)}
      onActivate={(id) => void invoke('activate_app', { id, toggle: true }).catch(reportError)}
      onDrop={finishDrop} onMoveApp={moveApp} onMeasure={setLayoutHeight} onError={reportError} />
  </>;
}

export function DemoDock({ settings, items, onSettings }: { settings: Settings; items: WidgetItem[]; onSettings: () => void }) {
  const [apps, setApps] = useState(demoApps);
  return <Dock settings={settings} apps={apps} allApps={apps} items={items} preview onSettings={onSettings} onError={console.error} onActivate={(id) => {
    setApps((current) => current.map((app) => app.id === id ? {
      ...app, active: !app.active, windows: app.windows.length ? app.windows : [{ id, title: app.name }],
    } : { ...app, active: false }));
  }} />;
}
