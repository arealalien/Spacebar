import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { readBrowserDock, writeBrowserDock } from './dockStorage';

export type DockEdge = 'bottom' | 'top' | 'left' | 'right';
export const verticalDock = (edge: DockEdge) => edge === 'left' || edge === 'right';

export const accents = {
  red: { label: 'Red', color: '#ef666c' },
  orange: { label: 'Orange', color: '#f89b5e' },
  yellow: { label: 'Yellow', color: '#f2cc60' },
  green: { label: 'Green', color: '#79dcb0' },
  blue: { label: 'Blue', color: '#83baff' },
  purple: { label: 'Purple', color: '#c8b6ff' },
  pink: { label: 'Pink', color: '#ed98c0' },
  dark: { label: 'Dark gray', color: '#34363d' },
  light: { label: 'Light gray', color: '#e8e9ed' },
};

const legacyAccents = { lilac: 'purple', mint: 'green', peach: 'orange', sky: 'blue' } as const;

export const appIconShapes = { original: 'Original', rounded: 'Rounded corners', circle: 'Circular' };
export const appIconBackgrounds = {
  transparent: 'Transparent', tinted: 'Tinted', black: 'Black', white: 'White', custom: 'Custom color',
};
export const appIconClickEffects = {
  none: 'Default (no animation)', press: 'Scale down', stars: 'Star pop', squish: 'Squish', bounce: 'Bounce',
};
export const appIconClickSpeedRange = { min: 50, max: 200 };
export const dockPaddingRange = { min: 0, max: 24 };

export interface Settings {
  edge: DockEdge;
  alignment: 'start' | 'center' | 'end';
  fullSpan: boolean;
  iconSize: number;
  spacing: number;
  padding: number;
  offset: number;
  zoom: number;
  falloff: number;
  duration: number;
  magnify: boolean;
  names: boolean;
  previews: boolean;
  previewDisplay: 'window' | 'dock';
  indicators: boolean;
  notificationBadges: 'dot' | 'none' | 'count';
  hideSettingsIcon: boolean;
  hideInFullscreen: boolean;
  autoHide: boolean;
  replaceTaskbar: boolean;
  replaceWindowsStart: boolean;
  revealDuration: number;
  revealAnimation: 'easing' | 'bounce';
  bounceAmount: number;
  material: 'tinted' | 'midnight' | 'clear';
  dockSnow: boolean;
  tintColor: string;
  accent: keyof typeof accents;
  appIconShape: keyof typeof appIconShapes;
  appIconBackground: keyof typeof appIconBackgrounds;
  appIconColor: string;
  appIconClickEffect: keyof typeof appIconClickEffects;
  appIconClickSpeed: number;
}

export interface AppAppearance {
  theme: 'system' | 'light' | 'dark';
  tintColor: string;
  blurAmount: number;
  tintAmount: number;
}

export const appAppearanceAmountRange = { min: 0, max: 100 };
export const defaultAppAppearance: AppAppearance = {
  theme: 'system', tintColor: accents.purple.color, blurAmount: 100, tintAmount: 100,
};

export const defaults: Settings = {
  edge: 'bottom', alignment: 'center', fullSpan: false,
  iconSize: 46, spacing: 10, padding: 8, offset: 12, zoom: 1.55, falloff: 2.1, duration: 420,
  magnify: true, names: true, previews: true, previewDisplay: 'window', indicators: true, notificationBadges: 'dot',
  hideSettingsIcon: false, hideInFullscreen: true, autoHide: false, replaceTaskbar: true, replaceWindowsStart: false,
  revealDuration: 260, revealAnimation: 'easing', bounceAmount: 35,
  material: 'tinted', tintColor: '#c8b6ff', dockSnow: false,
  accent: 'purple',
  appIconShape: 'original', appIconBackground: 'transparent', appIconColor: '#30343e',
  appIconClickEffect: 'none', appIconClickSpeed: 100,
};

