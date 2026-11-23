import { invoke } from '@tauri-apps/api/core';
import { Check, CircleMinus, Image as ImageIcon, LayoutGrid, LogOut, Pin, Settings2, Square, SquareX, X } from 'lucide-react';
import { useLayoutEffect, useRef, useState, type CSSProperties, type MouseEvent } from 'react';
import { AppIcon, appIconStyle, DockAppIcon } from './AppIcon';
import { ColorPicker } from './ColorPicker';
import { SurfaceSnow } from './SurfaceSnow';
import { WidgetSpecimen } from './WidgetFaces';
import { useDockApps } from './useDockApps';
import { useSurfacePayload } from './useSurfacePayload';
import { defaults, normalizeHexColor, type DockApp, type Settings } from './model';
import {
  allWidgets, audioDefaults, defaultSpacerSize, dockPopupAnchor, editWidgets, hasWidgetSurface, maxSpacerSize,
  minSpacerSize, removeWidget, resizeSpacer, rowApps, setWidgetColor,
  setAudioOption, setWidgetSize, supportsWidgetSize, supportsWidgetSurface, toggleWidgetSurface,
  useWidgets, widgetAppearance, widgetColorOptions, widgetColorStyle, widgetColorValue, widgetNames,
  type AudioVisualizerOptions, type WidgetDragSource, type WidgetItem, type WidgetKind, type WidgetSize,
} from './widgets';
export { dockPopupAnchor } from './widgets';

type MenuTarget = { type: 'dock' } | { type: 'app'; id: string; name: string; icon?: string; pinned: boolean; can_pin: boolean; folder_entry_id?: string; running?: boolean; can_end_task?: boolean }
  | { type: 'widget'; id: string; name: string; kind: WidgetItem['kind'] };

export function appMenuTarget(app: Pick<DockApp, 'id' | 'name' | 'icon' | 'pinned' | 'launchPath'>, folderEntryId?: string): MenuTarget {
  return { type: 'app', id: app.id, name: app.name, icon: app.icon, pinned: !!app.pinned, can_pin: !!app.launchPath, folder_entry_id: folderEntryId };
}

export function widgetMenuTarget(item: WidgetItem): MenuTarget {
  return { type: 'widget', id: item.id, name: item.name ?? widgetNames[item.kind as WidgetKind], kind: item.kind };
}

export function openDockMenu(event: MouseEvent<HTMLElement>, target: MenuTarget, onError: (error: unknown) => void) {
  event.preventDefault();
  event.stopPropagation();
  void invoke('open_context_menu', { target, anchor: dockPopupAnchor(event.currentTarget) }).catch(onError);
}

export function middleClick(action: () => void) {
  return {
    onMouseDown(event: MouseEvent<HTMLButtonElement>) {
      if (event.button === 1) event.preventDefault();
    },
    onAuxClick(event: MouseEvent<HTMLButtonElement>) {
      if (event.button !== 1) return;
      event.preventDefault();
      action();
    },
  };
}

const sizeLabels: Record<WidgetSize, string> = { small: 'Small', normal: 'Normal', large: 'Large' };
const audioOptions = [
  { key: 'audioAlignment', label: 'Bar alignment', choices: { center: 'Centered', bottom: 'Bottom' } },
  { key: 'audioSpeed', label: 'Response speed', choices: { slow: 'Slow', normal: 'Normal', fast: 'Fast' } },
  { key: 'audioSensitivity', label: 'Sensitivity', choices: { low: 'Low', normal: 'Normal', high: 'High' } },
] as const;

function WidgetColorControls({ name, label, defaultColor, color, onChange }: {
  name: string; label: string; defaultColor: string; color?: string; onChange: (color: string | undefined) => void;
}) {
  const pickerId = name + '-color-picker';
  const errorId = name + '-color-error';
  const activeColor = normalizeHexColor(color);
  const [hex, setHex] = useState(activeColor?.toUpperCase() ?? '');
  const [invalid, setInvalid] = useState(false);

  function editHex(next: string) {
    setHex(next);
    setInvalid(false);
  }

  function choose(next: string | undefined) {
    editHex(next?.toUpperCase() ?? '');
    onChange(next);
  }

  return <form className="context-widget-options" onSubmit={(event) => {
    event.preventDefault();
    const next = normalizeHexColor(hex);
    if (next) choose(next);
    else setInvalid(true);
  }}>
    <div className="context-color-heading">
      <label htmlFor={pickerId}>{label}</label>
      <button type="button" className="context-default-color" aria-label={name === 'accent' ? 'Default color' : `Default ${label} color`}
        aria-pressed={!activeColor} onClick={() => choose(undefined)}>Default</button>
    </div>
    <div className="context-option-input">
      <ColorPicker id={pickerId} label={name === 'accent' ? 'Widget color' : `${label} color`} value={normalizeHexColor(hex) ?? activeColor ?? defaultColor}
        onInput={(value) => editHex(value.toUpperCase())} onCommit={choose} />
      <input type="text" aria-label={name === 'accent' ? 'Hex color' : `${label} hex color`} placeholder="#RRGGBB" autoComplete="off" spellCheck={false}
        aria-invalid={invalid || undefined} aria-describedby={invalid ? errorId : undefined} value={hex} maxLength={12}
        onChange={(event) => editHex(event.currentTarget.value)} />
      <button type="submit" aria-label={name === 'accent' ? 'Apply hex color' : `Apply ${label} hex color`}>Apply</button>
    </div>
    {invalid && <span id={errorId} role="alert">Enter a 3- or 6-digit hex color.</span>}
  </form>;
}

