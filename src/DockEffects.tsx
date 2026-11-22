import { invoke } from '@tauri-apps/api/core';
import { emitTo, listen } from '@tauri-apps/api/event';
import { useCallback, useEffect, useLayoutEffect, useRef, useState, type RefObject } from 'react';
import { flakeLifetime, flakePosition, type DockSnow, type EffectRect, type Snowflake, type SnowLedge, type SnowParticles, type SnowShape } from './dockSnow';
import { verticalDock, type Settings } from './model';
import { SnowPath } from './SurfaceSnow';
import { useSurfacePayload } from './useSurfacePayload';

interface EffectsScene {
  opacity: number;
  offsetX: number;
  offsetY: number;
}

const sampleInterval = 32;
const particleEvent = 'dock-snow-particles';
const shapes = new WeakMap<SVGSVGElement, SnowShape>();
const outlineSize = 32;
const outlines = new WeakMap<HTMLImageElement, { source: string; top?: (number | null)[] }>();
const quarterPixel = (value: number) => Math.round(value * 4) / 4;
let outlineCanvas: HTMLCanvasElement | undefined;

function iconOutline(image: HTMLImageElement) {
  if (!image.complete || !image.naturalWidth || !image.naturalHeight) return;
  const source = image.currentSrc;
  const previous = outlines.get(image);
  if (previous?.source === source) return previous.top;
  const result: { source: string; top?: (number | null)[] } = { source };
  outlines.set(image, result);
  const canvas = outlineCanvas ??= document.createElement('canvas');
  canvas.width = canvas.height = outlineSize;
  const context = canvas.getContext('2d', { willReadFrequently: true });
  if (!context) return;
  const scale = outlineSize / Math.max(image.naturalWidth, image.naturalHeight);
  const width = image.naturalWidth * scale;
  const height = image.naturalHeight * scale;
  try {
    context.drawImage(image, (outlineSize - width) / 2, (outlineSize - height) / 2, width, height);
    const pixels = context.getImageData(0, 0, outlineSize, outlineSize).data;
    const top = Array.from({ length: outlineSize }, (_, x) => {
      for (let y = 0; y < outlineSize; y++) {
        if (pixels[(y * outlineSize + x) * 4 + 3] >= 100) return y / outlineSize;
      }
      return null;
    });
    if (top.some((row) => row !== null)) {
      result.top = top.map((row, index) => {
        if (row === null) return null;
        const previous = top[index - 1] ?? row;
        const next = top[index + 1] ?? row;
        if (Math.max(Math.abs(row - previous), Math.abs(row - next)) > 4 / outlineSize) return null;
        return (previous + 2 * row + next) / 4;
      });
      if (!result.top.some((row) => row !== null)) result.top = top;
    }
  } catch {
    // Some external image sources disallow pixel reads. Use the icon's visible box.
  }
  return result.top;
}

export function DockSnowCap({ id, snow }: { id: string; snow: DockSnow }) {
  const attach = useCallback((element: SVGSVGElement | null) => {
    if (element) snow.paintCap(id, element);
  }, [id, snow]);
  return <svg ref={attach} className="dock-snow-cap" data-snow-ledge={id}
    aria-hidden="true" preserveAspectRatio="none">
    <SnowPath />
  </svg>;
}

