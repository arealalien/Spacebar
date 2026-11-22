import { AppWindow, CodeXml, Compass, Folder, Music2, Terminal } from 'lucide-react';
import type { CSSProperties } from 'react';
import type { AppNotification, DockApp, Settings } from './model';

const demoIcons = { files: Folder, browser: Compass, terminal: Terminal, editor: CodeXml, music: Music2 };

export function AppIcon({ app }: { app: Pick<DockApp, 'id' | 'icon'> }) {
  if (app.icon) return <img className="app-icon" src={app.icon} alt="" draggable={false} />;
  const Icon = demoIcons[app.id as keyof typeof demoIcons];
  return Icon
    ? <span className={`sample-icon ${app.id}`}><Icon size="61%" strokeWidth={1.9} /></span>
    : <span className="sample-icon fallback"><AppWindow size="61%" /></span>;
}

export function appIconStyle(settings: Settings): CSSProperties {
  const backgrounds = {
    transparent: 'transparent',
    tinted: 'color-mix(in oklab, var(--dock-tone) 42%, #17181c)',
    black: '#000',
    white: '#fff',
    custom: `color-mix(in oklab, ${settings.appIconColor} 22%, #292a31)`,
  };
  const background = settings.appIconBackground !== 'transparent';
  const shape = settings.appIconShape;
  return {
    '--app-icon-background': backgrounds[settings.appIconBackground],
    '--app-icon-radius': shape === 'circle' ? '50%' : shape === 'rounded' || background ? '24%' : '0',
    '--app-icon-clip': shape === 'original' && !background ? 'visible' : 'hidden',
    '--app-icon-art-size': background ? '74%' : '100%',
  } as CSSProperties;
}

export function DockAppIcon({ app }: { app: Pick<DockApp, 'id' | 'icon'> }) {
  return <span className="dock-app-image" aria-hidden="true"><AppIcon app={app} /></span>;
}

export function notificationLabel(name: string, notification: AppNotification | undefined, mode: Settings['notificationBadges']) {
  if (!notification || mode === 'none') return name;
  const { count, capped } = notification;
  return count ? `${name}, ${count}${capped ? '+' : ''} unread notification${count === 1 && !capped ? '' : 's'}`
    : `${name}, unread notifications`;
}

export function NotificationBadge({ notification, mode }: { notification?: AppNotification; mode: Settings['notificationBadges'] }) {
  if (!notification || mode === 'none') return null;
  const { count, capped } = notification;
  const label = mode === 'count' && count ? (count > 99 ? '99+' : `${count}${capped ? '+' : ''}`) : '';
  return <span className="app-notification-badge" data-numbered={!!label} aria-hidden="true">{label}</span>;
}
