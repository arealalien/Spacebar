export const widgetCatalog = {
  apps: { name: 'App icons', detail: 'Pinned and running apps', width: 0 },
  folder: { name: 'Folder', detail: 'Group apps and widgets', width: 0 },
  'widget-group': { name: 'Widget group', detail: 'Keep widgets together on the dock with adjustable spacing and padding', width: 128 },
  'widget-tray': { name: 'Widget tray', detail: 'Fit more widgets into a compact strip on the dock', width: 105 },
  start: { name: 'Windows Start', detail: 'Open the native Windows Start menu', width: 0 },
  'spacebar-start': { name: 'Spacebar Start', detail: 'Search apps and access power options in a dock menu', width: 0 },
  spacer: { name: 'Spacer', detail: 'Add an adjustable gap between widgets', width: 74 },
  divider: { name: 'Divider', detail: 'Separate dock sections with a line', width: 74 },
  clock: { name: 'Clock', detail: 'Local time and date', width: 116 },
  'clock-plain': { name: 'Digital clock', detail: 'Time with optional background and outline', width: 96 },
  'clock-analog': { name: 'Analog clock', detail: 'Local time on an analog dial', width: 0 },
  'clock-twilight': { name: 'Twilight clock', detail: 'Analog time with a warm second hand', width: 0 },
  'clock-flip': { name: 'Flip clock', detail: 'Split-flap hours and minutes', width: 112 },
  'day-progress': { name: 'Day progress', detail: 'A dot for each hour of the day', width: 126 },
  'quick-note': { name: 'Quick Note', detail: 'A scratchpad that saves as you type', width: 0 },
  'drawing-pad': { name: 'Drawing Pad', detail: 'Sketch with a brush and eraser', width: 0 },
  image: { name: 'Image / GIF', detail: 'Crop a photo or animation to fit your dock', width: 0 },
  agenda: { name: 'Agenda', detail: 'See and manage upcoming events', width: 156 },
  'quick-tasks': { name: 'Quick Tasks', detail: 'Keep a checklist and see your next task on the dock', width: 150 },
  'color-picker': { name: 'Color Picker', detail: 'Pick a screen color and copy hex values from your history', width: 0 },
  weather: { name: 'Weather · Clean', detail: 'Current conditions and an understated hourly forecast on hover', width: 118 },
  'weather-colorful': { name: 'Weather · Colorful', detail: 'A weather icon with a frosted seven-day outlook on hover', width: 0 },
  'weather-3d': { name: 'Weather · 3D', detail: 'A square tile with sculpted weather icons and an hourly list on hover', width: 0 },
  'weather-ambient': { name: 'Weather · Ambient', detail: 'Follow the local sun and moon, with current weather on hover', width: 126 },
  spotify: { name: 'Spotify artwork', detail: 'Album art behind the playback controls', width: 196 },
  'spotify-card': { name: 'Spotify track', detail: 'Track, artist, artwork, and playback buttons', width: 182 },
  'spotify-mini': { name: 'Spotify mini', detail: 'Album art with play and pause on hover', width: 0 },
  'spotify-text': { name: 'Spotify text', detail: 'Track and artist with playback controls on hover', width: 158 },
  volume: { name: 'Audio output', detail: 'Windows speaker volume and mute', width: 168 },
  microphone: { name: 'Microphone', detail: 'Default microphone input level and mute', width: 168 },
  mixer: { name: 'Volume mixer', detail: 'Choose the output device and adjust its volume', width: 0 },
  'audio-visualizer': { name: 'Audio Visualizer', detail: 'Watch bass, mids, and treble react to Windows audio', width: 120 },
  cpu: { name: 'CPU', detail: 'Live processor usage', width: 0 },
  ram: { name: 'RAM', detail: 'Live physical memory usage', width: 0 },
  gpu: { name: 'GPU', detail: 'Live graphics engine usage when reported by Windows', width: 0 },
  performance: { name: 'System load', detail: 'CPU, RAM, and GPU usage together', width: 143 },
  'system-rings': { name: 'System rings', detail: 'Side-by-side CPU and memory gauges', width: 118 },
  network: { name: 'Network', detail: 'Download and upload rates, history, and session usage', width: 203 },
  'network-usage': { name: 'Network usage', detail: 'Total data sent and received while monitoring', width: 171 },
  storage: { name: 'Storage', detail: 'Free space on the Windows system drive', width: 145 },
  tray: { name: 'System tray', detail: 'Live Windows tray icons in a Spacebar panel', width: 0 },
  'show-desktop': { name: 'Show desktop', detail: 'Hide or restore desktop windows', width: 0 },
  notifications: { name: 'Notifications', detail: 'Calendar and access to Windows notifications', width: 0 },
  'quick-settings': { name: 'Quick settings', detail: 'Audio controls and shortcuts to Wi-Fi, Bluetooth, and display', width: 0 },
  keyboard: { name: 'On-screen keyboard', detail: 'Toggle the Windows on-screen keyboard', width: 0 },
  display: { name: 'Display settings', detail: 'Open Windows display and brightness settings', width: 0 },
  'notification-settings': { name: 'Notification settings', detail: 'Open Windows settings for Do Not Disturb and notifications', width: 0 },
  capture: { name: 'Screenshots', detail: 'Region capture, full-screen save, and Snipping Tool', width: 145 },
};

export type WidgetKind = keyof typeof widgetCatalog;
export const clockKinds = ['clock', 'clock-plain', 'clock-analog', 'clock-twilight', 'clock-flip', 'day-progress'] as const;
export const spotifyKinds = ['spotify', 'spotify-card', 'spotify-mini', 'spotify-text'] as const;
export const weatherKinds = ['weather', 'weather-colorful', 'weather-3d', 'weather-ambient'] as const;
export type ClockKind = typeof clockKinds[number];
export type SpotifyKind = typeof spotifyKinds[number];
export type WeatherKind = typeof weatherKinds[number];
export const isClockKind = (kind: string): kind is ClockKind => clockKinds.some((clock) => clock === kind);
export const isSpotifyKind = (kind: string): kind is SpotifyKind => spotifyKinds.some((spotify) => spotify === kind);
export const isWeatherKind = (kind: string): kind is WeatherKind => weatherKinds.some((weather) => weather === kind);
export const isLayoutWidget = (kind: string) => kind === 'spacer' || kind === 'divider';
export const isStartWidget = (kind: string): kind is 'start' | 'spacebar-start' => kind === 'start' || kind === 'spacebar-start';
export const isWidgetContainer = (kind: string): kind is 'widget-group' | 'widget-tray' => kind === 'widget-group' || kind === 'widget-tray';
export const repeatableWidget = (kind: WidgetKind) => kind === 'folder' || kind === 'clock-plain' || kind === 'image' || isLayoutWidget(kind) || isWidgetContainer(kind);

export const widgetNames = Object.fromEntries(Object.entries(widgetCatalog).map(([kind, widget]) => [kind, widget.name])) as Record<WidgetKind, string>;
