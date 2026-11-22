import { invoke } from '@tauri-apps/api/core';
import { ArrowUpRight, AudioLines, Music2 } from 'lucide-react';
import { useCallback, useEffect, useLayoutEffect, useRef, useState, type CSSProperties } from 'react';
import { useHoverPopup, type HoverPopupKind } from './HoverPopup';
import { defaults, desktop, type Settings } from './model';
import { PlaybackControls, type MediaAction, type PlaybackState } from './PlaybackControls';
import { useSurfacePayload } from './useSurfacePayload';
import { isWeatherKind, type SpotifyKind } from './widgetCatalog';
import { widgetSnapshot } from './widgetSnapshots';
import { WeatherPanel } from './WeatherWidgets';

interface SpotifyTimeline {
  position: number;
  duration: number;
  updatedAt: number | null;
  playbackRate: number;
  canSeek: boolean;
}

interface SpotifyTrack extends PlaybackState {
  available: boolean;
  title: string;
  artist: string;
  artwork?: string;
  timeline?: SpotifyTimeline | null;
}

const spotify = widgetSnapshot<SpotifyTrack>('spotify_snapshot', 1100, 'spotify-changed');
const previewTrack: SpotifyTrack = {
  available: true, title: 'Sample track', artist: 'Sample artist',
  playing: false, canPrevious: true, canToggle: true, canNext: true,
};
type PlaybackProps = { track?: SpotifyTrack; specimen?: boolean; onError?: (error: unknown) => void };

function trackTitle(track: SpotifyTrack | undefined, specimen?: boolean) {
  return track?.title || (specimen ? 'Spotify' : track?.available ? 'No track playing' : 'Open Spotify to play');
}

function Artwork({ track, className }: { track?: SpotifyTrack; className?: string }) {
  return <span className={`spotify-artwork${className ? ` ${className}` : ''}`} data-loaded={!!track?.artwork} aria-hidden="true">
    <Music2 />
    {track?.artwork && <img src={track.artwork} alt="" draggable={false} />}
  </span>;
}

async function controlSpotify(action: MediaAction) {
  await invoke('spotify_control', { action });
  await spotify.refresh();
}

function SpotifyControls({ track, ...props }: PlaybackProps & { compact?: boolean }) {
  return <PlaybackControls playback={track} label="Spotify controls" className="spotify-controls" onAction={controlSpotify} {...props} />;
}

function TrackInfo({ track, specimen, tooltip }: Pick<PlaybackProps, 'track' | 'specimen'> & { tooltip?: boolean }) {
  const title = trackTitle(track, specimen);
  const label = track?.artist ? `${title} · ${track.artist}` : title;
  return <div className="spotify-track" title={tooltip ? label : undefined}>
    <strong>{title}</strong>{track?.artist && <small>{track.artist}</small>}
  </div>;
}

function playbackTime(seconds: number) {
  const wholeSeconds = Math.floor(seconds);
  const minutes = Math.floor(wholeSeconds / 60);
  const remainder = String(wholeSeconds % 60).padStart(2, '0');
  return `${minutes}:${remainder}`;
}

const wavePeriod = 1600;
const waveCenter = 6;
const defaultAmplitude = 2.7;

function progressWavePath(width: number, phase: number, amplitude: number) {
  if (width <= 0) return '';
  const angle = phase * 2 * Math.PI;
  const radiansPerPixel = 2 * Math.PI / 32;
  let path = `M0 ${waveCenter}`;
  let lastX = 0;
  let lastY = waveCenter;
  for (let x = 3; x < width; x += 3) {
    const edge = Math.min(x, width - x, 12) / 12;
    const taper = edge * edge * (3 - 2 * edge);
    const y = waveCenter + amplitude * taper * Math.sin(x * radiansPerPixel - angle);
    // Joining the sample midpoints keeps the curve smooth as its phase changes.
    path += `Q${lastX} ${lastY.toFixed(2)} ${(lastX + x) / 2} ${((lastY + y) / 2).toFixed(2)}`;
    lastX = x;
    lastY = y;
  }
  return `${path}Q${lastX} ${lastY.toFixed(2)} ${width.toFixed(2)} ${waveCenter}`;
}