function measureSnow(frame: HTMLElement, body: HTMLElement, settings: Settings) {
  const origin = frame.getBoundingClientRect();
  const measure = (element: Element): EffectRect => {
    const rect = element.getBoundingClientRect();
    return { x: quarterPixel(rect.left - origin.left), y: quarterPixel(rect.top - origin.top),
      width: quarterPixel(rect.width), height: quarterPixel(rect.height) };
  };
  const surface = body.querySelector('.dock-surface');
  const strip = measure(surface ?? body);
  const ledges: SnowLedge[] = [];
  const caps: SVGSVGElement[] = [];
  let { x: left, y: top } = strip;
  let right = left + strip.width;
  let bottom = top + strip.height;
  function include(rect: EffectRect) {
    left = Math.min(left, rect.x);
    top = Math.min(top, rect.y);
    right = Math.max(right, rect.x + rect.width);
    bottom = Math.max(bottom, rect.y + rect.height);
  }
  function ledge(element: Element, rect: EffectRect, kind: 'dock' | 'icon' | 'widget', image?: HTMLImageElement | null) {
    const cap = element.querySelector<SVGSVGElement>(':scope > .dock-snow-cap');
    const id = cap?.dataset.snowLedge;
    if (!cap || !id || rect.width < 2 || rect.height < 2) return;
    const width = cap.clientWidth;
    const height = cap.clientHeight;
    if (width < 2 || height < 2) return;
    const paintedTile = settings.appIconBackground !== 'transparent';
    const outline = kind === 'icon' && image && !paintedTile ? iconOutline(image) : undefined;
    const rounding = settings.appIconShape === 'circle' ? .5 : paintedTile || settings.appIconShape === 'rounded' || !image ? .24 : 0;
    const radius = kind === 'dock' ? 16 : kind === 'widget' ? 11 : Math.min(width, height) * rounding;
    let shape = shapes.get(cap);
    if (!shape || shape.width !== width || shape.height !== height || shape.radius !== radius || shape.outline !== outline) {
      shape = { width, height, radius, outline };
      shapes.set(cap, shape);
    }
    ledges.push({ ...(kind === 'dock' ? measure(cap) : rect), id, dock: kind === 'dock', shape });
    caps.push(cap);
  }
  if (surface) ledge(surface, strip, 'dock');

  for (const slot of body.querySelectorAll<HTMLElement>('.dock-slot:not([data-dragged="true"])')) {
    const art = slot.querySelector('.dock-click-art');
    const icon = art?.querySelector<HTMLElement>('.dock-app-image');
    if (!art || !icon) continue;
    const rect = measure(icon);
    if (rect.width < 2 || rect.height < 2) continue;
    include(rect);
    ledge(art, rect, 'icon', icon.querySelector('img'));
  }

  for (const card of body.querySelectorAll<HTMLElement>(':scope > .dock-widget')) {
    if (card.dataset.kind === 'spacer' || card.dataset.kind === 'divider') continue;
    const rect = measure(card);
    if (rect.width < 2 || rect.height < 2) continue;
    include(rect);
    ledge(card, rect, 'widget');
  }

  return { area: { x: left, y: top, width: right - left, height: bottom - top }, ledges, caps };
}

function SnowCanvas({ scene, local }: { scene: EffectsScene; local?: ReadonlyMap<number, Snowflake> }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const latest = useRef(scene);
  const [remote] = useState(() => new Map<number, Snowflake>());

  useLayoutEffect(() => { latest.current = scene; }, [scene]);

  useEffect(() => {
    if (local) return;
    const subscription = listen<SnowParticles>(particleEvent, ({ payload }) => {
      for (const particle of payload.added) remote.set(particle.id, particle);
      for (const id of payload.removed) remote.delete(id);
    });
    return () => { void subscription.then((unlisten) => unlisten()).catch(console.error); };
  }, [local, remote]);

  useEffect(() => {
    const element = canvas.current;
    const context = element?.getContext('2d');
    if (!element || !context) return;
    const particles = local ?? remote;
    let width = 0;
    let height = 0;
    let scale = 1;
    let timer: number | undefined;
    let dirty: EffectRect | undefined;
    function paint() {
      if (!context || width <= 0 || height <= 0) return;
      context.setTransform(scale, 0, 0, scale, 0, 0);
      if (dirty) context.clearRect(dirty.x, dirty.y, dirty.width, dirty.height);
      const now = performance.timeOrigin + performance.now();
      const { offsetX, offsetY } = latest.current;
      let minX = Infinity;
      let minY = Infinity;
      let maxX = -Infinity;
      let maxY = -Infinity;
      context.fillStyle = '#f3faff';
      for (const flake of particles.values()) {
        const position = flakePosition(flake, now);
        if (position.age >= flakeLifetime) {
          if (!local) remote.delete(flake.id);
          continue;
        }
        const x = position.x + offsetX;
        const y = position.y + offsetY;
        const radius = flake.radius;
        if (x < -radius || y < -radius || x > width + radius || y > height + radius) continue;
        minX = Math.min(minX, x - radius);
        minY = Math.min(minY, y - radius);
        maxX = Math.max(maxX, x + radius);
        maxY = Math.max(maxY, y + radius);
        context.globalAlpha = Math.min(.9, (position.age + (flake.shedFrom ? .28 : 0)) * 3, (flakeLifetime - position.age) * 1.8);
        context.beginPath();
        context.arc(x, y, radius, 0, Math.PI * 2);
        context.fill();
      }
      dirty = minX === Infinity ? undefined : { x: minX - 2, y: minY - 2, width: maxX - minX + 4, height: maxY - minY + 4 };
    }
    function tick() {
      if (document.hidden) return;
      paint();
      timer = window.setTimeout(tick, 16);
    }
    function syncVisibility() {
      window.clearTimeout(timer);
      tick();
    }
    const observer = new ResizeObserver(([entry]) => {
      width = entry.contentRect.width;
      height = entry.contentRect.height;
      scale = Math.min(devicePixelRatio, 2, Math.sqrt(1_600_000 / Math.max(1, width * height)));
      element.width = Math.max(1, Math.ceil(width * scale));
      element.height = Math.max(1, Math.ceil(height * scale));
      dirty = undefined;
    });
    observer.observe(element);
    document.addEventListener('visibilitychange', syncVisibility);
    syncVisibility();
    return () => {
      observer.disconnect();
      window.clearTimeout(timer);
      document.removeEventListener('visibilitychange', syncVisibility);
    };
  }, [local, remote]);

  return <canvas ref={canvas} className="dock-effect-snow" aria-hidden="true" style={{ opacity: scene.opacity }} />;
}

