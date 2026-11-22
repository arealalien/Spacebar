export type SkyPhase = 'day' | 'dawn' | 'dusk' | 'night';

export interface SkyReading {
  phase: SkyPhase;
  marker?: { body: 'sun' | 'moon'; progress: number };
}

const radians = Math.PI / 180;
const axialTilt = 23.4397 * radians;
// Apparent rise and set thresholds for geocentric coordinates.
// https://aa.usno.navy.mil/faq/RST_defs
const sunHorizon = -0.8333 * radians;
const moonHorizon = 0.125 * radians;

function skyPosition(siderealAngle: number, latitude: number, eclipticLongitude: number, eclipticLatitude: number) {
  const rightAscension = Math.atan2(
    Math.sin(eclipticLongitude) * Math.cos(axialTilt) - Math.tan(eclipticLatitude) * Math.sin(axialTilt),
    Math.cos(eclipticLongitude),
  );
  const declination = Math.asin(
    Math.sin(eclipticLatitude) * Math.cos(axialTilt) + Math.cos(eclipticLatitude) * Math.sin(axialTilt) * Math.sin(eclipticLongitude),
  );
  const angle = siderealAngle - rightAscension;
  const hourAngle = Math.atan2(Math.sin(angle), Math.cos(angle));
  const offset = Math.sin(latitude) * Math.sin(declination);
  const amplitude = Math.cos(latitude) * Math.cos(declination);
  const sineAltitude = offset + amplitude * Math.cos(hourAngle);
  return { hourAngle, offset, amplitude, sineAltitude };
}

function arcProgress(position: ReturnType<typeof skyPosition>, horizon: number) {
  // Estimate the span of the visible arc at the current declination. Clamping
  // also gives a full-day arc when the body stays above the polar horizon.
  const edge = (Math.sin(horizon) - position.offset) / position.amplitude;
  const halfArc = Math.acos(Math.max(-1, Math.min(1, edge)));
  if (halfArc <= 0) return 0.5;
  return Math.max(0, Math.min(1, (position.hourAngle + halfArc) / (2 * halfArc)));
}

export function readAmbientSky(time: number, latitude: number, longitude: number): SkyReading {
  // Low-precision solar and lunar orbital elements referenced to J2000.
  // https://github.com/mourner/suncalc/blob/v1.9.0/suncalc.js
  const days = (time - Date.UTC(2000, 0, 1, 12)) / 86_400_000;
  const siderealAngle = (280.16 + 360.9856235 * days + longitude) * radians;
  const observerLatitude = latitude * radians;
  const solarAnomaly = (357.5291 + 0.98560028 * days) * radians;
  const solarCorrection = 1.9148 * Math.sin(solarAnomaly) + 0.02 * Math.sin(2 * solarAnomaly) + 0.0003 * Math.sin(3 * solarAnomaly);
  const sun = skyPosition(siderealAngle, observerLatitude, solarAnomaly + (solarCorrection + 282.9372) * radians, 0);
  const solarAltitude = Math.asin(Math.max(-1, Math.min(1, sun.sineAltitude))) / radians;
  let phase: SkyPhase = 'day';
  if (solarAltitude < -6) phase = 'night';
  else if (solarAltitude < 6) phase = sun.hourAngle < 0 ? 'dawn' : 'dusk';

  if (sun.sineAltitude > Math.sin(sunHorizon)) {
    return { phase, marker: { body: 'sun', progress: arcProgress(sun, sunHorizon) } };
  }

  const lunarAnomaly = (134.963 + 13.064993 * days) * radians;
  const lunarArgument = (93.272 + 13.229350 * days) * radians;
  const lunarLongitude = (218.316 + 13.176396 * days + 6.289 * Math.sin(lunarAnomaly)) * radians;
  const moon = skyPosition(siderealAngle, observerLatitude, lunarLongitude, 5.128 * Math.sin(lunarArgument) * radians);
  const marker = moon.sineAltitude > Math.sin(moonHorizon)
    ? { body: 'moon' as const, progress: arcProgress(moon, moonHorizon) }
    : undefined;
  return { phase, marker };
}