function ProgressWave({ active }: { active: boolean }) {
  const bounds = useRef<HTMLSpanElement>(null);
  const curve = useRef<SVGPathElement>(null);
  const width = useRef(0);
  const phase = useRef(0);
  const amplitude = useRef(defaultAmplitude);
  const speed = useRef(1);

  useLayoutEffect(() => {
    const element = bounds.current;
    const path = curve.current;
    if (!element || !path) return;
    const observer = new ResizeObserver(([entry]) => {
      width.current = entry.contentRect.width;
      path.setAttribute('d', progressWavePath(width.current, phase.current, amplitude.current));
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    const path = curve.current;
    if (!path || !active) return;
    const motion = matchMedia('(prefers-reduced-motion: no-preference) and (forced-colors: none)');
    let stop: (() => void) | undefined;
    const updateMotion = () => {
      stop?.();
      stop = undefined;
      if (!motion.matches) return;
      let lastFrame = performance.now();
      let frameTimer = 0;
      let sampleTimer = 0;
      let stopped = false;
      let averageDb: number | undefined;
      let levelVariation = .4;
      let targetAmplitude = defaultAmplitude;
      let targetSpeed = 1;

      const sample = async () => {
        const peak = await invoke<number | null>('spotify_output_peak').catch(() => null);
        if (stopped) return;
        const hasPeak = typeof peak === 'number' && Number.isFinite(peak);
        if (hasPeak) {
          const decibels = 20 * Math.log10(Math.max(.001, Math.min(1, peak)));
          const reference = averageDb ?? decibels;
          const change = decibels - reference;
          // Use the track's recent range so even similarly loud peaks produce visible accents.
          const range = Math.max(.28, levelVariation) * 2.2;
          const audible = Math.min(1, (decibels + 60) / 20);
          const intensity = audible * Math.min(1, Math.max(0, .45 + change / range));
          averageDb = audible > 0 ? reference + change * .035 : undefined;
          levelVariation = audible > 0 ? levelVariation + (Math.abs(change) - levelVariation) * .035 : .4;
          targetAmplitude = .75 + 4.25 * intensity;
          targetSpeed = .42 + 1.78 * intensity;
        } else {
          averageDb = undefined;
          levelVariation = .4;
          targetAmplitude = defaultAmplitude;
          targetSpeed = 1;
        }
        sampleTimer = window.setTimeout(sample, hasPeak ? 32 : 240);
      };

      // Repaint the path in unfocused transparent Windows WebViews as well.
      const draw = () => {
        const now = performance.now();
        const elapsed = now - lastFrame;
        lastFrame = now;
        const amplitudeResponse = targetAmplitude > amplitude.current ? 30 : 120;
        amplitude.current = targetAmplitude + (amplitude.current - targetAmplitude) * Math.exp(-elapsed / amplitudeResponse);
        const previousSpeed = speed.current;
        const speedResponse = targetSpeed > previousSpeed ? 85 : 275;
        speed.current = targetSpeed + (previousSpeed - targetSpeed) * Math.exp(-elapsed / speedResponse);
        // Accumulate movement so changes in speed never restart the wave.
        const frameSpeed = (previousSpeed + speed.current) / 2;
        phase.current = (phase.current + frameSpeed * elapsed / wavePeriod) % 1;
        path.setAttribute('d', progressWavePath(width.current, phase.current, amplitude.current));
        frameTimer = window.setTimeout(draw, 16);
      };
      draw();
      if (desktop) void sample();
      stop = () => {
        stopped = true;
        window.clearTimeout(frameTimer);
        window.clearTimeout(sampleTimer);
      };
    };
    updateMotion();
    motion.addEventListener('change', updateMotion);
    return () => {
      stop?.();
      motion.removeEventListener('change', updateMotion);
    };
  }, [active]);

  return <span className="spotify-progress-wave" ref={bounds}>
    <svg focusable="false"><path ref={curve} /></svg>
  </span>;
}

function TrackProgress({ timeline, playing, onError }: {
  timeline: SpotifyTimeline; playing: boolean; onError: (error: unknown) => void;
}) {
  const [now, setNow] = useState(Date.now);
  const [preview, setPreview] = useState<number>();
  const [seeking, setSeeking] = useState(false);
  const [visible, setVisible] = useState(false);
  const slider = useRef<HTMLInputElement>(null);
  const rail = useRef<HTMLSpanElement>(null);
  const advancing = playing && timeline.updatedAt !== null;
  useEffect(() => {
    const element = rail.current;
    if (!element) return;
    let intersecting = false;
    const updateVisibility = () => setVisible(intersecting && !document.hidden);
    const observer = new IntersectionObserver(([entry]) => {
      intersecting = entry.isIntersecting;
      updateVisibility();
    });
    observer.observe(element);
    document.addEventListener('visibilitychange', updateVisibility);
    return () => {
      observer.disconnect();
      document.removeEventListener('visibilitychange', updateVisibility);
    };
  }, []);

  useEffect(() => {
    if (!advancing || !visible) return;
    setNow(Date.now());
    const timer = window.setInterval(() => setNow(Date.now()), 200);
    return () => window.clearInterval(timer);
  }, [advancing, visible]);

  const elapsedSinceUpdate = advancing ? Math.max(0, now - (timeline.updatedAt ?? now)) / 1000 : 0;
  const projected = timeline.position + elapsedSinceUpdate * timeline.playbackRate;
  const position = Math.min(timeline.duration, preview ?? projected);
  const elapsed = playbackTime(position);
  const remaining = playbackTime(Math.ceil(timeline.duration - position));
  const valueText = `${elapsed} of ${playbackTime(timeline.duration)}`;
  const minuteDigits = Math.max(2, playbackTime(Math.ceil(timeline.duration)).length - 3);
  const timeSlot = `${'0'.repeat(minuteDigits)}:00`;

  useEffect(() => {
    const input = slider.current;
    if (!input || seeking) return;
    const seek = async () => {
      const position = input.valueAsNumber;
      setSeeking(true);
      try {
        await invoke('spotify_control', { action: 'seek', position });
        await spotify.refresh();
      } catch (error) {
        onError(error);
      } finally {
        setSeeking(false);
        setPreview(undefined);
      }
    };
    // The native change event fires when a drag commits. React's onChange also fires as the thumb moves.
    input.addEventListener('change', seek);
    return () => input.removeEventListener('change', seek);
  }, [timeline.canSeek, seeking, onError]);

  const waveActive = visible && playing && !seeking && preview === undefined && position > 0 && position < timeline.duration;
  return <div className="spotify-progress"
    style={{ '--played': `${position / timeline.duration * 100}%` } as CSSProperties}>
    <span className="spotify-progress-time" data-slot={timeSlot} aria-hidden="true"><span>{elapsed}</span></span>
    <div className="spotify-progress-control">
      <span className="spotify-progress-rail" ref={rail} aria-hidden="true">
        <ProgressWave active={waveActive} />
      </span>
      {timeline.canSeek
        ? <input ref={slider} type="range" min={0} max={timeline.duration} step="any" value={position} aria-disabled={seeking}
            aria-label="Playback position" aria-valuetext={valueText}
            onChange={(event) => { if (!seeking) setPreview(event.currentTarget.valueAsNumber); }}
            onPointerDown={(event) => { if (seeking) event.preventDefault(); }}
            onPointerCancel={() => { if (!seeking) setPreview(undefined); }}
            onKeyDown={(event) => { if (seeking && event.key !== 'Tab') event.preventDefault(); }} />
        : <progress aria-label="Playback progress" aria-valuetext={valueText} max={timeline.duration} value={position} />}
    </div>
    <span className="spotify-progress-time" data-slot={`−${timeSlot}`} aria-hidden="true"><span>−{remaining}</span></span>
  </div>;
}

export function SpotifyFace({ kind, widget, specimen, live, onError }: {
  kind: SpotifyKind; widget?: string; specimen: boolean; live?: boolean; onError?: (error: unknown) => void;
}) {
  const snapshot = spotify.useValue(!specimen || !!live);
  const track = specimen && !snapshot?.title ? previewTrack : snapshot;
  const compact = kind === 'spotify-mini';
  const cover = kind === 'spotify' || compact;
  const trackCard = kind === 'spotify-card';
  const interaction = !specimen && widget;
  const face = useRef<HTMLDivElement>(null);
  const open = useHoverPopup(face, interaction || undefined, 'spotify', onError);

  return <div ref={face} className={`widget-face ${cover ? 'spotify-face' : 'spotify-track-face'} ${kind}`} role={interaction ? 'group' : undefined}
    tabIndex={interaction ? 0 : undefined} aria-label={interaction ? `Spotify, ${trackTitle(track)}` : undefined}
    onPointerEnter={interaction ? (event) => { if (event.pointerType !== 'touch') open(event.currentTarget); } : undefined}
    onClick={interaction ? (event) => { if (!(event.target as Element).closest('button')) open(event.currentTarget, true); } : undefined}
    onKeyDown={interaction ? (event) => {
      if (event.target === event.currentTarget && !event.repeat && (event.key === 'Enter' || event.key === ' ')) {
        event.preventDefault();
        open(event.currentTarget, true, true);
      }
    } : undefined}>
    {cover && <Artwork track={track} className="spotify-backdrop" />}
    {compact ? <SpotifyControls track={track} specimen={specimen} compact onError={onError} />
      : <>
        {trackCard && <Artwork track={track} className="spotify-tile" />}
        {kind === 'spotify-text' && <AudioLines className="spotify-text-icon" aria-hidden="true" />}
        <div className="spotify-card-info">
          <TrackInfo track={track} specimen={specimen} />
          {kind !== 'spotify-text' && <SpotifyControls track={track} specimen={specimen} onError={onError} />}
        </div>
      </>}
  </div>;
}

type WidgetPopupDetails = { id: number; kind: HoverPopupKind; keyboard: boolean; edge: Settings['edge']; animated: boolean; closing: boolean };

function SpotifyPlayerContent({ onOpened, onError }: {
  onOpened: () => Promise<void>;
  onError: (error: unknown) => void;
}) {
  const track = spotify.useValue(true);
  const [opening, setOpening] = useState(false);

  async function openSpotify() {
    setOpening(true);
    try {
      await invoke('open_spotify');
      await onOpened();
    } catch (error) {
      onError(error);
    } finally {
      setOpening(false);
    }
  }

  return <>
    <Artwork track={track} className="spotify-player-cover" />
    <button className="spotify-player-app" type="button" aria-label="Open Spotify" title="Open Spotify"
      disabled={opening} onClick={() => void openSpotify()}><ArrowUpRight aria-hidden="true" /></button>
    <div className="spotify-player-info">
      <TrackInfo track={track} tooltip />
      {track?.timeline && <TrackProgress key={`${track.title}\0${track.artist}`} timeline={track.timeline}
        playing={track.playing} onError={onError} />}
      <SpotifyControls track={track} onError={onError} />
    </div>
  </>;
}

export function StartSpotify({ onError }: { onError: (error: unknown) => void }) {
  return <section className="spotify-player start-spotify" aria-label="Spotify">
    <SpotifyPlayerContent onError={onError} onOpened={() => invoke('close_system_panel')} />
  </section>;
}

export function WidgetPopup() {
  const popup = useSurfacePayload<WidgetPopupDetails>('widget-popup-changed', 'read_widget_popup');
  const weatherKind = popup && isWeatherKind(popup.kind) ? popup.kind : undefined;
  const cleanWeather = weatherKind === 'weather';
  const settings = useSurfacePayload<Settings>('settings-changed', 'read_settings', cleanWeather) ?? defaults;
  const panel = useRef<HTMLElement>(null);
  const reveal = useRef(0);
  const [error, setError] = useState('');
  const onError = useCallback((error: unknown) => setError(String(error)), []);
  const close = useCallback(() => invoke<void>('close_widget_popup'), []);

  useLayoutEffect(() => {
    const element = panel.current;
    if (!element || !popup) {
      reveal.current = 0;
      return;
    }
    const { id, closing, animated } = popup;
    const from = reveal.current;
    const to = closing ? 0 : 1;
    const reducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
    const fullDuration = reducedMotion ? 90 : closing ? 130 : 185;
    const duration = animated ? fullDuration * Math.abs(to - from) : 0;
    const started = performance.now();
    let timer = 0;

    // The transparent Windows WebView can stall compositor-driven transitions.
    const draw = () => {
      const position = duration ? Math.min(1, (performance.now() - started) / duration) : 1;
      const eased = closing ? position ** 2 : 1 - (1 - position) ** 3;
      reveal.current = from + (to - from) * eased;
      element.style.setProperty('--panel-reveal', String(reveal.current));
      if (position < 1) {
        timer = window.setTimeout(draw, 16);
      } else if (closing) {
        void invoke('finish_widget_popup_exit', { id }).catch(console.error);
      }
    };

    draw();
    return () => window.clearTimeout(timer);
  }, [popup]);

  useEffect(() => {
    if (!popup || popup.closing) return;
    setError('');
    if (popup.keyboard) {
      const target = panel.current?.querySelector<HTMLButtonElement>('button[data-control="toggle"]:enabled') ?? panel.current;
      target?.focus();
    }
    const dismiss = (event: KeyboardEvent) => {
      if (event.key === 'Escape' && !event.defaultPrevented) void close().catch(onError);
    };
    window.addEventListener('keydown', dismiss);
    return () => window.removeEventListener('keydown', dismiss);
  }, [popup, close, onError]);

  if (!popup) return null;
  const frame = {
    'data-edge': popup.edge,
    'data-closing': popup.closing,
    ref: panel,
    tabIndex: -1,
    inert: popup.closing,
  };
  if (weatherKind) {
    const weatherFrame = cleanWeather ? {
      ...frame, 'data-material': settings.material, style: { '--dock-tint': settings.tintColor } as CSSProperties,
    } : frame;
    return <WeatherPanel kind={weatherKind} frame={weatherFrame} keyboard={popup.keyboard} error={error} onError={onError} />;
  }
  return <main {...frame} className="spotify-player spotify-popup widget-popup-motion" role="dialog" aria-label="Spotify playback">
    <SpotifyPlayerContent onError={onError} onOpened={close} />
    {error && <p className="spotify-popup-error" role="alert" title={error}>{error}</p>}
  </main>;
}
