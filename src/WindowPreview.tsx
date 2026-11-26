import { invoke } from '@tauri-apps/api/core';
import { useEffect, type CSSProperties } from 'react';
import { middleClick } from './DockOverlays';
import { useDockAppearance } from './dockAppearance';
import { defaults, desktop, type Settings } from './model';
import { PlaybackControls, type PlaybackState } from './PlaybackControls';
import { useSurfacePayload } from './useSurfacePayload';

interface PreviewCard {
  id: string;
  title: string;
  left: number;
  top: number;
  width: number;
  height: number;
  header: number;
  image: boolean;
}

interface PreviewLayout {
  token: string;
  open: boolean;
  animated: boolean;
  exitOpacity: number;
  cards: PreviewCard[];
  playback?: PlaybackState | null;
  controlsHeight: number;
}

export function WindowPreview() {
  const layout = useSurfacePayload<PreviewLayout>('preview-layout', 'read_preview_layout');
  const settings = useSurfacePayload<Settings>('settings-changed', 'read_settings');
  const { tint, 'data-spotify-style': spotifyStyle } = useDockAppearance(settings ?? defaults, !!layout);
  const token = layout?.open ? layout.token : undefined;
  useEffect(() => {
    if (!desktop || !token) return;
    let active = true;
    let timer = 0;
    const refresh = async () => {
      try {
        if (!await invoke<boolean>('preview_media', { token })) return;
      } catch (error) { if (active) console.error(error); }
      if (active) timer = window.setTimeout(refresh, 1100);
    };
    void refresh();
    return () => { active = false; window.clearTimeout(timer); };
  }, [token]);

  if (!layout) return null;
  return <div key={layout.token} className="native-preview-strip" data-spotify-style={spotifyStyle} data-open={layout.open} data-animated={layout.animated} aria-label="Open windows" inert={!layout.open}
    style={{ '--exit-opacity': layout.exitOpacity, '--dock-tint': tint } as CSSProperties}>
    {layout.cards.map((card) => <button key={card.id} className="native-preview-card" aria-label={`Switch to ${card.title}`}
      style={{ left: card.left, top: card.top, width: card.width, height: card.height }}
      {...middleClick(() => void invoke('close_preview_window', { id: card.id, token: layout.token }).catch(console.error))}
      onClick={() => void invoke('activate_preview_window', { id: card.id }).catch(console.error)}>
      <span className="native-preview-title" style={{ height: card.header }}>{card.title}</span>
      {!card.image && <span className="native-preview-fallback">Preview unavailable</span>}
    </button>)}
    {layout.playback && layout.controlsHeight > 0 && <PlaybackControls label="Media playback" className="native-preview-media" playback={layout.playback}
      style={{ height: layout.controlsHeight }} onAction={(action) => invoke('preview_media', { token: layout.token, action })} />}
  </div>;
}
