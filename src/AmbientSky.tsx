import { useEffect, useState, type CSSProperties } from 'react';
import { readAmbientSky, type SkyReading } from './ambientOrbit';
import type { WeatherLocation } from './widgetContent';

const circleCap = 0.35;
const capAngle = Math.acos(1 - 2 * circleCap);
const skyStyle = { '--ambient-cap': circleCap } as CSSProperties;
const sample: SkyReading = { phase: 'dusk', marker: { body: 'sun', progress: 0.72 } };

export function AmbientSky({ location, specimen = false }: { location?: WeatherLocation | null; specimen?: boolean }) {
  const [time, setTime] = useState(Date.now);
  const latitude = location?.latitude;
  const longitude = location?.longitude;

  useEffect(() => {
    if (latitude == null || longitude == null || specimen) return;
    setTime(Date.now());
    const timer = window.setInterval(() => setTime(Date.now()), 60_000);
    return () => window.clearInterval(timer);
  }, [latitude, longitude, specimen]);

  const reading = specimen ? sample : latitude == null || longitude == null
    ? undefined : readAmbientSky(time, latitude, longitude);
  const marker = reading?.marker;
  const angle = ((marker?.progress ?? 0.5) * 2 - 1) * capAngle;
  const position = {
    '--ambient-marker-x': Math.sin(angle),
    '--ambient-marker-y': 1 - Math.cos(angle),
  } as CSSProperties;
  return <span className="ambient-sky" data-phase={reading?.phase ?? 'unset'} style={skyStyle} aria-hidden="true">
    <span className="ambient-orbit" />
    {marker && <span key={marker.body} className="ambient-orb" data-body={marker.body} style={position} />}
  </span>;
}
