import '@fontsource-variable/bricolage-grotesque';
import '@fontsource-variable/dm-sans';
import './style.css';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { AppWindow, Check, ChevronRight, CircleAlert, LayoutGrid, Maximize2, Minus, Monitor, Moon, Mouse, Newspaper, Palette, PanelBottom, RotateCcw, Save, Shapes, Sun, SunMoon, X, Zap } from 'lucide-react';
import { lazy, Suspense, useEffect, useId, useLayoutEffect, useRef, useState, type CSSProperties, type ReactNode } from 'react';
import { createRoot } from 'react-dom/client';
import spacebarIcon from '../icon-new.svg';
import { version } from '../package.json';
import { ColorPicker } from './ColorPicker';
import { appIconStyle } from './AppIcon';
import { DemoDock, DesktopDock } from './Dock';
import { DockExclusions } from './DockExclusions';
import { DesktopDockEffects } from './DockEffects';
import { DockContextMenu, DragPreview } from './DockOverlays';
import { WindowPreview } from './WindowPreview';
import { FolderPanel } from './FolderPanel';
import { WidgetLibrary, WidgetsSidebar, type WidgetSection } from './WidgetLibrary';
import { ImageEditorWindow } from './ImageWidget';
import { SystemPanel } from './SystemPanel';
import { WidgetPopup } from './SpotifyWidgets';
import { SavedDocksPage } from './SavedDocksPage';
import { SurfaceSnow } from './SurfaceSnow';
import { UpdateNotice } from './UpdateNotice';
import type { InstalledReleaseNotes } from './WhatsNewPage';
import { editSavedDocks, type SavedDockEdit } from './savedDocks';
import { allWidgets, initialWidgets, useWidgets } from './widgets';
import { accents, appAppearanceAmountRange, appIconBackgrounds, appIconClickEffects, appIconClickSpeedRange, contrastingInk, defaultAppAppearance, defaults, desktop, dockPaddingRange, normalizeHexColor, readAppAppearance, readSettings, verticalDock, writeAppAppearance, writeSettings, type AppAppearance, type Settings } from './model';

const sections = {
  dock: { label: 'Dock layout', icon: PanelBottom, description: 'Position the dock and choose what it shows.' },
  appearance: { label: 'Dock appearance', icon: Palette, description: 'Style the dock and choose when it’s visible.' },
  icons: { label: 'App icons', icon: Shapes, description: 'Style app icons and set click effects.' },
  motion: { label: 'Motion', icon: Zap, description: 'Adjust how app icons respond as you move across the dock.' },
  app: { label: 'App appearance', icon: SunMoon, description: 'Choose a theme, blur, and tint for Settings.' },
  saved: { label: 'Saved docks', icon: Save, description: 'Switch between your saved dock configurations.' },
};
type Section = keyof typeof sections;
const WhatsNewPage = lazy(() => import('./WhatsNewPage'));

const appThemes = [
  { value: 'system', label: 'System default', icon: Monitor },
  { value: 'light', label: 'Light', icon: Sun },
  { value: 'dark', label: 'Dark', icon: Moon },
] as const;

const appTintPresets = [
  accents.red, accents.orange, accents.yellow, accents.green,
  accents.blue, accents.purple, accents.pink, { label: 'Slate', color: '#a4adbb' },
];

function Row({ label, detail, detailId, children }: { label: string; detail?: string; detailId?: string; children: ReactNode }) {
  return <div className="setting-row"><div><div className="setting-label">{label}</div>{detail && <div id={detailId} className="setting-detail">{detail}</div>}</div>{children}</div>;
}

function Slider({ label, detail, value, min, max, step = 1, format, disabled, onChange }: {
  label: string; detail?: string; value: number; min: number; max: number; step?: number;
  format: (value: number) => string; disabled?: boolean; onChange: (value: number) => void;
}) {
  const detailId = useId();
  return <Row label={label} detail={detail} detailId={detailId}><div className="slider-control">
    <div className="slider-field" style={{ '--progress': `${(value - min) / (max - min) * 100}%` } as CSSProperties}>
      <input type="range" aria-label={label} aria-describedby={detail ? detailId : undefined} aria-valuetext={format(value)}
        value={value} min={min} max={max} step={step} disabled={disabled}
        onChange={(event) => onChange(event.currentTarget.valueAsNumber)} />
    </div>
    <output>{format(value)}</output>
  </div></Row>;
}

