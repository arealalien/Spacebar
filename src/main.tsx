import '@fontsource-variable/bricolage-grotesque';
import '@fontsource-variable/dm-sans';
import './style.css';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { AppWindow, Check, ChevronRight, CircleAlert, LayoutGrid, Maximize2, Minus, Mouse, Orbit, Palette, PanelBottom, RotateCcw, Save, Shapes, X, Zap } from 'lucide-react';
import { useEffect, useId, useRef, useState, type CSSProperties, type ReactNode } from 'react';
import { createRoot } from 'react-dom/client';
import { version } from '../package.json';
import { ColorPicker } from './ColorPicker';
import { appIconStyle } from './AppIcon';
import { DemoDock, DesktopDock } from './Dock';
import { DockContextMenu, DragPreview } from './DockOverlays';
import { WindowPreview } from './WindowPreview';
import { FolderPanel } from './FolderPanel';
import { WidgetLibrary, WidgetsSidebar, type WidgetSection } from './WidgetLibrary';
import { SystemPanel } from './SystemPanel';
import { WidgetPopup } from './SpotifyWidgets';
import { SavedDocksPage } from './SavedDocksPage';
import { UpdateNotice } from './UpdateNotice';
import { editSavedDocks, type SavedDockEdit } from './savedDocks';
import { allWidgets, initialWidgets, useWidgets } from './widgets';
import { accents, appIconBackgrounds, appIconClickEffects, appIconClickSpeedRange, appIconShapes, defaults, desktop, dockPaddingRange, normalizeHexColor, readSettings, verticalDock, writeSettings, type Settings } from './model';

const sections = {
  dock: { label: 'Dock layout', icon: PanelBottom, description: 'Set the dock size, spacing, and what appears when you hover.' },
  motion: { label: 'Motion', icon: Zap, description: 'Tune how icons respond as you move across the dock.' },
  appearance: { label: 'Appearance', icon: Palette, description: 'Customize how the dock looks and when it appears.' },
  icons: { label: 'App icons', icon: Shapes, description: 'Choose how app icons look and what they do when you click.' },
  saved: { label: 'Saved docks', icon: Save, description: 'Switch between your saved dock configurations.' },
};
type Section = keyof typeof sections;

function Row({ label, detail, children }: { label: string; detail?: string; children: ReactNode }) {
  return <div className="setting-row"><div><div className="setting-label">{label}</div>{detail && <div className="setting-detail">{detail}</div>}</div>{children}</div>;
}

function Slider({ label, detail, value, min, max, step = 1, format, disabled, onChange }: {
  label: string; detail?: string; value: number; min: number; max: number; step?: number;
  format: (value: number) => string; disabled?: boolean; onChange: (value: number) => void;
}) {
  return <Row label={label} detail={detail}><div className="slider-control">
    <input type="range" aria-label={label} aria-valuetext={format(value)} value={value} min={min} max={max} step={step} disabled={disabled}
      style={{ '--progress': `${(value - min) / (max - min) * 100}%` } as CSSProperties}
      onChange={(event) => onChange(event.currentTarget.valueAsNumber)} />
    <output>{format(value)}</output>
  </div></Row>;
}

function Toggle({ label, detail, checked, onChange }: { label: string; detail: string; checked: boolean; onChange: (value: boolean) => void }) {
  return <Row label={label} detail={detail}>
    <button type="button" className="toggle" role="switch" aria-label={label} aria-checked={checked} onClick={() => onChange(!checked)}><span /></button>
  </Row>;
}

function HexColorControl({ label, value, onChange }: { label: string; value: string; onChange: (color: string) => void }) {
  const errorId = useId();
  const [hex, setHex] = useState<string>();
  const [invalid, setInvalid] = useState(false);

  function choose(color: string) {
    setHex(undefined);
    setInvalid(false);
    if (color !== value) onChange(color);
  }

  return <form className="tint-control" onSubmit={(event) => {
    event.preventDefault();
    const color = normalizeHexColor(hex ?? value);
    if (color) choose(color);
    else setInvalid(true);
  }}>
    <div className="tint-inputs">
      <ColorPicker label={`${label} color`} value={value} onInput={choose} />
      <input className="tint-hex" type="text" aria-label={`${label} hex color`} placeholder="#RRGGBB" autoComplete="off" spellCheck={false}
        value={hex ?? value.toUpperCase()} aria-invalid={invalid || undefined} aria-describedby={invalid ? errorId : undefined}
        onChange={(event) => { setHex(event.currentTarget.value); setInvalid(false); }} />
      <button type="submit">Apply</button>
    </div>
    {invalid && <span id={errorId} role="alert">Enter a 3- or 6-digit hex color.</span>}
  </form>;
}

