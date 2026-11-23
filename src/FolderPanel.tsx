import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { ArrowUpFromLine, FolderOpen, Plus } from 'lucide-react';
import { useEffect, useState, type CSSProperties } from 'react';
import { AppIcon, appIconStyle, DockAppIcon, NotificationBadge, notificationLabel } from './AppIcon';
import { appMenuTarget, middleClick, openDockMenu, widgetMenuTarget } from './DockOverlays';
import { SurfaceSnow } from './SurfaceSnow';
import { WidgetFace } from './WidgetFaces';
import { useDockApps } from './useDockApps';
import { useSurfacePayload } from './useSurfacePayload';
import { editWidgets, placeWidget, renameFolder, rowApps, useWidgetDrag, useWidgets, widgetAppearance, widgetColorStyle, widgetNames, widgetScale, type WidgetItem, type WidgetKind } from './widgets';
import { defaults, type DockApp, type Settings } from './model';
import { widgetCatalog } from './widgetCatalog';

function FolderEntry({ item, apps, ungroupedApps, settings, onError }: {
  item: WidgetItem; apps: DockApp[]; ungroupedApps: DockApp[]; settings: Settings; onError: (error: unknown) => void;
}) {
  const badgeMode = settings.notificationBadges;
  const drag = useWidgetDrag({ type: 'item', id: item.id });
  const app = item.kind === 'app' ? apps.find((app) => app.id === item.appId) : undefined;
  const name = app?.name ?? item.name ?? widgetNames[item.kind as WidgetKind] ?? 'App';
  const imageRatio = item.image?.crop.ratio ?? 1;
  const wide = item.kind !== 'app' && (widgetCatalog[item.kind].width >= 140 || item.kind === 'apps' || imageRatio > 1);
  const scale = widgetScale(item.size);
  const faceStyle = {
    ...widgetColorStyle(item), '--folder-widget-height': `${48 * scale}px`,
    '--folder-app-size': `${40 * scale}px`, '--widget-unit': `${38 * scale}px`,
    '--image-ratio': imageRatio,
  } as CSSProperties;
  return <div className={`folder-entry folder-entry-${item.kind}${wide ? ' folder-entry-wide' : ''}`} {...drag} onContextMenu={(event) => openDockMenu(event,
    item.kind === 'app' ? appMenuTarget(app ?? { id: item.appId ?? '', name, icon: item.icon }, item.id) : widgetMenuTarget(item), onError)}>
    <div className="folder-entry-face" style={faceStyle}
      data-square={item.kind !== 'app' && item.kind !== 'apps' && item.kind !== 'image' && widgetCatalog[item.kind].width === 0}
      {...widgetAppearance(item)}>
      {item.kind === 'app' ? <button className="folder-app" aria-label={`Open ${notificationLabel(name, app?.notification, badgeMode)}`} title={name} onClick={() => void invoke('launch_folder_app', { id: item.id }).catch(onError)}
        {...middleClick(() => void invoke('launch_folder_app', { id: item.id, newWindow: true }).catch(onError))}>
        <DockAppIcon app={app ?? { id: item.appId ?? '', icon: item.icon }} />
        <NotificationBadge notification={app?.notification} mode={badgeMode} />
        {settings.indicators && app?.windows.length ? <span className={`running-dot ${app.active ? 'active' : ''}`} /> : null}
      </button> : item.kind === 'apps' ? <div className="folder-app-row" aria-label="App icons">
        {ungroupedApps.map((app) => <button key={app.id} className="folder-app" data-no-drag aria-label={`Open ${notificationLabel(app.name, app.notification, badgeMode)}`} title={app.name}
          onContextMenu={(event) => openDockMenu(event, appMenuTarget(app), onError)}
          {...middleClick(() => void invoke('launch_app', { id: app.id }).catch(onError))}
          onClick={() => void invoke('activate_app', { id: app.id }).catch(onError)}><DockAppIcon app={app} /><NotificationBadge notification={app.notification} mode={badgeMode} /></button>)}
        {ungroupedApps.length === 0 && <span className="folder-note">No apps in this row</span>}
      </div> : <WidgetFace item={item} apps={apps} onError={onError} />}
    </div>
    <div className="folder-entry-caption"><span>{name}</span><button type="button" data-no-drag aria-label={`Move ${name} out of folder`}
      title={item.kind === 'app' ? 'Return to app icons' : 'Move to dock'} onClick={() => void editWidgets((layout) => placeWidget(layout, { type: 'item', id: item.id })).catch(onError)}><ArrowUpFromLine size={13} /></button></div>
  </div>;
}

