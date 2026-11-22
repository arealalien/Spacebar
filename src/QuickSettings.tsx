import { invoke } from '@tauri-apps/api/core';
import { ArrowLeft, Bluetooth, Check, ChevronRight, ExternalLink, LockKeyhole, Mic, RefreshCw, Sun, Volume2, Wifi, X } from 'lucide-react';
import { useCallback, useEffect, useRef, useState, type ReactNode } from 'react';
import { AudioFace } from './SystemWidgets';

type AudioDevice = 'input' | 'output';
type Page = 'overview' | AudioDevice | 'wifi' | 'bluetooth';
type Endpoint = { id: string; name: string; selected: boolean };
type Network = { id: string; name: string; signal: number; connected: boolean; secured: boolean; saved: boolean; needsPassword: boolean; canConnect: boolean };
type Peripheral = { id: string; name: string; paired: boolean; connected: boolean; canPair: boolean; lowEnergy: boolean };

const pageNames: Record<Page, string> = {
  overview: 'Quick Settings', input: 'Microphone', output: 'Audio output', wifi: 'Wi-Fi', bluetooth: 'Bluetooth',
};

function useDevices<T>(command: string, device?: AudioDevice) {
  const [rows, setRows] = useState<T[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const refresh = useCallback(async (scan = false) => {
    setLoading(true);
    setError('');
    try {
      setRows(await invoke<T[]>(command, device ? { device } : command === 'wifi_networks' ? { scan } : {}));
    } catch (error) { setError(String(error)); }
    finally { setLoading(false); }
  }, [command, device]);
  useEffect(() => { void refresh(command === 'wifi_networks'); }, [command, refresh]);
  return { rows, setRows, loading, error, refresh };
}

function DeviceStatus({ loading, error, empty }: { loading?: boolean; error?: string; empty?: string }) {
  if (error) return <p className="quick-status" role="alert">{error}</p>;
  if (loading || empty) return <p className="quick-status" role="status">{loading ? 'Looking for devices…' : empty}</p>;
  return null;
}

function DetailFooter({ refresh, disabled, children }: { refresh: () => void; disabled: boolean; children?: ReactNode }) {
  return <footer className="quick-detail-footer">
    <button type="button" onClick={refresh} disabled={disabled}><RefreshCw size={14} />Refresh</button>
    {children}
  </footer>;
}

function WindowsLink({ action, children, onError }: { action: string; children: ReactNode; onError: (error: unknown) => void }) {
  return <button type="button" onClick={() => void invoke('system_action', { action }).catch(onError)}>{children}<ExternalLink size={13} /></button>;
}

function AudioDevices({ device }: { device: AudioDevice }) {
  const { rows, setRows, refresh, loading, error } = useDevices<Endpoint>('audio_devices', device);
  const [pending, setPending] = useState('');
  const [failure, setFailure] = useState('');
  const Icon = device === 'input' ? Mic : Volume2;

  async function select(id: string) {
    setFailure('');
    setPending(id);
    try { setRows(await invoke<Endpoint[]>('select_audio_device', { device, id })); }
    catch (error) { setFailure(String(error)); }
    finally { setPending(''); }
  }

  return <>
    <div className="quick-scroll" aria-busy={loading || !!pending}>
      <AudioFace device={device} expanded onError={(error) => setFailure(String(error))} />
      <DeviceStatus error={failure || error} loading={loading && !rows.length} empty={!rows.length && !loading ? 'No audio devices available.' : undefined} />
      <div className="quick-list">
        {rows.map(({ id, name, selected }) => <button type="button" className="quick-row quick-device"
          data-selected={selected} key={id} aria-pressed={selected} disabled={!!pending || loading}
          onClick={() => void select(id)}>
          <Icon size={17} /><span><strong>{name}</strong>{selected && <small>Default device</small>}</span>
          {selected && <Check size={16} />}
        </button>)}
      </div>
    </div>
    <DetailFooter refresh={() => { setFailure(''); void refresh(); }} disabled={loading || !!pending} />
  </>;
}

function WifiDevices({ onError }: { onError: (error: unknown) => void }) {
  const { rows, loading, error, refresh } = useDevices<Network>('wifi_networks');
  const [selected, setSelected] = useState<Network>();
  const [password, setPassword] = useState('');
  const [pending, setPending] = useState('');
  const [failure, setFailure] = useState('');

  async function connect(network: Network, key?: string) {
    setPending(network.id);
    setFailure('');
    setPassword('');
    try {
      await invoke('wifi_connect', { id: network.id, password: key ?? null });
      setSelected(undefined);
      await refresh();
    } catch (error) { setFailure(String(error)); }
    finally { setPending(''); }
  }

  return <>
    <div className="quick-scroll" aria-busy={!!pending || loading}>
      <DeviceStatus error={failure || error} loading={loading && !rows.length} empty={!rows.length && !loading ? 'No Wi-Fi networks found.' : undefined} />
      <div className="quick-list">
        {rows.map((network) => <div key={network.id}>
          <div className="quick-row" data-selected={network.connected}>
            <Wifi size={17} />
            <span><strong>{network.name}</strong><small>{network.connected ? 'Connected' : network.saved ? 'Saved' : network.secured ? 'Secured' : 'Open'} · {network.signal}% signal</small></span>
            {network.connected ? <Check size={16} /> : network.canConnect ?
              <button type="button" disabled={!!pending || loading} onClick={() => {
                setFailure('');
                if (network.needsPassword) { setSelected(network); setPassword(''); }
                else { setSelected(undefined); void connect(network); }
              }}>{pending === network.id ? 'Connecting…' : 'Connect'}</button> : <LockKeyhole size={15} />}
          </div>
          {selected?.id === network.id && <form className="quick-password" onSubmit={(event) => {
            event.preventDefault();
            if (!pending) void connect(network, password);
          }}>
            <input type="password" autoComplete="off" autoFocus maxLength={64} required placeholder="Network password" aria-label={`Password for ${network.name}`}
              value={password} onChange={(event) => setPassword(event.currentTarget.value)} disabled={!!pending} />
            <button type="submit" disabled={!!pending || !password}>{pending ? 'Connecting…' : 'Join'}</button>
          </form>}
        </div>)}
      </div>
    </div>
    <DetailFooter refresh={() => { setSelected(undefined); setPassword(''); setFailure(''); void refresh(true); }} disabled={!!pending || loading}>
      <WindowsLink action={error.toLowerCase().includes('location') ? 'location' : 'wifi'} onError={onError}>
        {error.toLowerCase().includes('location') ? 'Location settings' : 'Windows Wi-Fi settings'}
      </WindowsLink>
    </DetailFooter>
  </>;
}

function BluetoothDevices({ onError }: { onError: (error: unknown) => void }) {
  const { rows, loading, error, refresh } = useDevices<Peripheral>('bluetooth_devices');
  const [pending, setPending] = useState('');
  const [failure, setFailure] = useState('');
  const [notice, setNotice] = useState('');

  async function connect(device: Peripheral) {
    setPending(device.id);
    setFailure('');
    setNotice('');
    try {
      const result = await invoke<'Paired' | 'Connected' | 'Windows'>('bluetooth_connect', { id: device.id });
      setNotice(result === 'Windows' ? 'Finish connecting this device in Windows Bluetooth settings.' : `${result} to ${device.name}.`);
      await refresh();
    } catch (error) { setFailure(String(error)); }
    finally { setPending(''); }
  }

  return <>
    <div className="quick-scroll" aria-busy={loading || !!pending}>
      <DeviceStatus error={failure || error} loading={loading && !rows.length} empty={!rows.length && !loading ? 'No Bluetooth devices found. Put a device in pairing mode, then refresh.' : undefined} />
      {notice && <p className="quick-status" role="status">{notice}</p>}
      <div className="quick-list">
        {rows.map((device) => <div className="quick-row" key={device.id} data-selected={device.connected}>
          <Bluetooth size={17} />
          <span><strong>{device.name}</strong><small>{device.connected ? 'Connected' : device.paired ? 'Paired' : 'Available to pair'}</small></span>
          {device.connected ? <Check size={16} /> : device.paired && !device.lowEnergy ?
            <WindowsLink action="bluetooth" onError={onError}>Manage</WindowsLink> :
            <button type="button" disabled={loading || !!pending || (!device.paired && !device.canPair)}
              onClick={() => void connect(device)}>{pending === device.id ? 'Connecting…' : device.paired ? 'Connect' : 'Pair'}</button>}
        </div>)}
      </div>
    </div>
    <DetailFooter refresh={() => { setFailure(''); setNotice(''); void refresh(); }} disabled={loading || !!pending}>
      <WindowsLink action="bluetooth" onError={onError}>Windows Bluetooth settings</WindowsLink>
    </DetailFooter>
  </>;
}

export function QuickSettings({ initial, onClose, onError }: { initial: Page; onClose: () => void; onError: (error: unknown) => void }) {
  const [page, setPage] = useState<Page>(initial);
  const heading = useRef<HTMLHeadingElement>(null);
  useEffect(() => { heading.current?.focus({ preventScroll: true }); }, [page]);
  return <section className="quick-panel" onKeyDown={(event) => {
    if (event.key === 'Escape' && page !== 'overview' && initial === 'overview') {
      event.stopPropagation();
      setPage('overview');
    }
  }}>
    <header className="quick-header">
      {page !== 'overview' && <button type="button" className="quick-round" title="Quick Settings" aria-label="Back to Quick Settings" onClick={() => setPage('overview')}><ArrowLeft size={18} /></button>}
      <h1 id="system-panel-title" ref={heading} tabIndex={-1}>{pageNames[page]}</h1>
      <button type="button" className="quick-round" title="Close" aria-label="Close Quick Settings" onClick={onClose}><X size={17} /></button>
    </header>
    {page === 'overview' ? <>
      <div className="quick-scroll quick-overview">
        <div className="quick-connectivity">
          {([{ id: 'wifi', Icon: Wifi, detail: 'Browse networks' }, { id: 'bluetooth', Icon: Bluetooth, detail: 'Pair devices' }] as const).map(({ id, Icon, detail }) =>
            <button type="button" className="quick-tile" onClick={() => setPage(id)} key={id}>
              <span className="quick-tile-icon"><Icon size={19} /></span>
              <span><strong>{pageNames[id]}</strong><small>{detail}</small></span><ChevronRight className="quick-chevron" size={15} />
            </button>)}
        </div>
        <div className="quick-sound">
          <AudioFace device="output" onDevices={() => setPage('output')} onError={onError} />
          <AudioFace device="input" onDevices={() => setPage('input')} onError={onError} />
        </div>
      </div>
      <footer className="quick-detail-footer">
        <button type="button" onClick={() => void invoke('system_action', { action: 'display' }).catch(onError)}><Sun size={16} />Display settings<ExternalLink size={13} /></button>
      </footer>
    </> : page === 'wifi' ? <WifiDevices onError={onError} /> : page === 'bluetooth' ? <BluetoothDevices onError={onError} /> : <AudioDevices key={page} device={page} />}
  </section>;
}