export function DockContextMenu() {
  const target = useSurfacePayload<MenuTarget>('context-menu-opened', 'read_context_menu');
  const settings = useSurfacePayload<Settings>('settings-changed', 'read_settings');
  const { items } = useWidgets();
  const widgetTarget = target?.type === 'widget' ? target : undefined;
  const widget = widgetTarget ? allWidgets(items).find((item) => item.id === widgetTarget.id) : undefined;
  const actionRole = widgetTarget ? undefined : 'menuitem';
  const menu = useRef<HTMLElement>(null);
  const [error, setError] = useState('');
  useLayoutEffect(() => { setError(''); menu.current?.querySelector('button')?.focus(); }, [target]);
  if (!target) return null;

  async function run(action: () => Promise<unknown>, close = true) {
    setError('');
    try {
      await action();
      if (close) await invoke('close_context_menu');
    } catch (error) { setError(String(error)); }
  }

  return <main ref={menu} className="dock-context" role={widgetTarget ? 'dialog' : 'menu'}
    aria-label={widgetTarget ? `${widgetTarget.name} widget options` : 'Dock actions'} onContextMenu={(event) => event.preventDefault()} onKeyDown={(event) => {
    if (event.key === 'Escape') { void invoke('close_context_menu'); return; }
    if (widgetTarget) return;
    const buttons = [...event.currentTarget.querySelectorAll<HTMLButtonElement>('button:enabled')];
    const current = buttons.indexOf(document.activeElement as HTMLButtonElement);
    const next = event.key === 'ArrowDown' ? (current + 1) % buttons.length : event.key === 'ArrowUp' ? (current + buttons.length - 1) % buttons.length
      : event.key === 'Home' ? 0 : event.key === 'End' ? buttons.length - 1 : -1;
    if (next >= 0) { event.preventDefault(); buttons[next].focus(); }
  }}>
    {settings?.dockSnow && <SurfaceSnow />}
    <button type="button" role={actionRole} onClick={() => void run(() => invoke('open_settings', { widgets: true }))}><LayoutGrid size={15} />Widgets</button>
    <button type="button" role={actionRole} onClick={() => void run(() => invoke('open_settings'))}><Settings2 size={15} />Settings</button>
    {target.type === 'app' && <div className="dock-context-launch">
      <button type="button" role="menuitem" aria-label={`Open ${target.name}`} onClick={() => void run(() => target.folder_entry_id
        ? invoke('launch_folder_app', { id: target.folder_entry_id, newWindow: true }) : invoke('launch_app', { id: target.id }))}>
        <span><AppIcon app={target} /></span><span className="dock-context-name">{target.name}</span>
      </button>
    </div>}
    {widgetTarget && <div className="dock-context-target">{widgetTarget.name}</div>}
    {widgetTarget?.kind === 'image' && <button type="button" onClick={() => void run(() => invoke('open_image_editor', { id: widgetTarget.id }))}>
      <ImageIcon size={15} />Edit image
    </button>}
    {widget && supportsWidgetSize(widget.kind) && <div className="context-widget-options" role="group" aria-labelledby="widget-size-label">
      <span id="widget-size-label">Size</span>
      <div className="context-size-options">
        {(Object.entries(sizeLabels) as [WidgetSize, string][]).map(([size, label]) => <button key={size} type="button"
          aria-pressed={(widget.size ?? 'normal') === size}
          onClick={() => void run(() => editWidgets((items) => setWidgetSize(items, widget.id, size)), false)}>{label}</button>)}
      </div>
    </div>}
    {widget && widgetColorOptions(widget.kind).map((option) => <WidgetColorControls key={`${widget.id}:${option.key}`}
      name={option.key} label={option.label} defaultColor={option.defaultColor} color={widgetColorValue(widget, option)}
      onChange={(color) => void run(() => editWidgets((items) => setWidgetColor(items, widget.id, option.key, color)), false)} />)}
    {widget?.kind === 'audio-visualizer' && audioOptions.map(({ key, label, choices }) => <div key={key}
      className="context-widget-options" role="group" aria-labelledby={`${key}-label`}>
      <span id={`${key}-label`}>{label}</span>
      <div className="context-size-options">
        {(Object.entries(choices) as [AudioVisualizerOptions[typeof key], string][]).map(([value, choice]) => <button key={value} type="button"
          aria-pressed={(widget[key] ?? audioDefaults[key]) === value}
          onClick={() => void run(() => editWidgets((items) => setAudioOption(items, widget.id, key, value)), false)}>{choice}</button>)}
      </div>
    </div>)}
    {widget && supportsWidgetSurface(widget.kind) && (['background', 'frame'] as const).map((option) => {
      const checked = hasWidgetSurface(widget, option);
      const Icon = checked ? Check : Square;
      return <button key={option} type="button" aria-pressed={checked}
        onClick={() => void run(() => editWidgets((items) => toggleWidgetSurface(items, widget.id, option)), false)}>
        <Icon size={15} />{option === 'background' ? 'Show background' : 'Show frame'}
      </button>;
    })}
    {widget?.kind === 'spacer' && <form key={widget.id} className="context-widget-options" onSubmit={(event) => {
      event.preventDefault();
      const size = Number(new FormData(event.currentTarget).get('size'));
      void run(() => editWidgets((items) => resizeSpacer(items, widget.id, size)));
    }}>
      <label htmlFor="spacer-size">Spacer size</label>
      <div className="context-option-input"><input id="spacer-size" name="size" type="number" required min={minSpacerSize} max={maxSpacerSize}
        defaultValue={widget.spacerSize ?? defaultSpacerSize} aria-describedby="spacer-unit" />
        <span id="spacer-unit">px</span><button type="submit">Apply</button></div>
    </form>}
    {widgetTarget && <button type="button" className="context-danger" onClick={() => void run(() => editWidgets((items) => removeWidget(items, widgetTarget.id)))}><CircleMinus size={15} />Remove widget</button>}
    {target.type === 'app' && (target.pinned || target.can_pin) && <button type="button" role="menuitem" onClick={() => void run(() => invoke('set_app_pinned', { id: target.id, pinned: !target.pinned }))}>
      {target.pinned ? <CircleMinus size={15} /> : <Pin size={15} />}{target.pinned ? 'Unpin from dock' : 'Pin to dock'}
    </button>}
    {target.type === 'app' ? target.running && <div className="dock-context-group">
      <button type="button" role="menuitem" onClick={() => void run(() => invoke('execute_app_menu_action', { id: target.id, action: 'close' }))}>
        <X size={15} />Close app
      </button>
      <button type="button" className="context-danger" role="menuitem" disabled={!target.can_end_task}
        title={target.can_end_task ? 'Force the app to close' : 'This process is shared or protected'}
        onClick={() => void run(() => invoke('execute_app_menu_action', { id: target.id, action: 'end-task' }))}>
        <SquareX size={15} />End task
      </button>
    </div> : <button type="button" className="context-quit" role={actionRole} onClick={() => void run(() => invoke('quit_spacebar'), false)}><LogOut size={15} />Quit Spacebar</button>}
    {error && <p className="context-error" role="alert">{error}</p>}
  </main>;
}

export function DragPreview() {
  const source = useSurfacePayload<WidgetDragSource>('drag-preview', 'read_drag_preview');
  const settings = useSurfacePayload<Settings>('settings-changed', 'read_settings') ?? defaults;
  const { items } = useWidgets();
  const item = source?.type === 'item' ? allWidgets(items).find((item) => item.id === source.id) : undefined;
  const kind = source?.type === 'new' ? source.kind : item?.kind;
  const { apps } = useDockApps(kind === 'apps');
  if (!source) return null;
  const app = source.type === 'app' ? source.app : item?.kind === 'app' ? { id: item.appId ?? '', icon: item.icon } : undefined;
  return <div className={`drag-preview${app ? ' drag-preview-app' : ''}`} data-material={settings.material} aria-hidden="true"
    style={{ ...widgetColorStyle(item), ...appIconStyle(settings), '--dock-tint': settings.tintColor } as CSSProperties} {...(item && widgetAppearance(item))}>
    {app ? <DockAppIcon app={app} /> : kind && kind !== 'app' && <WidgetSpecimen kind={kind} apps={kind === 'apps' ? rowApps(apps, items) : apps} item={item} live />}
  </div>;
}