export function FolderPanel() {
  const [id, setId] = useState<string>();
  const [name, setName] = useState('Folder');
  const [adding, setAdding] = useState(false);
  const [query, setQuery] = useState('');
  const [error, setError] = useState('');
  const { items } = useWidgets();
  const { apps } = useDockApps(!!id);
  const settings = useSurfacePayload<Settings>('settings-changed', 'read_settings') ?? defaults;
  const folder = items.find((item) => item.id === id && item.kind === 'folder');
  const children = folder?.children ?? [];
  const grouped = new Set(children.filter((item) => item.kind === 'app').map((item) => item.appId));
  const candidates = apps.filter((app) => !grouped.has(app.id) && app.name.toLowerCase().includes(query.trim().toLowerCase()));
  const report = (error: unknown) => setError(String(error));

  useEffect(() => {
    let gone = false;
    let changed = false;
    const open = listen<string>('folder-opened', ({ payload }) => {
      changed = true;
      if (!gone) { setId(payload); setAdding(false); setQuery(''); setError(''); }
    });
    const close = listen('folder-closed', () => { changed = true; if (!gone) setId(undefined); });
    void Promise.all([open, close]).then(() => invoke<string | null>('read_open_folder')).then((current) => {
      if (!gone && !changed) setId(current ?? undefined);
    }).catch(console.error);
    return () => { gone = true; void Promise.all([open, close]).then((listeners) => listeners.forEach((unlisten) => unlisten())).catch(console.error); };
  }, []);

  useEffect(() => { setName(folder?.name ?? 'Folder'); }, [id, folder?.name]);

  function saveName() {
    if (id && name.trim() !== (folder?.name ?? 'Folder')) void editWidgets((items) => renameFolder(items, id, name)).catch(report);
  }

  return <main className="folder-panel" data-material={settings.material} style={{ ...appIconStyle(settings), '--dock-tint': settings.tintColor } as CSSProperties} onKeyDown={(event) => {
    if (event.key === 'Escape') void invoke('close_folder').catch(report);
  }}>
    {id && settings.dockSnow && <SurfaceSnow />}
    <header className="folder-header"><FolderOpen size={20} />
      <input aria-label="Folder name" value={name} maxLength={48} onChange={(event) => setName(event.currentTarget.value)} onBlur={saveName}
        onKeyDown={(event) => { if (event.key === 'Enter') event.currentTarget.blur(); }} />
    </header>
    <div className="folder-content">
      {error && <p className="widget-message" role="alert">{error}</p>}
      {adding ? <>
        <input className="folder-search" aria-label="Find an app" placeholder="Find an app" value={query} onChange={(event) => setQuery(event.currentTarget.value)} />
        <div className="folder-picker">
          {candidates.map((app) => <button key={app.id} title={app.name} onClick={() => {
            if (id) void editWidgets((items) => placeWidget(items, { type: 'app', app }, { folder: id })).catch(report);
          }}><AppIcon app={app} /><span>{app.name}</span></button>)}
          {candidates.length === 0 && <p className="folder-note">No apps to add.</p>}
        </div>
      </> : <>
        {!children.length && <p className="folder-empty">Drag an app or widget onto this folder, or add apps below.</p>}
        <div className="folder-grid">{children.map((item) => <FolderEntry key={item.id} item={item} apps={apps} ungroupedApps={rowApps(apps, items)} settings={settings} onError={report} />)}</div>
      </>}
    </div>
    <footer className="folder-footer"><button className="reset-button" onClick={() => { setAdding(!adding); setQuery(''); }}>{adding ? 'Back to folder' : <><Plus size={14} />Add apps</>}</button></footer>
  </main>;
}