export function normalizeHexColor(value: unknown) {
  if (typeof value !== 'string') return;
  const hex = value.trim().replace(/^#/, '').toLowerCase();
  if (/^[a-f\d]{6}$/.test(hex)) return '#' + hex;
  if (/^[a-f\d]{3}$/.test(hex)) return '#' + Array.from(hex, (digit) => digit.repeat(2)).join('');
}

export function contrastingInk(hex: string) {
  const [red, green, blue] = [1, 3, 5].map((index) => {
    const channel = Number.parseInt(hex.slice(index, index + 2), 16) / 255;
    return channel <= .04045 ? channel / 12.92 : ((channel + .055) / 1.055) ** 2.4;
  });
  return .2126 * red + .7152 * green + .0722 * blue > .179 ? '#000' : '#fff';
}

export const desktop = isTauri();
export const settingsStorageKey = 'spacebar.settings';

export interface AppWindow { id: string; title: string }
export interface AppNotification { count?: number; capped?: boolean }
export interface DockApp {
  id: string;
  name: string;
  icon?: string;
  launchPath?: string;
  windows: AppWindow[];
  active: boolean;
  pinned?: boolean;
  notification?: AppNotification;
}

export const demoApps: DockApp[] = [
  { id: 'files', name: 'File Explorer' }, { id: 'browser', name: 'Browser' },
  { id: 'terminal', name: 'Terminal' }, { id: 'editor', name: 'Code Editor' },
  { id: 'music', name: 'Music' },
].map((app, index) => ({
  ...app, active: index === 1, notification: app.id === 'editor' ? { count: 3 } : undefined,
  windows: app.id === 'browser' ? [{ id: 'browser', title: 'Browser' }, { id: 'downloads', title: 'Downloads' }]
    : index < 4 ? [{ id: app.id, title: app.name }] : [],
}));

export function readSettings(): Promise<Settings> {
  if (desktop) return invoke<Settings>('read_settings');
  return Promise.resolve(browserSettings());
}

export function browserSettings(): Settings {
  try {
    const stored = readBrowserDock().settings ?? JSON.parse(localStorage.getItem(settingsStorageKey) ?? 'null') ?? {};
    return normalizeSettings(stored);
  } catch {
    return defaults;
  }
}

export function normalizeSettings(value: unknown): Settings {
  const stored = typeof value === 'object' && value ? value as Record<string, unknown> : {};
  const material = stored.material === 'midnight' || stored.material === 'clear' ? stored.material : 'tinted';
  const oldAccent = typeof stored.accent === 'string' && Object.hasOwn(legacyAccents, stored.accent)
    ? legacyAccents[stored.accent as keyof typeof legacyAccents] : defaults.accent;
  const option = <Key extends string>(choices: Record<Key, unknown>, saved: unknown, fallback: Key): Key =>
    typeof saved === 'string' && Object.hasOwn(choices, saved) ? saved as Key : fallback;
  const preferences = Object.fromEntries(Object.keys(defaults).filter((key) => Object.hasOwn(stored, key))
    .map((key) => [key, stored[key]]));
  const speed = stored.appIconClickSpeed;
  const padding = stored.padding;
  return {
    ...defaults, ...preferences, material,
    dockSnow: stored.dockSnow === true,
    hideInFullscreen: stored.hideInFullscreen !== false,
    padding: typeof padding === 'number' && Number.isFinite(padding)
      ? Math.min(dockPaddingRange.max, Math.max(dockPaddingRange.min, Math.round(padding)))
      : defaults.padding,
    accent: option(accents, stored.accent, oldAccent),
    tintColor: normalizeHexColor(stored.tintColor) ?? defaults.tintColor,
    appIconShape: option(appIconShapes, stored.appIconShape, defaults.appIconShape),
    appIconBackground: option(appIconBackgrounds, stored.appIconBackground, defaults.appIconBackground),
    appIconColor: normalizeHexColor(stored.appIconColor) ?? defaults.appIconColor,
    appIconClickEffect: option(appIconClickEffects, stored.appIconClickEffect, defaults.appIconClickEffect),
    appIconClickSpeed: typeof speed === 'number' && Number.isFinite(speed)
      ? Math.min(appIconClickSpeedRange.max, Math.max(appIconClickSpeedRange.min, Math.round(speed)))
      : defaults.appIconClickSpeed,
  };
}

export async function writeSettings(settings: Settings) {
  if (desktop) await invoke('write_settings', { settings });
  else writeBrowserDock({ settings });
}

export async function readAppAppearance(): Promise<AppAppearance> {
  const appAppearance = desktop
    ? await invoke<AppAppearance>('read_app_appearance')
    : readBrowserDock().appAppearance;
  if (appAppearance) {
    const { theme, tintColor, blurAmount, tintAmount } = appAppearance;
    const { min, max } = appAppearanceAmountRange;
    const amount = (value: unknown, fallback: number) =>
      typeof value === 'number' && Number.isFinite(value)
        ? Math.min(max, Math.max(min, Math.round(value)))
        : fallback;
    return {
      theme: theme === 'light' || theme === 'dark' ? theme : 'system',
      tintColor: normalizeHexColor(tintColor) ?? defaultAppAppearance.tintColor,
      blurAmount: amount(blurAmount, defaultAppAppearance.blurAmount),
      tintAmount: amount(tintAmount, defaultAppAppearance.tintAmount),
    };
  }

  const accent = browserSettings().accent;
  const appearance: AppAppearance = {
    ...defaultAppAppearance,
    theme: accent === 'light' || accent === 'dark' ? accent : 'system',
    tintColor: accents[accent].color,
  };
  writeBrowserDock({ appAppearance: appearance });
  return appearance;
}

export async function writeAppAppearance(appearance: AppAppearance) {
  if (desktop) await invoke('write_app_appearance', { appearance });
  else writeBrowserDock({ appAppearance: appearance });
}

export function subscribeSettings(callback: (settings: Settings) => void) {
  return listen<Settings>('settings-changed', ({ payload }) => callback(payload));
}
