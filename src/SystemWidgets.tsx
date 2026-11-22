import { invoke } from '@tauri-apps/api/core';
import { Activity, ArrowDown, ArrowUp, AudioLines, Bell, Camera, ChevronRight, ChevronUp, Cpu, Crop, HardDrive, Keyboard, MemoryStick, Mic, MicOff, Monitor, Moon, Scan, SlidersHorizontal, Sun, Volume2, VolumeX, Wifi, type LucideIcon } from 'lucide-react';
import { useEffect, useRef, useState, type CSSProperties } from 'react';
import { widgetSnapshot } from './widgetSnapshots';
import { dockPopupAnchor } from './widgets';
import type { ClockKind, SpotifyKind, WidgetKind } from './widgetCatalog';
import { smoothLine } from './widgetGraph';

type SystemKind = Exclude<WidgetKind, 'apps' | 'start' | 'spacebar-start' | 'folder' | ClockKind | SpotifyKind>;
type Device = 'output' | 'input';
type AudioLevel = { level: number; muted: boolean };
type AudioSnapshot = Partial<Record<Device, AudioLevel | null>>;
type Capacity = { total: number; available: number };
type Telemetry = {
  cpu: number | null; gpu: number | null; ram: Capacity | null; disk: Capacity | null;
  network: { received: number; sent: number; rates: [number, number] | null; history: [number, number][] } | null;
};
type FaceProps = { specimen?: boolean; live?: boolean; onError?: (error: unknown) => void };

const audio = widgetSnapshot<AudioSnapshot>('audio_snapshot', 1100, 'audio-endpoint-changed');
const telemetry = widgetSnapshot<Telemetry>('system_snapshot');
let audioQueue = Promise.resolve();

const monitorKinds = ['cpu', 'ram', 'gpu', 'performance', 'system-rings', 'network', 'network-usage', 'storage'] as const;
type MonitorKind = typeof monitorKinds[number];

const previewHistory: [number, number][] = [
  [180, 75], [380, 140], [740, 110], [520, 210], [1050, 310],
  [880, 180], [410, 90], [730, 245], [1480, 410], [1120, 235],
].map(([download, upload]) => [download * 1024, upload * 1024]);
const gib = 1024 ** 3;
const previewTelemetry: Telemetry = {
  cpu: 28, gpu: 16,
  ram: { total: 32 * gib, available: 19 * gib },
  disk: { total: 512 * gib, available: 184 * gib },
  network: { received: 356 * 1024 ** 2, sent: 42 * 1024 ** 2, rates: previewHistory.at(-1) ?? null, history: previewHistory },
};

const percent = (value: number | null | undefined) => value == null ? '—' : Math.round(value).toString();
const used = (value: Capacity | null | undefined) => value?.total ? 100 * (1 - value.available / value.total) : undefined;

function bytes(value: number | undefined) {
  if (value == null) return '—';
  if (value < 1024) return Math.round(value) + ' B';
  const divisor = Math.min(4, Math.floor(Math.log(value) / Math.log(1024)));
  return (value / 1024 ** divisor).toLocaleString(undefined, { maximumFractionDigits: 1 }) + ' ' + ['B', 'KB', 'MB', 'GB', 'TB'][divisor];
}

const meterIcons = { cpu: Cpu, ram: MemoryStick, gpu: Monitor };
function Gauge({ kind, value, paired = false }: { kind: keyof typeof meterIcons; value: number | null | undefined; paired?: boolean }) {
  const Icon = meterIcons[kind];
  const label = kind.toUpperCase();
  const description = label + ' usage: ' + (value == null ? 'Unavailable' : percent(value) + '%');
  return <div className={`${paired ? 'gauge-face' : 'widget-face gauge-face'} gauge-${kind}`} role="img" aria-label={description} title={description}>
    <svg viewBox="0 0 44 44" aria-hidden="true">
      <circle className="gauge-track" cx="22" cy="22" r="19" />
      <circle className="gauge-progress" cx="22" cy="22" r="19" pathLength="100" strokeDasharray={String(value ?? 0) + ' 100'}
        opacity={value != null && value > 0 ? 1 : 0} />
    </svg>
    <span className="gauge-content">
      {!paired && <Icon aria-hidden="true" />}<strong>{percent(value)}{value == null ? '' : '%'}</strong>{paired && <small>{label}</small>}
    </span>
  </div>;
}

function Bar({ value }: { value: number | null | undefined }) {
  return <span className="meter-bar" aria-hidden="true"><i style={{ width: (value ?? 0) + '%' }} /></span>;
}

