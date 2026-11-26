import { ArrowLeft, Check, Clock3, Columns2, Cpu, FolderOpen, Grid2X2, Headphones, LayoutGrid, NotebookPen, Search } from 'lucide-react';
import { useState, type CSSProperties } from 'react';
import { WidgetSpecimen } from './WidgetFaces';
import { openImageEditor } from './ImageWidget';
import { demoApps, desktop, type DockApp } from './model';
import { useDockApps } from './useDockApps';
import { allWidgets, editWidgets, placeWidget, rowApps, useWidgetDrag, widgetAppearance, widgetColorStyle, widgetNames, type WidgetItem, type WidgetKind } from './widgets';
import { clockKinds, repeatableWidget, spotifyKinds, weatherKinds, widgetCatalog } from './widgetCatalog';

const widgetSections = [
  { id: 'clocks', name: 'Clocks', icon: Clock3, kinds: clockKinds },
  { id: 'personal', name: 'Personal', icon: NotebookPen, kinds: ['quick-note', 'drawing-pad', 'agenda', 'quick-tasks', 'color-picker', ...weatherKinds] as const },
  { id: 'media', name: 'Media & audio', icon: Headphones, kinds: ['image', ...spotifyKinds, 'volume', 'microphone', 'mixer', 'audio-visualizer'] as const },
  { id: 'performance', name: 'Performance', icon: Cpu, kinds: ['system-rings', 'cpu', 'ram', 'gpu', 'performance', 'network', 'network-usage', 'storage'] as const },
  { id: 'system', name: 'System', icon: Grid2X2, kinds: ['start', 'spacebar-start', 'tray', 'show-desktop', 'notifications', 'quick-settings', 'keyboard', 'display', 'notification-settings', 'capture'] as const },
  { id: 'apps', name: 'Apps & folders', icon: FolderOpen, kinds: ['folder', 'apps'] as const },
  { id: 'layout', name: 'Layout', icon: Columns2, kinds: ['widget-group', 'widget-tray', 'spacer', 'divider'] as const },
] as const;
export type WidgetSection = 'all' | typeof widgetSections[number]['id'];

export function WidgetsSidebar({ onBack, section, onSection, query, onQuery }: {
  onBack: () => void; section: WidgetSection; onSection: (section: WidgetSection) => void; query: string; onQuery: (query: string) => void;
}) {
  return <div className="widgets-sidebar-controls">
    <button className="widgets-back" onClick={onBack}><ArrowLeft size={16} />Settings</button>
    <label className="widget-search"><Search size={15} /><input aria-label="Search widgets" placeholder="Search widgets" value={query} onChange={(event) => onQuery(event.currentTarget.value)} /></label>
    <nav aria-label="Widget sections">
      <button className={`widget-section-link${section === 'all' ? ' selected' : ''}`} onClick={() => onSection('all')} aria-current={section === 'all' ? 'location' : undefined}><LayoutGrid size={16} />All widgets</button>
      {widgetSections.map(({ id, name, icon: Icon }) => <button key={id} className={`widget-section-link${section === id ? ' selected' : ''}`}
        onClick={() => onSection(id)} aria-current={section === id ? 'location' : undefined}><Icon size={16} />{name}</button>)}
    </nav>
  </div>;
}

function WidgetCard({ kind, items, ready, apps, onError }: {
  kind: WidgetKind; items: WidgetItem[]; ready: boolean; apps: DockApp[];
  onError: (error: unknown) => void;
}) {
  const installed = allWidgets(items).find((item) => item.kind === kind);
  const unique = !repeatableWidget(kind);
  const specimen = unique ? installed : undefined;
  const drag = useWidgetDrag(unique && installed ? { type: 'item', id: installed.id } : { type: 'new', kind }, ready);
  const label = widgetNames[kind];
  const { detail, width } = widgetCatalog[kind];
  return <button type="button" className={`widget-card gallery-${kind}`} aria-label={`${label}${unique && installed ? ', on dock' : ', add to dock'}`}
    title={detail} style={{ '--gallery-width': (width ? width + 12 : 86) + 'px' } as CSSProperties}
    disabled={!ready} {...drag} onClick={(event) => {
      if (kind === 'image') { void openImageEditor().catch(onError); return; }
      if ((event.detail === 0 || !desktop) && !(unique && installed)) {
        void editWidgets((layout) => placeWidget(layout, { type: 'new', kind })).catch(onError);
      }
    }}>
    <span className="widget-card-preview" data-drag-image data-square={width === 0 && kind !== 'apps'} aria-hidden="true"
      style={widgetColorStyle(specimen)} {...(specimen && widgetAppearance(specimen))}><WidgetSpecimen kind={kind} apps={apps} item={specimen} live /></span>
    {unique && installed && <span className="widget-card-added" aria-hidden="true"><Check size={10} strokeWidth={3} /></span>}
    <span className="widget-card-caption">{label}</span>
  </button>;
}

export function WidgetLibrary({ items, ready, query, selected }: {
  items: WidgetItem[]; ready: boolean; query: string; selected: WidgetSection;
}) {
  const [error, setError] = useState('');
  const { apps } = useDockApps();
  const visibleApps = desktop && apps.length ? rowApps(apps, items) : demoApps;
  const term = query.trim().toLowerCase();
  const sections = widgetSections.filter(({ id }) => selected === 'all' || selected === id)
    .map((section) => ({ ...section, kinds: section.kinds.filter((kind) =>
      `${section.name} ${widgetNames[kind]} ${widgetCatalog[kind].detail}`.toLowerCase().includes(term)) })).filter((section) => section.kinds.length);
  const report = (error: unknown) => setError(String(error));
  return <main className="widget-library">
    <header className="widget-library-heading">
      <h1>{selected === 'all' ? 'All Widgets' : widgetSections.find(({ id }) => id === selected)?.name}</h1>
      <span aria-live="polite">{items.length} on dock</span>
    </header>
    {error && <p className="widget-message" role="alert">{error}</p>}
    <div className="widget-sections" data-filtered={selected !== 'all'}>
      {sections.map(({ id, name, icon: Icon, kinds }) => <section key={id} id={`widgets-${id}`} className={`widget-section section-${id}`} aria-labelledby={`widgets-${id}-heading`}>
        <h2 id={`widgets-${id}-heading`}><Icon size={16} />{name}</h2>
        <div className="widget-grid">{kinds.map((kind) => <WidgetCard key={kind} kind={kind} items={items} ready={ready} apps={visibleApps} onError={report} />)}</div>
      </section>)}
      {!sections.length && <p className="widget-message">No widgets match “{query.trim()}”.</p>}
    </div>
  </main>;
}