export function DesktopDockEffects() {
  const scene = useSurfacePayload<EffectsScene>('dock-effects-changed', 'read_dock_effects');
  return scene ? <SnowCanvas scene={scene} /> : null;
}

export function DockEffects({ frame, body, settings, visible, motion, preview, snow }: {
  frame: RefObject<HTMLDivElement | null>; body: RefObject<HTMLDivElement | null>;
  settings: Settings; visible: boolean; motion: boolean; preview: boolean; snow: DockSnow;
}) {
  const [scene, setScene] = useState<EffectsScene>();
  const prepared = useRef<Promise<void>>(undefined);
  const shown = useRef(false);

  useEffect(() => () => {
    if (!preview && prepared.current) void invoke('update_dock_effects', { opacity: null }).catch(console.error);
  }, [preview]);

  useEffect(() => {
    let stopped = false;
    let sending = false;
    let pending: number | null | undefined;
    let timer: number | undefined;
    let lastOpacity: number | null | undefined;
    let lastSent = -Infinity;
    let lastStep = 0;

    async function sendScene() {
      sending = true;
      try {
        while (!stopped && pending !== undefined) {
          const opacity = pending;
          pending = undefined;
          if (opacity !== null) {
            prepared.current ??= invoke<void>('prepare_dock_effects').catch((error) => {
              prepared.current = undefined;
              throw error;
            });
            await prepared.current;
          } else if (!shown.current) {
            continue;
          }
          if (!stopped) {
            await invoke('update_dock_effects', { opacity });
            shown.current = opacity !== null;
          }
        }
      } catch (error) {
        console.error(error);
      } finally {
        sending = false;
      }
    }

    function publish(opacity: number | null, now: number) {
      if (lastOpacity === opacity && (preview || opacity === null || now - lastSent < 500)) return;
      lastOpacity = opacity;
      lastSent = now;
      if (preview) setScene(opacity === null ? undefined : { opacity, offsetX: 0, offsetY: 0 });
      else {
        pending = opacity;
        if (!sending) void sendScene();
      }
    }

    function flushParticles() {
      const changes = snow.takeChanges();
      if (changes && !preview && shown.current) {
        void emitTo('dock-effects', particleEvent, changes).catch(console.error);
      }
    }

    function sample() {
      const now = performance.now();
      if (stopped) return;
      const dock = body.current;
      const viewport = frame.current;
      const opacity = dock ? Math.round(Number(getComputedStyle(dock).opacity) * 100) / 100 : 0;
      if (!dock || !viewport || document.hidden || (!visible && opacity <= .01)) {
        snow.clearFlakes();
        flushParticles();
        if (shown.current || lastOpacity != null) publish(null, now);
        lastStep = 0;
        return;
      }
      const { area, ledges, caps } = measureSnow(viewport, dock, settings);
      const clock = performance.timeOrigin + now;
      snow.setLedges(ledges, motion && lastStep > 0, clock);
      if (motion && lastStep) snow.step(Math.min(.07, (now - lastStep) / 1000), clock, area, verticalDock(settings.edge));
      for (const cap of caps) snow.paintCap(cap.dataset.snowLedge!, cap);
      flushParticles();
      lastStep = now;
      publish(motion ? opacity : null, now);
      timer = window.setTimeout(sample, sampleInterval);
    }

    function visibilityChanged() {
      window.clearTimeout(timer);
      lastStep = 0;
      sample();
    }
    document.addEventListener('visibilitychange', visibilityChanged);
    sample();
    return () => {
      stopped = true;
      window.clearTimeout(timer);
      document.removeEventListener('visibilitychange', visibilityChanged);
    };
  }, [frame, body, settings, visible, motion, preview, snow]);

  return preview && scene ? <SnowCanvas scene={scene} local={snow.particles} /> : null;
}
