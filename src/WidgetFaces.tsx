import { invoke } from '@tauri-apps/api/core';
import { AudioLines, Clock3, CloudSun, Folder, Music2 } from 'lucide-react';
import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { DockAppIcon } from './AppIcon';
import { demoApps, type DockApp } from './model';
import { dockPopupAnchor, type WidgetItem, type WidgetKind } from './widgets';
import { isClockKind, isSpotifyKind, isStartWidget, isWeatherKind, widgetCatalog, type ClockKind } from './widgetCatalog';
import { SystemWidget, SystemWidgetGlyph } from './SystemWidgets';
import { SpotifyFace } from './SpotifyWidgets';
import { isPersonalWidget, PersonalGlyph, PersonalWidget } from './PersonalWidgets';
import { AudioVisualizer } from './AudioVisualizer';
import { WeatherWidget } from './WeatherWidgets';

const timeFormat = new Intl.DateTimeFormat(undefined, { hour: '2-digit', minute: '2-digit' });
const dateFormat = new Intl.DateTimeFormat(undefined, { weekday: 'short', month: 'short', day: 'numeric' });

function FlipTile({ value, previous }: { value: string; previous: string }) {
  const tile = useRef<HTMLSpanElement>(null);
  const changed = value !== previous;

  useLayoutEffect(() => {
    const element = tile.current;
    if (!element || value === previous || document.hidden || matchMedia('(prefers-reduced-motion: reduce)').matches) return;

    const start = performance.now();
    let timer = 0;
    const clearFlip = () => {
      delete element.dataset.flipping;
      element.style.removeProperty('--flip-angle');
    };
    // Use elapsed time to keep the hinge moving in an unfocused dock WebView.
    const draw = () => {
      const progress = Math.min(1, (performance.now() - start) / 260);
      if (progress === 1) {
        clearFlip();
        return;
      }
      const swing = progress * progress * (3 - 2 * progress);
      element.dataset.flipping = swing < .5 ? 'top' : 'bottom';
      element.style.setProperty('--flip-angle', `${-180 * swing}deg`);
      timer = window.setTimeout(draw, 16);
    };
    draw();
    return () => { window.clearTimeout(timer); clearFlip(); };
  }, [value, previous]);

  const half = (side: 'top' | 'bottom', digits: string) =>
    <span className="flip-half" data-half={side}><span className="flip-glyph">{digits}</span></span>;

  return <span className="flip-digit" ref={tile} aria-hidden="true">
    <span className="flip-glyph">{value}</span>
    {changed && <>
      {half('top', value)}
      {half('bottom', previous)}
      <span className="flip-flap">{half('top', previous)}{half('bottom', value)}</span>
    </>}
  </span>;
}

function ClockFace({ kind }: { kind: ClockKind }) {
  const [{ now, previous }, setTimes] = useState(() => {
    const now = new Date();
    return { now, previous: now };
  });
  useEffect(() => {
    const timer = window.setInterval(() => {
      const next = new Date();
      setTimes(({ now }) => ({ now: next, previous: now }));
    }, 1000);
    return () => window.clearInterval(timer);
  }, []);
  const time = timeFormat.format(now);
  const dateTime = now.toISOString();
  if (kind === 'day-progress') {
    const hour = now.getHours();
    const progress = Math.floor(100 * (hour * 60 + now.getMinutes()) / 1440);
    return <div className="widget-face day-face" role="meter" aria-label="Day progress" aria-valuemin={0} aria-valuemax={100} aria-valuenow={progress}>
      <div className="day-dots" aria-hidden="true">{Array.from({ length: 24 }, (_, index) =>
        <i key={index} data-hour={index < hour ? 'elapsed' : index === hour ? 'current' : 'ahead'} />)}</div>
      <div className="day-caption" aria-hidden="true"><span>Day</span><strong>{progress}%</strong></div>
    </div>;
  }
  if (kind === 'clock-flip') {
    const parts = timeFormat.formatToParts(now);
    const lastParts = timeFormat.formatToParts(previous);
    const part = (type: Intl.DateTimeFormatPartTypes, reading = parts) => reading.find((part) => part.type === type)?.value ?? '';
    const dayPeriod = part('dayPeriod');
    return <div className="widget-face flip-face"><time dateTime={dateTime} aria-label={time}>
      <FlipTile value={part('hour')} previous={part('hour', lastParts)} />
      <FlipTile value={part('minute')} previous={part('minute', lastParts)} />
      {dayPeriod && <span className="flip-period" aria-hidden="true">{dayPeriod}</span>}
    </time></div>;
  }
  if (kind === 'clock-analog' || kind === 'clock-twilight') {
    const hour = (now.getHours() % 12 + now.getMinutes() / 60) * 30;
    const minute = (now.getMinutes() + now.getSeconds() / 60) * 6;
    return <div className={`widget-face analog-face ${kind}`} title={time}>
      <time dateTime={dateTime} aria-label={time}>
        <span className="analog-dial" aria-hidden="true">
          {Array.from({ length: 12 }, (_, index) => <i key={index} className="clock-tick" data-major={index % 3 === 0} style={{ rotate: index * 30 + 'deg' }} />)}
          <span className="analog-hand hour-hand" style={{ rotate: hour + 'deg' }} />
          <span className="analog-hand minute-hand" style={{ rotate: minute + 'deg' }} />
          <span className="analog-hand second-hand" style={{ rotate: now.getSeconds() * 6 + 'deg' }} />
          <span className="analog-hub" />
        </span>
      </time>
    </div>;
  }
  if (kind === 'clock-plain') return <div className="widget-face digital-face"><time dateTime={dateTime}>{time}</time></div>;
  return <div className="widget-face clock-face">
    <div><time dateTime={dateTime}>{time}</time><span>{dateFormat.format(now)}</span></div>
  </div>;
}

