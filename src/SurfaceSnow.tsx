import { invoke } from '@tauri-apps/api/core';
import { useId, useLayoutEffect, useMemo, useRef, useState, type CSSProperties } from 'react';
import { panelSnowPath } from './dockSnow';
import { desktop, type AppAppearance } from './model';

export function SnowPath({ d }: { d?: string }) {
  const gradient = useId();
  return <>
    <defs><linearGradient id={gradient} x2="0%" y2="100%">
      <stop stopColor="var(--snow-crown, #fcfeff)" />
      <stop offset="48%" stopColor="var(--snow-middle, #f5faff)" />
      <stop offset="100%" stopColor="var(--snow-shade)" />
    </linearGradient></defs>
    <path d={d} fill={`url(#${gradient})`} />
  </>;
}

function NativeSurfaceSnow({ theme }: { theme?: AppAppearance['theme'] }) {
  const anchor = useRef<HTMLSpanElement>(null);
  useLayoutEffect(() => () => {
    void invoke('set_surface_snow', { appearance: null }).catch(console.error);
  }, []);

  useLayoutEffect(() => {
    const panel = anchor.current?.parentElement;
    if (!panel) return;
    const systemLight = matchMedia('(prefers-color-scheme: light)');
    const reducedMotion = matchMedia('(prefers-reduced-motion: reduce)');
    const started = performance.now();
    let timer = 0;
    let frame = 0;
    const cancel = () => {
      window.clearTimeout(timer);
      window.cancelAnimationFrame(frame);
    };
    const publish = () => {
      const style = getComputedStyle(panel);
      // The native roof lines up when the panel's entrance animation reaches its resting edge.
      if (Number(style.opacity) < 1 && performance.now() - started < 280) {
        timer = window.setTimeout(publish, 16);
        return;
      }
      const radius = theme ? 8 : Number.parseFloat(style.borderTopLeftRadius) || 0;
      const light = theme === 'light' || (theme === 'system' && systemLight.matches);
      const present = () => {
        cancel();
        void invoke('set_surface_snow', { appearance: { radius, light, motion: !reducedMotion.matches } }).catch(console.error);
      };
      frame = window.requestAnimationFrame(() => { frame = window.requestAnimationFrame(present); });
      timer = window.setTimeout(present, 90);
    };
    const schedule = () => {
      cancel();
      timer = window.setTimeout(publish, 0);
    };
    schedule();
    systemLight.addEventListener('change', schedule);
    reducedMotion.addEventListener('change', schedule);
    return () => {
      cancel();
      systemLight.removeEventListener('change', schedule);
      reducedMotion.removeEventListener('change', schedule);
    };
  }, [theme]);

  return <span ref={anchor} hidden />;
}

function BrowserSurfaceSnow() {
  const decoration = useRef<HTMLSpanElement>(null);
  const [{ width, headroom }, setSize] = useState({ width: 0, headroom: 0 });

  useLayoutEffect(() => {
    const element = decoration.current;
    if (!element) return;
    const measure = (width: number, height: number) => {
      const roundedWidth = Math.round(width);
      const headroom = Math.max(0, Math.floor(height) - 1);
      setSize((previous) => previous.width === roundedWidth && previous.headroom === headroom
        ? previous : { width: roundedWidth, headroom });
    };
    const observer = new ResizeObserver(([{ contentRect }]) => measure(contentRect.width, contentRect.height));
    observer.observe(element);
    measure(element.clientWidth, element.clientHeight);

    const visibility = () => element.toggleAttribute('data-paused', document.hidden);
    visibility();
    document.addEventListener('visibilitychange', visibility);
    return () => {
      observer.disconnect();
      document.removeEventListener('visibilitychange', visibility);
    };
  }, []);

  const path = useMemo(() => width > 0 && headroom > 0 ? panelSnowPath(width, headroom) : '', [width, headroom]);
  const count = width ? Math.min(14, Math.max(4, Math.round(width / 55))) : 0;
  return <span ref={decoration} className="surface-snow" aria-hidden="true">
    {Array.from({ length: count }, (_, index) => <i className="surface-snow-flake" key={index} style={{
      left: `${(index + .5 + Math.sin(index * 2.4) * .32) / count * 100}%`,
      '--flake-size': `${1.5 + (index * 11 % 13) / 7}px`,
      '--flake-drift': `${index * 13 % 19 - 9}px`,
      '--flake-fall': `${16 + index * 7 % 12}px`,
      '--flake-time': `${3.4 + (index * 13 % 21) * .18}s`,
      '--flake-delay': `${-.4 - (index * 17 % 29) * .21}s`,
    } as CSSProperties} />)}
    {path && <svg className="surface-snow-rim" height={headroom + 1} viewBox={`0 0 ${width} ${headroom + 1}`} preserveAspectRatio="none">
      <SnowPath d={path} />
    </svg>}
  </span>;
}

export function SurfaceSnow({ theme }: { theme?: AppAppearance['theme'] } = {}) {
  return desktop ? <NativeSurfaceSnow theme={theme} /> : <BrowserSurfaceSnow />;
}