function Toggle({ label, detail, checked, disabled, onChange }: {
  label: string; detail: string; checked: boolean; disabled?: boolean; onChange: (value: boolean) => void;
}) {
  return <Row label={label} detail={detail}>
    <button type="button" className="toggle" role="switch" aria-label={label} aria-checked={checked}
      disabled={disabled} onClick={() => onChange(!checked)}><span aria-hidden="true" /></button>
  </Row>;
}

function SegmentedSetting<Value extends string>({ label, detail, options, value, disabled, onChange }: {
  label: string;
  detail: string;
  options: readonly (readonly [value: Value, label: string])[];
  value: Value;
  disabled?: boolean;
  onChange: (value: Value) => void;
}) {
  const name = useId();
  const detailId = `${name}-detail`;
  const selectedIndex = options.findIndex(([option]) => option === value);

  return <Row label={label} detail={detail} detailId={detailId}>
    <div className="segmented-control" role="radiogroup" aria-label={label} aria-describedby={detailId}
      aria-disabled={disabled || undefined}
      style={{ '--segment-count': options.length, '--segment-index': selectedIndex } as CSSProperties}>
      <span className="segmented-indicator" aria-hidden="true" hidden={selectedIndex < 0} />
      {options.map(([option, text]) => <label key={option} className="setting-segment">
        <input type="radio" name={name} value={option} checked={value === option} disabled={disabled}
          onChange={() => onChange(option)} />
        <span>{text}</span>
      </label>)}
    </div>
  </Row>;
}