function StartupSetting() {
  const [enabled, setEnabled] = useState<boolean>();
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');

  useEffect(() => {
    let disposed = false;
    void invoke<boolean>('read_startup').then((value) => {
      if (!disposed) setEnabled(value);
    }).catch((error) => { if (!disposed) setError(String(error)); });
    return () => { disposed = true; };
  }, []);

  async function update(enabled: boolean) {
    setSaving(true);
    setError('');
    try { setEnabled(await invoke<boolean>('set_startup', { enabled })); }
    catch (error) { setError(String(error)); }
    finally { setSaving(false); }
  }

  return <section className="setting-group" aria-labelledby="startup"><h2 id="startup">Startup</h2>
    <div className="settings-list">
      <Row label="Start Spacebar on sign-in" detail="Open the dock when you sign in to Windows">
        <input type="checkbox" className="startup-checkbox" aria-label="Start Spacebar on sign-in"
          checked={enabled ?? false} disabled={enabled === undefined || saving}
          onChange={(event) => void update(event.currentTarget.checked)} />
      </Row>
    </div>
    {error && <div className="save-status has-error" role="alert"><CircleAlert size={14} />{error}</div>}
  </section>;
}

function SettingsApp() {
  const [tab, setTab] = useState<Section>('dock');
  const [widgetsOpen, setWidgetsOpen] = useState(false);
  const [widgetSection, setWidgetSection] = useState<WidgetSection>('all');
  const [widgetQuery, setWidgetQuery] = useState('');
  const { items, ready: widgetsReady } = useWidgets();
  const [settings, setSettings] = useState(defaults);
  const [status, setStatus] = useState('');
  const [saving, setSaving] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [resetCount, setResetCount] = useState(0);
  const [savingDock, setSavingDock] = useState(false);
  const saves = useRef(Promise.resolve());
  const revision = useRef(0);
  const draft = useRef(defaults);
  const section = sections[tab];
  const native = desktop ? getCurrentWindow() : undefined;

  useEffect(() => {
    void readSettings().then((value) => { draft.current = value; setSettings(value); })
      .catch((error) => setStatus(String(error))).finally(() => setLoaded(true));
  }, []);

  useEffect(() => {
    if (!desktop) return;
    let disposed = false;
    let changed = false;
    function showPage(widgets: boolean) {
      if (disposed) return;
      setWidgetsOpen(widgets);
      if (widgets) { setWidgetQuery(''); setWidgetSection('all'); }
    }
    const subscription = listen<boolean>('settings-page', ({ payload }) => {
      changed = true;
      showPage(payload);
    });
    void subscription.then(() => invoke<boolean>('read_settings_page')).then((widgets) => {
      if (!changed && typeof widgets === 'boolean') showPage(widgets);
    }).catch(console.error);
    return () => { disposed = true; void subscription.then((unlisten) => unlisten()).catch(console.error); };
  }, []);

  useEffect(() => {
    if (!native) return;
    const setEditing = (editing: boolean) => void invoke('set_widget_editing', { editing }).catch(console.error);
    setEditing(widgetsOpen);
    const subscription = native.onFocusChanged(({ payload: focused }) => { if (focused) setEditing(widgetsOpen); });
    return () => { void subscription.then((unlisten) => unlisten()).catch(console.error); };
  }, [widgetsOpen]);

  function apply(next: Settings) {
    const version = ++revision.current;
    draft.current = next;
    setSettings(next);
    setSaving(true);
    setStatus('');
    saves.current = saves.current.then(async () => {
      if (version !== revision.current) return;
      try { await writeSettings(next); }
      catch (error) { if (version === revision.current) setStatus(String(error)); }
      finally { if (version === revision.current) setSaving(false); }
    });
  }

  function update<K extends keyof Settings>(key: K, value: Settings[K]) {
    apply({ ...draft.current, [key]: value });
  }

  async function editDock(edit: SavedDockEdit) {
    setSavingDock(true);
    try {
      await saves.current;
      const result = await editSavedDocks(edit);
      if (result.settings) {
        draft.current = result.settings;
        setSettings(result.settings);
        setStatus('');
      }
      return result.list;
    } finally { setSavingDock(false); }
  }

  const px = (value: number) => `${value} px`;
  const percent = (value: number) => `${Math.round(value * 100)}%`;
  const milliseconds = (value: number) => value === 1000 ? '1 s' : `${value} ms`;
  const paddingGrowth = 2 * Math.max(0, settings.padding - defaults.padding);
  const previewHeight = verticalDock(settings.edge) ? 420 : 149 + settings.iconSize * (settings.magnify ? settings.zoom : 1) + paddingGrowth;

  return <div className="app-shell" data-theme={settings.accent} data-material={settings.material}
    style={{ ...appIconStyle(settings), '--theme-color': accents[settings.accent].color, '--dock-tint': settings.tintColor } as CSSProperties}>
    <header className="titlebar" data-tauri-drag-region>
      <span className="brand" data-tauri-drag-region><span className="brand-icon"><Orbit size={20} strokeWidth={1.9} /></span>Spacebar</span>
      <span className="title-divider" /><span className="window-label" data-tauri-drag-region>Settings</span>
      {native && <div className="window-actions">
        <button aria-label="Minimize" onClick={() => void native.minimize().then(() => invoke('set_widget_editing', { editing: false }))}><Minus size={15} /></button>
        <button aria-label="Toggle maximize" onClick={() => void native.toggleMaximize()}><Maximize2 size={13} /></button>
        <button className="close-button" aria-label="Close settings" onClick={() => void native.close()}><X size={16} /></button>
      </div>}
    </header>
    <div className="workspace">
      <aside className={`sidebar${widgetsOpen ? ' widgets-sidebar' : ''}`}>
        {widgetsOpen ? <WidgetsSidebar onBack={() => setWidgetsOpen(false)} section={widgetSection} onSection={(id) => { setWidgetSection(id); setWidgetQuery(''); }}
          query={widgetQuery} onQuery={(query) => { setWidgetQuery(query); setWidgetSection('all'); }} /> : <nav aria-label="Settings sections">
          <button className="nav-item" disabled={savingDock} onClick={() => { setWidgetsOpen(true); setWidgetSection('all'); setWidgetQuery(''); }}><LayoutGrid size={18} strokeWidth={1.8} /><span>Widgets</span><ChevronRight className="widget-nav-arrow" size={15} /></button>
          {(Object.keys(sections) as Section[]).map((id) => {
            const { icon: Icon, label } = sections[id];
            return <button key={id} className={`nav-item ${id === tab ? 'selected' : ''}`} disabled={savingDock} onClick={() => setTab(id)} aria-current={tab === id ? 'page' : undefined}>
              <Icon size={18} strokeWidth={1.8} /><span>{label}</span><ChevronRight className="nav-arrow" size={15} />
            </button>;
          })}
        </nav>}
        <div className="sidebar-bottom">
          <UpdateNotice />
          <div className="sidebar-foot"><span className="platform-icon"><AppWindow size={15} /></span><div>Spacebar for Windows<span>Version {version}</span></div></div>
        </div>
      </aside>
      {widgetsOpen ? <WidgetLibrary items={items} ready={widgetsReady} query={widgetQuery} selected={widgetSection} /> : <main className="settings-page" id="settings" aria-busy={!loaded}>
        <div className="page-heading"><div><h1>{section.label}</h1><p>{section.description}</p></div>
          {tab !== 'saved' && <button className="reset-button" disabled={!loaded} onClick={() => {
            apply(defaults);
            setResetCount((count) => count + 1);
          }}><RotateCcw size={15} />Reset defaults</button>}
        </div>
        {tab === 'saved' && <SavedDocksPage ready={loaded && widgetsReady} busy={savingDock} onEdit={editDock} />}
        <section className="preview-section" aria-label="Dock preview">
          <div className="preview-bar"><span className="preview-title"><span />Dock preview</span>
            {tab === 'icons' ? <div className="preview-click-options">
              <span className="preview-hint"><Mouse size={13} />Click an app to try it</span>
              <div className="preview-click-effect">
                <label htmlFor="app-click-effect">Click effect</label>
                <select id="app-click-effect" className="setting-select" aria-label="App icon click effect" value={settings.appIconClickEffect} disabled={!loaded}
                  onChange={(event) => update('appIconClickEffect', event.currentTarget.value as Settings['appIconClickEffect'])}>
                  {Object.entries(appIconClickEffects).map(([value, label]) => <option key={value} value={value}>{label}</option>)}
                </select>
              </div>
              <Slider label="Click speed" value={settings.appIconClickSpeed ?? defaults.appIconClickSpeed}
                {...appIconClickSpeedRange} step={5} format={(value) => `${value}%`} disabled={!loaded || settings.appIconClickEffect === 'none'}
                onChange={(value) => update('appIconClickSpeed', value)} />
            </div> : <span className="preview-hint"><Mouse size={13} />
              {settings.autoHide ? `Hover the ${settings.edge} edge` : 'Hover to try it'}</span>}
          </div>
          <div className="preview-scene" style={{ '--preview-height': `${previewHeight}px` } as CSSProperties}>
            <div className="orbital-plane" aria-hidden="true"><i /><i /><i /></div>
            <div className="preview-dock-area"><DemoDock settings={tab === 'icons' ? { ...settings, autoHide: false } : settings}
              items={tab === 'icons' ? initialWidgets : items} onSettings={() => { if (!savingDock) setTab('dock'); }} /></div>
          </div>
        </section>
        {loaded && tab !== 'saved' && <div className="settings-groups" key={tab}>
          {tab === 'dock' && <>
            <section className="setting-group dock-placement-settings" aria-labelledby="placement"><h2 id="placement">Placement</h2><div className="settings-list">
              <Row label="Screen edge" detail="Choose where the dock sits">
                <select className="setting-select" aria-label="Screen edge" value={settings.edge} onChange={(event) => update('edge', event.currentTarget.value as Settings['edge'])}>
                  <option value="bottom">Bottom</option><option value="top">Top</option><option value="left">Left</option><option value="right">Right</option>
                </select>
              </Row>
              <Row label="Alignment" detail="Position items along the selected edge">
                <select className="setting-select" aria-label="Alignment" value={settings.alignment} onChange={(event) => update('alignment', event.currentTarget.value as Settings['alignment'])}>
                  <option value="start">{verticalDock(settings.edge) ? 'Top' : 'Left'}</option><option value="center">Middle</option><option value="end">{verticalDock(settings.edge) ? 'Bottom' : 'Right'}</option>
                </select>
              </Row>
              <Toggle label={verticalDock(settings.edge) ? 'Full height' : 'Full width'} detail="Extend the dock across the available edge" checked={settings.fullSpan} onChange={(value) => update('fullSpan', value)} />
            </div></section>
            <section className="setting-group" aria-labelledby="dimensions"><h2 id="dimensions">Dimensions</h2><div className="settings-list">
              <Slider label="Icon size" detail="Base size for app icons and widgets" min={20} max={80} value={settings.iconSize} format={px} onChange={(value) => update('iconSize', value)} />
              <Slider label="Icon spacing" detail="Gap between apps" min={0} max={32} value={settings.spacing} format={px} onChange={(value) => update('spacing', value)} />
              <Slider label="Dock padding" detail="Space inside the dock around apps and widgets" {...dockPaddingRange}
                value={settings.padding} format={px} onChange={(value) => update('padding', value)} />
              <Slider label="Edge offset" detail="Space between the dock and screen edge" min={0} max={60} value={settings.offset} format={px} onChange={(value) => update('offset', value)} />
            </div></section>
            <section className="setting-group" aria-labelledby="feedback"><h2 id="feedback">App feedback</h2><div className="settings-list">
              <Toggle label="App names" detail="Show labels on hover" checked={settings.names} onChange={(value) => update('names', value)} />
              <Toggle label="Window previews" detail="Hover a thumbnail for a full-size view" checked={settings.previews} onChange={(value) => update('previews', value)} />
              <Row label="Full-size preview location" detail="Monitor used when you hover a window thumbnail">
                <select className="setting-select" aria-label="Full-size preview location" value={settings.previewDisplay} disabled={!settings.previews}
                  onChange={(event) => update('previewDisplay', event.currentTarget.value as Settings['previewDisplay'])}>
                  <option value="window">Window’s monitor</option>
                  <option value="dock">Dock’s monitor</option>
                </select>
              </Row>
              <Toggle label="Running indicators" detail="Mark apps with open windows" checked={settings.indicators} onChange={(value) => update('indicators', value)} />
              <Row label="Notification badges" detail="Show unread notifications on app icons">
                <select className="setting-select" aria-label="Notification badges" value={settings.notificationBadges}
                  onChange={(event) => update('notificationBadges', event.currentTarget.value as Settings['notificationBadges'])}>
                  <option value="dot">Dot</option><option value="none">Nothing</option><option value="count">Dot with number</option>
                </select>
              </Row>
            </div></section>
            {desktop && <StartupSetting />}
          </>}
          {tab === 'motion' && <>
            <section className="setting-group" aria-labelledby="zoom"><h2 id="zoom">Hover magnification</h2><div className="settings-list">
              <Toggle label="Magnify icons" detail="Enlarge apps near the pointer" checked={settings.magnify} onChange={(value) => update('magnify', value)} />
              <Slider label="Intensity" detail="Maximum icon size on hover" min={1} max={3} step={0.05} value={settings.zoom} format={percent} disabled={!settings.magnify} onChange={(value) => update('zoom', value)} />
            </div></section>
            <section className="setting-group" aria-labelledby="response"><h2 id="response">Response</h2><div className="settings-list">
              <Slider label="Falloff" detail="How far magnification spreads" min={0.5} max={5} step={0.1} value={settings.falloff} format={(value) => `${value.toFixed(1)}×`} disabled={!settings.magnify} onChange={(value) => update('falloff', value)} />
              <Slider label="Animation duration" detail="Time to reach the new size" min={60} max={1000} step={10} value={settings.duration} format={milliseconds} disabled={!settings.magnify} onChange={(value) => update('duration', value)} />
            </div></section>
          </>}
          {tab === 'icons' &&
            <section className="setting-group" aria-labelledby="icon-style"><h2 id="icon-style">Icon style</h2><div className="settings-list">
              <Row label="Shape" detail="Change the outline of compatible icons and icon backgrounds">
                <select className="setting-select" aria-label="App icon shape" value={settings.appIconShape}
                  onChange={(event) => update('appIconShape', event.currentTarget.value as Settings['appIconShape'])}>
                  {Object.entries(appIconShapes).map(([value, label]) => <option key={value} value={value}>{label}</option>)}
                </select>
              </Row>
              <Row label="Background" detail="Add a color behind app icons">
                <select className="setting-select" aria-label="App icon background" value={settings.appIconBackground}
                  onChange={(event) => update('appIconBackground', event.currentTarget.value as Settings['appIconBackground'])}>
                  {Object.entries(appIconBackgrounds).map(([value, label]) => <option key={value} value={value}>{label}</option>)}
                </select>
              </Row>
              {settings.appIconBackground === 'custom' && <Row label="Custom color" detail="Pick a background color or enter hex">
                <HexColorControl key={resetCount} label="App icon background" value={settings.appIconColor}
                  onChange={(value) => update('appIconColor', value)} />
              </Row>}
            </div></section>
          }
          {tab === 'appearance' && <>
            <section className="setting-group material-group" aria-labelledby="material">
              <h2 id="material">Dock material</h2>
              <div className="material-layout">
                <div className="material-options" role="group" aria-label="Dock material">
                  {(['tinted', 'midnight', 'clear'] as const).map((material) => <button aria-pressed={settings.material === material} key={material} onClick={() => update('material', material)}>
                    <span className="material-swatch" aria-hidden="true"><span className={`material-${material}`}><i /><i /><i /></span></span><span>{material}</span>
                  </button>)}
                </div>
                <div className="settings-list tint-settings">
                  <Row label="Tint color" detail="Colors previews and running indicators, and dock and widget surfaces in Tinted mode">
                    <HexColorControl key={resetCount} label="Tint" value={settings.tintColor} onChange={(value) => update('tintColor', value)} />
                  </Row>
                </div>
              </div>
            </section>
            <section className="setting-group" aria-labelledby="accent"><h2 id="accent">Accent color</h2>
              <div className="settings-list accent-options" role="group" aria-label="Settings and preview wallpaper accent">
                {(Object.keys(accents) as Settings['accent'][]).map((accent) => <button key={accent}
                  aria-pressed={settings.accent === accent} onClick={() => update('accent', accent)}>
                  <span className="accent-swatch" style={{ background: accents[accent].color, color: accent === 'dark' ? 'white' : '#17181d' }} aria-hidden="true">
                    {settings.accent === accent && <Check size={13} strokeWidth={2.6} />}
                  </span>{accents[accent].label}
                </button>)}
              </div>
            </section>
            <section className="setting-group" aria-labelledby="visibility"><h2 id="visibility">Dock visibility</h2><div className="settings-list">
              <Toggle label="Replace Windows taskbar" detail="Show Spacebar while it’s running" checked={settings.replaceTaskbar} onChange={(value) => update('replaceTaskbar', value)} />
              {allWidgets(items).some((item) => item.kind === 'spacebar-start') &&
                <Toggle label="Replace Windows Start menu" detail="Also use Spacebar Start for Windows Start buttons"
                  checked={settings.replaceWindowsStart} onChange={(value) => update('replaceWindowsStart', value)} />}
              <Toggle label="Hide settings icon" detail="Open Settings from the dock menu" checked={settings.hideSettingsIcon} onChange={(value) => update('hideSettingsIcon', value)} />
              <Toggle label="Automatically hide dock" detail="Reveal it from the selected screen edge" checked={settings.autoHide} onChange={(value) => update('autoHide', value)} />
            </div></section>
            <section className="setting-group" aria-labelledby="reveal"><h2 id="reveal">Reveal animation</h2><div className="settings-list">
              <Row label="Animation type" detail="Motion as the dock slides in and out">
                <select className="setting-select" aria-label="Reveal animation type" value={settings.revealAnimation} disabled={!settings.autoHide} onChange={(event) => update('revealAnimation', event.currentTarget.value as Settings['revealAnimation'])}>
                  <option value="easing">Easing</option><option value="bounce">Bounce</option>
                </select>
              </Row>
              <Slider label="Reveal duration" detail="Time to show or hide the dock" min={60} max={1000} step={10} value={settings.revealDuration} format={milliseconds} disabled={!settings.autoHide} onChange={(value) => update('revealDuration', value)} />
              {settings.revealAnimation === 'bounce' && <Slider label="Bounce amount" detail="How far the dock overshoots" min={0} max={100} value={settings.bounceAmount} format={(value) => `${value}%`} disabled={!settings.autoHide} onChange={(value) => update('bounceAmount', value)} />}
            </div></section>
          </>}
        </div>}
        {(status || tab !== 'saved') && <div className={`save-status ${status ? 'has-error' : ''}`} role="status">{status ? <><CircleAlert size={14} />{status}</> : <><Check size={14} />{!loaded ? 'Loading preferences…' : saving ? 'Saving preferences…' : 'Changes save automatically'}</>}</div>}
      </main>}
    </div>
  </div>;
}

const surface = new URLSearchParams(location.search).get('surface');
if (surface) document.documentElement.dataset.surface = surface;

createRoot(document.getElementById('root')!).render(surface === 'dock' && desktop ? <DesktopDock />
  : surface === 'preview' ? <WindowPreview /> : surface === 'peek' ? null
  : surface === 'folder' && desktop ? <FolderPanel />
  : surface === 'context' && desktop ? <DockContextMenu />
  : surface === 'system' && desktop ? <SystemPanel />
  : surface === 'widget-popup' && desktop ? <WidgetPopup />
  : surface === 'drag' && desktop ? <DragPreview /> : <SettingsApp />);