function NetworkFace({ sample, totals }: { sample?: Telemetry['network']; totals: boolean }) {
  const rates = sample?.rates;
  const values = totals ? [sample?.received, sample?.sent] : [rates?.[0], rates?.[1]];
  const history = sample?.history ?? [];
  const max = Math.max(1, ...history.flat());
  const curve = (lane: 0 | 1) => smoothLine(history.map((pair) => 22 - 19 * pair[lane] / max));
  const format = (value: number | undefined) => bytes(value) + (value != null && !totals ? '/s' : '');
  const download = format(values[0]);
  const upload = format(values[1]);
  const title = (totals ? 'Data this session' : 'Current network transfer rate') + ' · Download ' + download + ' · Upload ' + upload;
  return <div className="widget-face network-face" title={title}>
    <div className="network-values">
      <span aria-label={'Download ' + download}><ArrowDown size={12} aria-hidden="true" /><strong>{download}</strong></span>
      <span aria-label={'Upload ' + upload}><ArrowUp size={12} aria-hidden="true" /><strong>{upload}</strong></span>
    </div>
    {totals ? <small>Data this session</small> : history.length > 1
      ? <svg className="network-graph" viewBox="0 0 100 24" preserveAspectRatio="none" aria-hidden="true">
        <path className="network-upload" d={curve(1)} /><path className="network-download" d={curve(0)} />
      </svg>
      : <small>{sample === null ? 'Network unavailable' : 'Collecting samples'}</small>}
  </div>;
}

function MonitorFace({ kind, specimen, live }: FaceProps & { kind: MonitorKind }) {
  const snapshot = telemetry.useValue(!specimen || !!live);
  const reading = snapshot ?? (specimen ? previewTelemetry : undefined);
  if (kind === 'cpu' || kind === 'ram' || kind === 'gpu') return <Gauge kind={kind} value={kind === 'ram' ? used(reading?.ram) : reading?.[kind]} />;
  if (kind === 'system-rings') return <div className="widget-face rings-face" aria-label="CPU and memory usage">
    <Gauge kind="cpu" value={reading?.cpu} paired /><Gauge kind="ram" value={used(reading?.ram)} paired />
  </div>;
  if (kind === 'network' || kind === 'network-usage') return <NetworkFace sample={reading?.network} totals={kind === 'network-usage'} />;
  if (kind === 'storage') return <div className="widget-face storage-face" title={bytes(reading?.disk?.available) + ' free on the Windows system drive'}>
    <div><HardDrive size={14} /><strong>{bytes(reading?.disk?.available)}</strong><small>free</small></div>
    <Bar value={used(reading?.disk)} />
  </div>;
  return <div className="widget-face performance-face" aria-label="System load">
    {(['CPU', 'RAM', 'GPU'] as const).map((label, i) => {
      const value = [reading?.cpu, used(reading?.ram), reading?.gpu][i];
      return <div key={label}><span>{label}</span><Bar value={value} /><strong>{percent(value)}{value == null ? '' : '%'}</strong></div>;
    })}
  </div>;
}