export function WindowsMark() { return <span className="windows-mark" aria-hidden="true"><i /><i /><i /><i /></span>; }

export function WidgetSpecimen({ kind, apps = demoApps, item, live = false }: { kind: WidgetKind; apps?: DockApp[]; item?: WidgetItem; live?: boolean }) {
  if (isClockKind(kind)) return <ClockFace kind={kind} />;
  if (isSpotifyKind(kind)) return <SpotifyFace kind={kind} specimen live={live} />;
  if (isWeatherKind(kind)) return <WeatherWidget kind={kind} specimen />;
  if (isPersonalWidget(kind)) return <PersonalWidget kind={kind} specimen />;
  switch (kind) {
    case 'audio-visualizer': return <AudioVisualizer specimen options={item} />;
    case 'start': case 'spacebar-start': return <span className="widget-face start-face"><WindowsMark /></span>;
    case 'spacer': return <span className="layout-face spacer-face" aria-hidden="true" />;
    case 'divider': return <span className="layout-face divider-face" aria-hidden="true" />;
    case 'folder': return <span className="widget-face folder-face"><FolderContents item={item} apps={apps} /></span>;
    case 'apps': return <div className="widget-face apps-face">{apps.map((app) => <span key={app.id}><DockAppIcon app={app} /></span>)}</div>;
    default: return <SystemWidget kind={kind} specimen live={live} />;
  }
}

function FolderContents({ item, apps }: { item?: WidgetItem; apps: DockApp[] }) {
  const children = item?.children?.slice(0, 4) ?? [];
  if (!children.length) return <Folder size="53%" />;
  return <span className="folder-mini-grid">{children.map((child) => <span key={child.id}>
    {child.kind === 'app' ? <DockAppIcon app={apps.find((app) => app.id === child.appId) ?? { id: child.appId ?? '', icon: child.icon }} />
      : isStartWidget(child.kind) ? <WindowsMark /> : isClockKind(child.kind) ? <Clock3 size="75%" /> : isSpotifyKind(child.kind) ? <Music2 size="75%" />
      : isPersonalWidget(child.kind) ? <PersonalGlyph kind={child.kind} />
      : child.kind === 'audio-visualizer' ? <AudioLines size="75%" /> : isWeatherKind(child.kind) ? <CloudSun size="75%" /> : <SystemWidgetGlyph kind={child.kind} />}
  </span>)}</span>;
}

export function WidgetFace({ item, apps, specimen = false, active = true, onFolder, onError }: {
  item: WidgetItem;
  apps: DockApp[];
  specimen?: boolean;
  active?: boolean;
  onFolder?: (button: HTMLButtonElement) => void;
  onError: (error: unknown) => void;
}) {
  if (isClockKind(item.kind)) return <ClockFace kind={item.kind} />;
  if (isSpotifyKind(item.kind)) return <SpotifyFace kind={item.kind} widget={item.id} specimen={specimen} onError={onError} />;
  if (isWeatherKind(item.kind)) return <WeatherWidget kind={item.kind} widget={item.id} specimen={specimen} onError={onError} />;
  if (isPersonalWidget(item.kind)) return <PersonalWidget kind={item.kind} specimen={specimen} onError={onError} />;
  switch (item.kind) {
    case 'audio-visualizer': return <AudioVisualizer specimen={specimen} active={active} options={item} />;
    case 'start': return <button type="button" className="widget-face start-face" aria-label="Open Windows Start menu" title="Start menu"
      onClick={(event) => { if (!specimen) void invoke('toggle_start_menu', { anchor: dockPopupAnchor(event.currentTarget) }).catch(onError); }}><WindowsMark /></button>;
    case 'spacebar-start': return <button type="button" className="widget-face start-face" aria-label="Toggle Spacebar Start" title="Spacebar Start" aria-haspopup="dialog"
      onClick={(event) => { if (!specimen) void invoke('open_system_panel', { panel: 'spacebar-start', anchor: dockPopupAnchor(event.currentTarget) }).catch(onError); }}><WindowsMark /></button>;
    case 'folder': {
      return <button type="button" className="widget-face folder-face" aria-haspopup="dialog" title={item.name ?? 'Folder'} aria-label={`Toggle ${item.name ?? 'Folder'}`} onClick={(event) => onFolder?.(event.currentTarget)}>
        <FolderContents item={item} apps={apps} />
      </button>;
    }
    case 'app': case 'apps': return null;
    case 'spacer': return <span className="layout-face spacer-face" aria-hidden="true" />;
    case 'divider': return <span className="layout-face divider-face" aria-hidden="true" />;
    default: return <SystemWidget kind={item.kind} specimen={specimen} live={specimen} onError={onError} />;
  }
}

export function preferredWidgetWidth(kind: WidgetItem['kind'], size: number) {
  const width = kind === 'app' ? 0 : widgetCatalog[kind].width;
  if (kind === 'clock') return Math.max(96, size * 2.1);
  if (kind === 'spotify') return Math.max(158, size * 3.75);
  return width ? Math.max(width * 0.72, width * size / 46) : size;
}