function HexColorControl({ label, value, onChange }: { label: string; value: string; onChange: (color: string) => void }) {
  const errorId = useId();
  const [hex, setHex] = useState<string>();
  const [invalid, setInvalid] = useState(false);

  useEffect(() => {
    setHex(undefined);
    setInvalid(false);
  }, [value]);

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

function AppAppearanceSettings({ appearance, onChange }: {
  appearance: AppAppearance; onChange: (change: Partial<AppAppearance>) => void;
}) {
  const themeName = useId();

  return <div className="settings-groups">
    <section className="setting-group" aria-labelledby="app-theme-heading">
      <h2 id="app-theme-heading">Theme</h2>
      <p className="setting-group-detail" id="app-theme-detail">System default follows Windows appearance.</p>
      <div className="app-theme-options" role="radiogroup" aria-labelledby="app-theme-heading" aria-describedby="app-theme-detail">
        {appThemes.map(({ value, label, icon: Icon }) => <label key={value} className="app-theme-option">
          <input type="radio" name={themeName} value={value} checked={appearance.theme === value}
            onChange={() => onChange({ theme: value })} />
          <span className="app-theme-card">
            <span className="theme-sample" data-sample={value} aria-hidden="true">
              <span className="theme-sample-window">
                <span className="theme-sample-rail"><i /><i /><i /></span>
                <span className="theme-sample-content"><i /><i /><i /></span>
              </span>
            </span>
            <span className="app-theme-caption">
              <Icon size={15} aria-hidden="true" />{label}
              {appearance.theme === value && <Check className="app-theme-check" size={14} aria-hidden="true" />}
            </span>
          </span>
        </label>)}
      </div>
    </section>
    <section className="setting-group" aria-labelledby="app-glass-heading">
      <h2 id="app-glass-heading">Frosted glass</h2>
      <div className="settings-list">
        <Slider label="Blur amount" detail="Choose Off for a solid background."
          value={appearance.blurAmount} {...appAppearanceAmountRange}
          format={(amount) => amount === 0 ? 'Off' : `${amount}%`} onChange={(blurAmount) => onChange({ blurAmount })} />
      </div>
    </section>
    <section className="setting-group" aria-labelledby="app-tint-heading">
      <h2 id="app-tint-heading">Tint color</h2>
      <div className="settings-list">
        <Slider label="Tint amount" detail="Adjusts the background tint."
          value={appearance.tintAmount} {...appAppearanceAmountRange}
          format={(amount) => `${amount}%`} onChange={(tintAmount) => onChange({ tintAmount })} />
        <div className="app-tint-presets" role="group" aria-label="Tint presets">
          {appTintPresets.map(({ label, color }) => <button key={color} type="button"
            aria-pressed={appearance.tintColor === color} onClick={() => onChange({ tintColor: color })}>
            <span className="app-tint-swatch" style={{ '--preset-color': color } as CSSProperties} aria-hidden="true">
              {appearance.tintColor === color && <Check size={15} strokeWidth={2.6} />}
            </span>
            <span>{label}</span>
          </button>)}
        </div>
        <Row label="Custom color" detail="Pick a color or enter a hex value">
          <HexColorControl label="App tint" value={appearance.tintColor} onChange={(tintColor) => onChange({ tintColor })} />
        </Row>
      </div>
    </section>
  </div>;
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
      <Toggle label="Start Spacebar on sign-in" detail="Open the dock when you sign in to Windows"
        checked={enabled ?? false} disabled={enabled === undefined || saving} onChange={(value) => void update(value)} />
    </div>
    {error && <div className="save-status has-error" role="alert"><CircleAlert size={14} />{error}</div>}
  </section>;
}

function SettingsApp() {
  const [tab, setTab] = useState<Section | 'whats-new'>('dock');
  const [release, setRelease] = useState<InstalledReleaseNotes>();
  const [releaseDismiss, setReleaseDismiss] = useState({ pending: false, error: '' });
  const [widgetsOpen, setWidgetsOpen] = useState(false);
  const [widgetSection, setWidgetSection] = useState<WidgetSection>('all');
  const [widgetQuery, setWidgetQuery] = useState('');
  const { items, ready: widgetsReady } = useWidgets();
  const [settings, setSettings] = useState(defaults);
  const [appearance, setAppearance] = useState(defaultAppAppearance);
  const [appearanceLoaded, setAppearanceLoaded] = useState(false);
  const [appearanceSave, setAppearanceSave] = useState({ pending: false, error: '' });
  const [status, setStatus] = useState('');
  const [saving, setSaving] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [resetCount, setResetCount] = useState(0);
  const [savingDock, setSavingDock] = useState(false);
  const saves = useRef(Promise.resolve());
  const revision = useRef(0);
  const draft = useRef(defaults);
  const appearanceRevision = useRef(0);
  const appearanceDraft = useRef(defaultAppAppearance);
  const settingsNav = useRef<HTMLElement>(null);
  const settingsPage = useRef<HTMLElement>(null);
  const section = sections[tab === 'whats-new' ? 'dock' : tab];
  const releaseTitle = release ? 'What’s new in ' + release.version : '';
  const native = desktop ? getCurrentWindow() : undefined;

  useEffect(() => {
    void readSettings().then((value) => { draft.current = value; setSettings(value); })
      .catch((error) => setStatus(String(error))).finally(() => setLoaded(true));
    void readAppAppearance().then((value) => { appearanceDraft.current = value; setAppearance(value); })
      .catch((error) => setAppearanceSave({ pending: false, error: String(error) }))
      .finally(() => setAppearanceLoaded(true));
  }, []);

  useEffect(() => {
    if (!desktop) return;
    let alive = true;
    void invoke<InstalledReleaseNotes | null>('read_whats_new')
      .then((notes) => { if (alive) setRelease(notes ?? undefined); })
      .catch(console.error);
    return () => { alive = false; };
  }, []);

  useLayoutEffect(() => {
    settingsPage.current?.scrollTo({ top: 0 });
  }, [tab]);

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

  function applyAppAppearance(change: Partial<AppAppearance>) {
    const revision = ++appearanceRevision.current;
    const next = { ...appearanceDraft.current, ...change };
    appearanceDraft.current = next;
    setAppearance(next);
    setAppearanceSave({ pending: true, error: '' });
    saves.current = saves.current.then(async () => {
      if (revision !== appearanceRevision.current) return;
      let error = '';
      try { await writeAppAppearance(next); }
      catch (failure) { error = String(failure); }
      if (revision === appearanceRevision.current) setAppearanceSave({ pending: false, error });
    });
  }

  async function dismissRelease(button: HTMLButtonElement) {
    setReleaseDismiss({ pending: true, error: '' });
    try {
      await invoke('dismiss_whats_new');
    } catch {
      setReleaseDismiss({ pending: false, error: 'Couldn’t dismiss this tab. Try again.' });
      return;
    }
    const focused = document.activeElement;
    setRelease(undefined);
    setTab((current) => current === 'whats-new' ? 'dock' : current);
    setReleaseDismiss({ pending: false, error: '' });
    if (focused === button || focused === document.body) {
      window.requestAnimationFrame(() => settingsNav.current?.querySelector<HTMLButtonElement>('[aria-current="page"]')?.focus());
    }
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
  const sideDock = verticalDock(settings.edge);
  const previewHeight = sideDock ? 420 : 118 + settings.iconSize * (settings.magnify ? settings.zoom : 1) + paddingGrowth;
  const appAppearancePage = tab === 'app';
  const pageLoaded = appAppearancePage ? appearanceLoaded : loaded;
  const pageSaving = appAppearancePage ? appearanceSave.pending : saving;
  const pageError = appAppearancePage ? appearanceSave.error : status;

  return <div className="app-shell" data-theme={appearance.theme} data-material={settings.material} data-native={desktop || undefined}
    data-blur-off={appearance.blurAmount === 0 || undefined}
    style={{ ...appIconStyle(settings), '--app-tint': appearance.tintColor, '--app-blur': appearance.blurAmount / 100,
      '--app-tint-strength': appearance.tintAmount / 100,
      '--on-accent': contrastingInk(appearance.tintColor), '--dock-tint': settings.tintColor } as CSSProperties}>
    <header className="titlebar" data-tauri-drag-region>
      {settings.dockSnow && <SurfaceSnow theme={appearance.theme} />}
      <span className="brand" data-tauri-drag-region><img className="brand-icon" src={spacebarIcon} alt="" />Spacebar</span>
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
          query={widgetQuery} onQuery={(query) => { setWidgetQuery(query); setWidgetSection('all'); }} /> : <nav ref={settingsNav} className="settings-nav" aria-label="Settings sections">
          {release && <div className="release-tab">
            <button type="button" className={tab === 'whats-new' ? 'nav-item selected' : 'nav-item'} data-section="whats-new"
              disabled={savingDock} aria-current={tab === 'whats-new' ? 'page' : undefined} onClick={() => setTab('whats-new')}>
              <Newspaper size={16} strokeWidth={1.8} aria-hidden="true" />
              <span className="release-tab-label"><span>What’s new </span><small>in {release.version}</small></span>
            </button>
            <button type="button" className="release-tab-close" aria-label={'Dismiss ' + releaseTitle} title="Dismiss release notes"
              disabled={releaseDismiss.pending} onClick={(event) => void dismissRelease(event.currentTarget)}><X size={15} aria-hidden="true" /></button>
            {releaseDismiss.error && <p className="release-notes-error" role="alert">{releaseDismiss.error}</p>}
          </div>}
          {(Object.keys(sections) as Section[]).map((id) => {
            const { icon: Icon, label } = sections[id];
            return <button key={id} className={`nav-item ${id === tab ? 'selected' : ''}`} data-section={id}
              disabled={savingDock} onClick={() => setTab(id)} aria-current={tab === id ? 'page' : undefined}>
              <Icon size={18} strokeWidth={1.8} /><span>{label}</span>
            </button>;
          })}
          <button className="nav-item" disabled={savingDock} onClick={() => { setWidgetsOpen(true); setWidgetSection('all'); setWidgetQuery(''); }}><LayoutGrid size={18} strokeWidth={1.8} /><span>Widgets</span><ChevronRight className="widget-nav-arrow" size={15} /></button>
        </nav>}
        <div className="sidebar-bottom">
          <UpdateNotice />
          <div className="sidebar-foot"><span className="platform-icon"><AppWindow size={15} /></span><div>Spacebar for Windows<span>Version {version}</span></div></div>
        </div>
      </aside>
      {widgetsOpen ? <WidgetLibrary items={items} ready={widgetsReady} query={widgetQuery} selected={widgetSection} />
        : tab === 'whats-new' && release ? <main ref={settingsPage} className="settings-page" id="settings" aria-labelledby="release-title">
          <div className="page-heading"><h1 id="release-title">{releaseTitle}</h1></div>
          <Suspense fallback={<p role="status">Loading release notes…</p>}><WhatsNewPage release={release} /></Suspense>
        </main> : <main ref={settingsPage} className="settings-page" id="settings" aria-busy={!pageLoaded}>
        <div className="page-heading"><div><h1>{section.label}</h1><p>{section.description}</p></div>
          {tab !== 'saved' && <button className="reset-button" disabled={!pageLoaded} onClick={() => {
            if (appAppearancePage) applyAppAppearance(defaultAppAppearance);
            else apply(defaults);
            setResetCount((count) => count + 1);
          }}><RotateCcw size={15} />{appAppearancePage ? 'Reset appearance' : 'Reset defaults'}</button>}
        </div>
        {tab === 'saved' && <SavedDocksPage ready={loaded && widgetsReady} busy={savingDock} onEdit={editDock} />}
        {appAppearancePage && appearanceLoaded && <AppAppearanceSettings key={resetCount} appearance={appearance} onChange={applyAppAppearance} />}
        {!appAppearancePage && <section className="preview-section" aria-label="Dock preview">
          <div className="preview-bar"><span className="preview-title"><span />Dock preview</span>
            <span className="preview-hint"><Mouse size={13} />
              {tab === 'icons' ? 'Click an app icon to try its effect' : settings.autoHide ? `Hover the ${settings.edge} edge` : 'Hover to try it'}</span>
          </div>
          <div className="preview-scene" style={{ '--preview-height': `${previewHeight}px` } as CSSProperties}>
            <div className="orbital-plane" aria-hidden="true"><i /><i /><i /></div>
            <div className="preview-dock-area"><DemoDock settings={tab === 'icons' ? { ...settings, autoHide: false } : settings}
              items={tab === 'icons' ? initialWidgets : items} onSettings={() => { if (!savingDock) setTab('dock'); }} /></div>
          </div>
        </section>}
        {loaded && tab !== 'saved' && !appAppearancePage && <div className="settings-groups" key={tab}>
          {tab === 'dock' && <>
            <section className="setting-group dock-placement-settings" aria-labelledby="placement"><h2 id="placement">Placement</h2><div className="settings-list">
              <Row label="Screen edge" detail="Choose where the dock sits">
                <select className="setting-select" aria-label="Screen edge" value={settings.edge} onChange={(event) => update('edge', event.currentTarget.value as Settings['edge'])}>
                  <option value="bottom">Bottom</option><option value="top">Top</option><option value="left">Left</option><option value="right">Right</option>
                </select>
              </Row>
              <SegmentedSetting label="Alignment" detail="Position items along the selected edge" value={settings.alignment}
                options={[
                  ['start', sideDock ? 'Top' : 'Left'], ['center', 'Center'], ['end', sideDock ? 'Bottom' : 'Right'],
                ]}
                onChange={(value) => update('alignment', value)} />
              <Toggle label={sideDock ? 'Full height' : 'Full width'} detail="Extend the dock across the available edge" checked={settings.fullSpan} onChange={(value) => update('fullSpan', value)} />
            </div></section>
            <section className="setting-group" aria-labelledby="dimensions"><h2 id="dimensions">Size and spacing</h2><div className="settings-list">
              <Slider label="Icon size" detail="Base size for app icons and widgets" min={20} max={80} value={settings.iconSize} format={px} onChange={(value) => update('iconSize', value)} />
              <Slider label="Icon spacing" detail="Gap between apps" min={0} max={32} value={settings.spacing} format={px} onChange={(value) => update('spacing', value)} />
              <Slider label="Dock padding" detail="Space inside the dock around apps and widgets" {...dockPaddingRange}
                value={settings.padding} format={px} onChange={(value) => update('padding', value)} />
              <Slider label="Edge offset" detail="Space between the dock and screen edge" min={0} max={60} value={settings.offset} format={px} onChange={(value) => update('offset', value)} />
            </div></section>
            <section className="setting-group" aria-labelledby="feedback"><h2 id="feedback">App feedback</h2><div className="settings-list">
              <Toggle label="App names" detail="Show labels on hover" checked={settings.names} onChange={(value) => update('names', value)} />
              <Toggle label="Window previews" detail="Hover a thumbnail for a full-size view" checked={settings.previews} onChange={(value) => update('previews', value)} />
              <SegmentedSetting label="Full-size preview location" value={settings.previewDisplay}
                detail={settings.previews ? 'Choose a monitor for the full-size preview' : 'Turn on Window previews to choose a monitor'}
                options={[['window', 'Window’s monitor'], ['dock', 'Dock’s monitor']]} disabled={!settings.previews}
                onChange={(value) => update('previewDisplay', value)} />
              <Toggle label="Running indicators" detail="Mark apps with open windows" checked={settings.indicators} onChange={(value) => update('indicators', value)} />
              <SegmentedSetting label="Notification badges" detail="Show unread activity on app icons" value={settings.notificationBadges}
                options={[['none', 'Off'], ['dot', 'Dot'], ['count', 'Count']]}
                onChange={(value) => update('notificationBadges', value)} />
            </div></section>
            {desktop && <StartupSetting />}
          </>}
          {tab === 'motion' && <>
            <section className="setting-group" aria-labelledby="zoom"><h2 id="zoom">Hover magnification</h2><div className="settings-list">
              <Toggle label="Magnify icons" detail="Enlarge apps near the pointer" checked={settings.magnify} onChange={(value) => update('magnify', value)} />
              <Slider label="Maximum size" detail="Largest icon size on hover" min={1} max={3} step={0.05} value={settings.zoom} format={percent} disabled={!settings.magnify} onChange={(value) => update('zoom', value)} />
              <Slider label="Magnification reach" detail="How far the effect spreads to nearby icons" min={0.5} max={5} step={0.1} value={settings.falloff} format={(value) => `${value.toFixed(1)}×`} disabled={!settings.magnify} onChange={(value) => update('falloff', value)} />
              <Slider label="Animation duration" detail="Time to reach the new size" min={60} max={1000} step={10} value={settings.duration} format={milliseconds} disabled={!settings.magnify} onChange={(value) => update('duration', value)} />
            </div></section>
          </>}
          {tab === 'icons' && <>
            <section className="setting-group" aria-labelledby="icon-style"><h2 id="icon-style">Icon style</h2><div className="settings-list">
              <SegmentedSetting label="Icon shape" detail="Change the outline of compatible icons and backgrounds" value={settings.appIconShape}
                options={[['original', 'Original'], ['rounded', 'Rounded'], ['circle', 'Circular']]}
                onChange={(value) => update('appIconShape', value)} />
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
            <section className="setting-group" aria-labelledby="icon-clicks"><h2 id="icon-clicks">Click behavior</h2><div className="settings-list">
              <Row label="Click effect" detail="Play an animation when you click an app icon">
                <select className="setting-select" aria-label="App icon click effect" value={settings.appIconClickEffect}
                  onChange={(event) => update('appIconClickEffect', event.currentTarget.value as Settings['appIconClickEffect'])}>
                  {Object.entries(appIconClickEffects).map(([value, label]) => <option key={value} value={value}>{label}</option>)}
                </select>
              </Row>
              <Slider label="Click speed" detail={settings.appIconClickEffect === 'none' ? 'Choose a click effect to adjust its speed' : 'How quickly the click effect plays'}
                value={settings.appIconClickSpeed ?? defaults.appIconClickSpeed} {...appIconClickSpeedRange} step={5}
                format={(value) => `${value}%`} disabled={settings.appIconClickEffect === 'none'}
                onChange={(value) => update('appIconClickSpeed', value)} />
            </div></section>
          </>}
          {tab === 'appearance' && <>
            <section className="setting-group" aria-labelledby="material">
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
            <section className="setting-group" aria-labelledby="dock-effects"><h2 id="dock-effects">Effects</h2><div className="settings-list">
              <Toggle label="Snow" detail="Let snow settle on the dock, menus, popups, and Settings"
                checked={settings.dockSnow} onChange={(value) => update('dockSnow', value)} />
            </div></section>
            <section className="setting-group" aria-labelledby="visibility"><h2 id="visibility">Dock visibility</h2><div className="settings-list">
              <Toggle label="Replace Windows taskbar" detail="Show Spacebar while it’s running" checked={settings.replaceTaskbar} onChange={(value) => update('replaceTaskbar', value)} />
              {allWidgets(items).some((item) => item.kind === 'spacebar-start') &&
                <Toggle label="Replace Windows Start menu" detail="Also use Spacebar Start for Windows Start buttons"
                  checked={settings.replaceWindowsStart} onChange={(value) => update('replaceWindowsStart', value)} />}
              <Toggle label="Hide settings icon" detail="Open Settings from the dock menu" checked={settings.hideSettingsIcon} onChange={(value) => update('hideSettingsIcon', value)} />
              <Toggle label="Hide in fullscreen" detail="Hide the dock while an app fills the dock’s display"
                checked={settings.hideInFullscreen} onChange={(value) => update('hideInFullscreen', value)} />
            </div></section>
            {desktop && <DockExclusions />}
            <section className="setting-group" aria-labelledby="reveal"><h2 id="reveal">Auto-hide</h2><div className="settings-list">
              <Toggle label="Automatically hide dock" detail="Reveal it from the selected screen edge" checked={settings.autoHide} onChange={(value) => update('autoHide', value)} />
              <SegmentedSetting label="Reveal animation" value={settings.revealAnimation}
                detail={settings.autoHide ? 'Choose how the dock slides in and out' : 'Turn on auto-hide to choose an animation'}
                options={[['easing', 'Smooth'], ['bounce', 'Bounce']]} disabled={!settings.autoHide}
                onChange={(value) => update('revealAnimation', value)} />
              <Slider label="Reveal duration" detail="Time to show or hide the dock" min={60} max={1000} step={10} value={settings.revealDuration} format={milliseconds} disabled={!settings.autoHide} onChange={(value) => update('revealDuration', value)} />
              {settings.revealAnimation === 'bounce' && <Slider label="Bounce amount" detail="How far the dock overshoots" min={0} max={100} value={settings.bounceAmount} format={(value) => `${value}%`} disabled={!settings.autoHide} onChange={(value) => update('bounceAmount', value)} />}
            </div></section>
          </>}
        </div>}
        {(pageError || tab !== 'saved') && <div className={`save-status ${pageError ? 'has-error' : ''}`} role="status">{pageError ? <><CircleAlert size={14} />{pageError}</> : <><Check size={14} />{!pageLoaded ? 'Loading preferences…' : pageSaving ? 'Saving preferences…' : 'Changes save automatically'}</>}</div>}
      </main>}
    </div>
  </div>;
}

const params = new URLSearchParams(location.search);
const surface = params.get('surface');
document.documentElement.dataset.surface = surface ?? 'settings';

createRoot(document.getElementById('root')!).render(surface === 'dock' && desktop ? <DesktopDock />
  : surface === 'dock-effects' && desktop ? <DesktopDockEffects />
  : surface === 'preview' ? <WindowPreview /> : surface === 'peek' ? null
  : surface === 'folder' && desktop ? <FolderPanel />
  : surface === 'context' && desktop ? <DockContextMenu />
  : surface === 'system' && desktop ? <SystemPanel />
  : surface === 'widget-popup' && desktop ? <WidgetPopup />
  : surface === 'image-editor' ? <ImageEditorWindow widgetId={params.get('widget')} />
  : surface === 'drag' && desktop ? <DragPreview /> : <SettingsApp />);
