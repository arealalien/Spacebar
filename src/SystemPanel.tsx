import { invoke } from '@tauri-apps/api/core';
import { Bell, Settings2, X } from 'lucide-react';
import { useCallback, useEffect, useLayoutEffect, useRef, useState, type CSSProperties } from 'react';
import { Calendar } from './Calendar';
import { useDockAppearance } from './dockAppearance';
import { contrastingInk, defaults, type Settings } from './model';
import { AgendaPanel, QuickNote } from './PersonalPanels';
import { DrawingPad } from './DrawingPad';
import { QuickTasks, ScreenColors } from './UtilityPanels';
import { PersonalGlyph, isPersonalWidget } from './PersonalWidgets';
import { StartLauncher } from './StartLauncher';
import { SurfaceSnow } from './SurfaceSnow';
import { useSurfacePayload } from './useSurfacePayload';
import { QuickSettings } from './QuickSettings';
import { SystemTray } from './SystemTray';

type PanelKind = 'tray' | 'quick-settings' | 'audio-output' | 'microphone' | 'notifications' | 'quick-note' | 'drawing-pad' | 'agenda' | 'quick-tasks' | 'color-picker' | 'spacebar-start';

const names: Record<PanelKind, string> = {
  tray: 'System tray', 'quick-settings': 'Quick settings', notifications: 'Notifications', 'spacebar-start': 'Spacebar Start',
  'quick-note': 'Quick Note', 'drawing-pad': 'Drawing Pad', agenda: 'Agenda',
  'quick-tasks': 'Quick Tasks', 'color-picker': 'Color Picker',
  'audio-output': 'Audio output', microphone: 'Microphone',
};

export function SystemPanel() {
  const panel = useSurfacePayload<PanelKind>('system-panel-changed', 'read_system_panel');
  const settings = useSurfacePayload<Settings>('settings-changed', 'read_settings') ?? defaults;
  const { tint, material, ...appearance } = useDockAppearance(settings, !!panel);
  const startBackdrop = useSurfacePayload<string>('start-backdrop-changed', 'read_start_backdrop');
  const content = useRef<HTMLElement>(null);
  const activePanel = useRef(panel);
  const [error, setError] = useState('');
  const report = useCallback((error: unknown) => {
    if (activePanel.current === panel) setError(String(error));
  }, [panel]);
  const close = () => void invoke('close_system_panel').catch(report);
  const action = (action: string) => void invoke('system_action', { action }).catch(report);
  useEffect(() => { setError(''); if (panel !== 'spacebar-start') content.current?.focus(); }, [panel]);
  useLayoutEffect(() => {
    activePanel.current = panel;
    const element = content.current;
    if (panel !== 'spacebar-start' || !element || matchMedia('(prefers-reduced-motion: reduce)').matches) return;
    const started = performance.now();
    let timer = 0;
    // Elapsed time lets the transparent WebView finish the entrance even if compositor frames stall.
    const draw = () => {
      const progress = Math.min(1, (performance.now() - started) / 185);
      if (progress === 1) {
        element.style.removeProperty('--start-enter');
      } else {
        element.style.setProperty('--start-enter', String(1 - (1 - progress) ** 3));
        timer = window.setTimeout(draw, 16);
      }
    };
    draw();
    return () => { window.clearTimeout(timer); element.style.removeProperty('--start-enter'); };
  }, [panel]);

  if (!panel) return null;
  const personal = isPersonalWidget(panel);
  const quick = panel === 'quick-settings' || panel === 'audio-output' || panel === 'microphone';
  const utility = panel === 'quick-tasks' || panel === 'color-picker';
  return <main className={`system-panel${utility ? ' utility-panel' : ''}`} data-panel={panel} data-quick={quick || undefined} data-personal={personal || undefined} data-material={material} {...appearance} data-edge={settings.edge}
    role={utility ? 'dialog' : undefined} aria-labelledby="system-panel-title" tabIndex={-1} ref={content}
    style={{ '--dock-tint': tint, '--calendar-day-ink': material === 'tinted' ? contrastingInk(tint) : undefined } as CSSProperties}
    onKeyDown={(event) => { if (event.key === 'Escape') close(); }}>
    {settings.dockSnow && <SurfaceSnow key={panel} />}
    {(quick || panel === 'spacebar-start' || panel === 'tray') && startBackdrop && <img className="start-backdrop" src={startBackdrop} alt="" draggable={false} />}
    {panel === 'spacebar-start' ? <>
      <StartLauncher onClose={close} onError={report} />
    </> : quick ? <QuickSettings key={panel} initial={panel === 'microphone' ? 'input' : panel === 'audio-output' ? 'output' : 'overview'}
      onClose={close} onError={report} /> : panel === 'tray' ? <SystemTray onClose={close} onError={report} /> : <>
      <header className="system-panel-header">
        {personal && <span className="personal-panel-icon"><PersonalGlyph kind={panel} /></span>}
        <h1 id="system-panel-title">{names[panel]}</h1>
        {personal && <button type="button" className="personal-panel-close" aria-label={`Close ${names[panel]}`} onClick={close}><X size={16} /></button>}
      </header>
      <div className="system-panel-body">
        {panel === 'notifications' && <>
          <div className="system-notification-links"><button type="button" onClick={() => action('notifications')}><Bell size={15} />Windows notifications</button>
            <button type="button" title="Notification settings" aria-label="Notification settings" onClick={() => action('notification-settings')}><Settings2 size={15} /></button></div>
          <Calendar />
        </>}
        {panel === 'quick-note' && <QuickNote onError={report} />}
        {panel === 'drawing-pad' && <DrawingPad onError={report} />}
        {panel === 'agenda' && <AgendaPanel onError={report} />}
        {panel === 'quick-tasks' && <QuickTasks onError={report} />}
        {panel === 'color-picker' && <ScreenColors onError={report} />}
      </div>
    </>}
    {error && <p className="system-panel-error" role="alert">{error}</p>}
  </main>;
}