export function AudioFace({ device, specimen, live, onError, onDevices, expanded }: FaceProps & { device: Device; onDevices?: () => void; expanded?: boolean }) {
  const snapshot = audio.useValue(!specimen || !!live);
  const data = snapshot ? snapshot[device] : specimen ? { level: device === 'input' ? .72 : .58, muted: false } : undefined;
  const [draft, setDraft] = useState<number>();
  const request = useRef(0);
  const timer = useRef<number>(undefined);
  const label = device === 'input' ? 'Microphone' : 'Audio output';
  const Icon = device === 'input' ? data?.muted ? MicOff : Mic : data?.muted ? VolumeX : Volume2;
  const total = draft ?? (data ? Math.round(data.level * 100) : 0);
  const shown = total + '%';
  useEffect(() => () => window.clearTimeout(timer.current), []);

  function send(level: number | null, toggleMute: boolean, revision: number) {
    audioQueue = audioQueue.then(async () => {
      try {
        const result = await invoke<AudioLevel>('audio_control', { device, level: level == null ? null : level / 100, toggleMute });
        audio.update((old) => ({ ...old, [device]: result }));
      } catch (error) { onError?.(error); }
      finally { if (request.current === revision) setDraft(undefined); }
    });
  }

  return <div className="widget-face audio-face" data-muted={data?.muted}>
    <div className="audio-head">
      {specimen ? <span className="audio-icon"><Icon size={14} /></span> : <button type="button" className="audio-icon" aria-label={(data?.muted ? 'Unmute ' : 'Mute ') + label.toLowerCase()}
        title={(data?.muted ? 'Unmute ' : 'Mute ') + label.toLowerCase()} disabled={!data} onClick={() => send(null, true, request.current)}><Icon size={14} /></button>}
      {!specimen && !expanded ? <button type="button" className="audio-device-open" data-no-drag
        aria-label={`Choose ${device === 'input' ? 'input' : 'output'} device`} aria-haspopup="dialog"
        onClick={(event) => {
          if (onDevices) onDevices();
          else void invoke('open_system_panel', { panel: device === 'input' ? 'microphone' : 'audio-output', anchor: dockPopupAnchor(event.currentTarget.closest<HTMLElement>('.audio-face') ?? event.currentTarget) }).catch(onError);
        }}><span>{label}</span><ChevronRight size={13} /></button> : <span>{label}</span>}
      <strong>{data?.muted ? 'Muted' : data ? shown : '—'}</strong>
    </div>
    {specimen ? <Bar value={data ? total : undefined} /> : <input type="range" aria-label={label + ' level'} data-no-drag
      min={0} max={100} disabled={!data} value={total} aria-valuetext={shown}
      style={{ '--audio-level': shown } as CSSProperties} onChange={(event) => {
        const level = event.currentTarget.valueAsNumber;
        const revision = ++request.current;
        setDraft(level);
        window.clearTimeout(timer.current);
        timer.current = window.setTimeout(() => send(level, false, revision), 70);
      }} />}
  </div>;
}

const tools: Partial<Record<SystemKind, [string, string, LucideIcon]>> = {
  tray: ['tray', 'Toggle Spacebar system tray', ChevronUp],
  'show-desktop': ['desktop', 'Show or restore the desktop', Monitor],
  notifications: ['notifications', 'Toggle calendar and notifications', Bell],
  'quick-settings': ['quick-settings', 'Toggle Spacebar quick settings', SlidersHorizontal],
  keyboard: ['keyboard', 'Toggle the Windows on-screen keyboard', Keyboard],
  display: ['display', 'Open Windows display settings', Sun],
  'notification-settings': ['notification-settings', 'Open notification and Do Not Disturb settings', Moon],
  mixer: ['audio-output', 'Choose audio output', AudioLines],
};
const captures: [string, string, LucideIcon][] = [
  ['region', 'Snip a screen region', Crop],
  ['fullscreen', 'Save a full-screen screenshot', Scan],
  ['snipping-tool', 'Open Snipping Tool', Camera],
];

const systemGlyphs: Partial<Record<WidgetKind, LucideIcon>> = {
  ...meterIcons, 'system-rings': Activity, performance: Activity, network: Wifi, 'network-usage': Wifi,
  storage: HardDrive, volume: Volume2, microphone: Mic, capture: Camera,
};

export function SystemWidgetGlyph({ kind }: { kind: WidgetKind }) {
  const Icon = systemGlyphs[kind] ?? tools[kind as SystemKind]?.[2];
  return Icon ? <Icon aria-hidden="true" /> : <i />;
}

function Tool({ tool: [action, label, Icon], specimen, onError }: FaceProps & { tool: [string, string, LucideIcon] }) {
  const opensPanel = action === 'tray' || action === 'notifications' || action === 'quick-settings' || action === 'audio-output';
  return specimen ? <span className="system-tool" data-action={action}><Icon /></span>
    : <button type="button" className="system-tool" data-action={action} aria-label={label} title={label} aria-haspopup={opensPanel ? 'dialog' : undefined}
      onClick={(event) => {
        void (opensPanel ? invoke('open_system_panel', { panel: action, anchor: dockPopupAnchor(event.currentTarget) })
          : invoke('system_action', { action })).catch(onError);
      }}><Icon /></button>;
}

export function SystemWidget({ kind, ...props }: FaceProps & { kind: SystemKind }) {
  if (kind === 'volume' || kind === 'microphone') return <AudioFace device={kind === 'volume' ? 'output' : 'input'} {...props} />;
  if ((monitorKinds as readonly string[]).includes(kind)) return <MonitorFace kind={kind as MonitorKind} {...props} />;
  if (kind === 'capture') return <div className="widget-face capture-face" aria-label="Screenshot shortcuts">
    {captures.map(tool => <Tool key={tool[0]} tool={tool} {...props} />)}
  </div>;
  const tool = tools[kind];
  return tool && <div className="widget-face single-tool-face"><Tool tool={tool} {...props} /></div>;
}
