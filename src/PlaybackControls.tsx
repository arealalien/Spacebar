import { Pause, Play, SkipBack, SkipForward } from 'lucide-react';
import { useState, type CSSProperties } from 'react';

export type MediaAction = 'previous' | 'toggle' | 'next';

export interface PlaybackState {
  playing: boolean;
  canPrevious: boolean;
  canToggle: boolean;
  canNext: boolean;
}

export function PlaybackControls({ playback, label, className, style, onAction, onError, specimen, compact = false }: {
  playback?: PlaybackState;
  label: string;
  className: string;
  style?: CSSProperties;
  onAction: (action: MediaAction) => Promise<unknown>;
  onError?: (error: unknown) => void;
  specimen?: boolean;
  compact?: boolean;
}) {
  const [pending, setPending] = useState(false);
  const toggle = { action: 'toggle' as const, label: playback?.playing ? 'Pause' : 'Play', icon: playback?.playing ? Pause : Play, enabled: playback?.canToggle };
  const buttons = compact ? [toggle] : [
    { action: 'previous' as const, label: 'Previous track', icon: SkipBack, enabled: playback?.canPrevious },
    toggle,
    { action: 'next' as const, label: 'Next track', icon: SkipForward, enabled: playback?.canNext },
  ];

  async function control(action: MediaAction) {
    setPending(true);
    try { await onAction(action); }
    catch (error) { (onError ?? console.error)(error); }
    finally { setPending(false); }
  }

  return <div className={className} style={style} role={specimen ? undefined : 'group'} aria-label={specimen ? undefined : label}>
    {buttons.map(({ action, label, icon: Icon, enabled }) => specimen
      ? <span key={action} data-control={action}><Icon fill="currentColor" /></span>
      : <button key={action} type="button" data-control={action} aria-label={label} title={label} data-no-drag={compact ? undefined : true}
        disabled={pending || !enabled} onClick={() => void control(action)}><Icon fill="currentColor" /></button>)}
  </div>;
}
