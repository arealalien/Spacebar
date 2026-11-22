import { useEffect, useState } from 'react';
import type { WeatherLocation } from './widgetContent';

export interface Forecast {
  timezone: string;
  current: {
    time: number; temperature_2m: number; weather_code: number; is_day: number;
    relative_humidity_2m?: number | null; wind_speed_10m?: number | null;
  };
  hourly: { time: number[]; temperature_2m: number[]; weather_code: number[]; is_day: number[]; precipitation_probability: number[] };
  daily: {
    time?: number[];
    temperature_2m_min: (number | null)[];
    temperature_2m_max: (number | null)[];
    weather_code?: (number | null)[];
    sunrise?: (number | null)[];
    sunset?: (number | null)[];
    daylight_duration?: (number | null)[];
    precipitation_probability_max?: (number | null)[];
  };
}

interface ForecastState {
  forecast?: Forecast;
  failed?: boolean;
}

const freshness = 15 * 60_000;
const cache = new Map<string, { forecast: Forecast; received: number }>();
const requests = new Map<string, Promise<Forecast>>();

function loadForecast(key: string, latitude: number, longitude: number) {
  const pending = requests.get(key);
  if (pending) return pending;
  const params = new URLSearchParams({
    latitude: String(latitude), longitude: String(longitude),
    current: 'temperature_2m,weather_code,is_day,relative_humidity_2m,wind_speed_10m',
    hourly: 'temperature_2m,weather_code,is_day,precipitation_probability',
    daily: 'temperature_2m_min,temperature_2m_max,weather_code,sunrise,sunset,daylight_duration,precipitation_probability_max',
    timezone: 'auto', timeformat: 'unixtime', forecast_days: '7',
  });
  const request = fetch('https://api.open-meteo.com/v1/forecast?' + params, { signal: AbortSignal.timeout(12_000) })
    .then(async (response) => {
      if (!response.ok) throw new Error('Weather is unavailable.');
      const forecast: Forecast = await response.json();
      if (!Number.isFinite(forecast.current?.time) || !Array.isArray(forecast.hourly?.time) || !forecast.daily) {
        throw new Error('The forecast is incomplete.');
      }
      if (cache.size >= 12 && !cache.has(key)) cache.delete(cache.keys().next().value!);
      cache.set(key, { forecast, received: Date.now() });
      return forecast;
    }).finally(() => requests.delete(key));
  requests.set(key, request);
  return request;
}

export function useForecast(location?: WeatherLocation | null, enabled = true) {
  const latitude = location?.latitude;
  const longitude = location?.longitude;
  const key = enabled && latitude != null && longitude != null ? `${latitude},${longitude}` : undefined;
  const [state, setState] = useState<ForecastState & { key: string }>();
  const [revision, setRevision] = useState(0);

  useEffect(() => {
    if (!key || latitude == null || longitude == null) return;
    let active = true;
    const currentKey = key;
    const coordinates = { latitude, longitude };
    let timer = 0;
    async function refresh() {
      const saved = cache.get(currentKey);
      const age = saved ? Date.now() - saved.received : freshness;
      if (saved && age < freshness) {
        setState({ key: currentKey, forecast: saved.forecast });
        timer = window.setTimeout(refresh, Math.max(1000, freshness - age));
        return;
      }
      setState({ key: currentKey, forecast: saved?.forecast });
      let delay = freshness;
      try {
        const forecast = await loadForecast(currentKey, coordinates.latitude, coordinates.longitude);
        if (active) setState({ key: currentKey, forecast });
      } catch {
        if (active) setState({ key: currentKey, forecast: saved?.forecast, failed: true });
        delay = 60_000;
      }
      if (active) timer = window.setTimeout(refresh, delay);
    }
    void refresh();
    return () => { active = false; window.clearTimeout(timer); };
  }, [key, latitude, longitude, revision]);

  const matching = state?.key === key ? state : undefined;
  return { forecast: matching?.forecast ?? (key ? cache.get(key)?.forecast : undefined), failed: matching?.failed,
    retry: () => setRevision((value) => value + 1) };
}

export async function findWeatherLocations(query: string, signal: AbortSignal): Promise<WeatherLocation[]> {
  const parameters = new URLSearchParams({ name: query, count: '6', language: navigator.language.split('-')[0], format: 'json' });
  const response = await fetch('https://geocoding-api.open-meteo.com/v1/search?' + parameters,
    { signal: AbortSignal.any([signal, AbortSignal.timeout(12_000)]) });
  if (!response.ok) throw new Error('City search is unavailable.');
  const { results = [] }: { results?: (WeatherLocation & { admin1?: string; country?: string })[] } = await response.json();
  return results.filter((item) => item.name && Number.isFinite(item.latitude) && Number.isFinite(item.longitude))
    .map(({ name, admin1, country, latitude, longitude }) => ({
      name, latitude, longitude, detail: [...new Set([admin1, country].filter((part) => part && part !== name))].join(', '),
    }));
}

export function degrees(celsius: number | null | undefined, fahrenheit: boolean) {
  if (celsius == null || !Number.isFinite(celsius)) return '—';
  return Math.round(fahrenheit ? celsius * 9 / 5 + 32 : celsius) + '°';
}
