import { Cloud, CloudDrizzle, CloudFog, CloudLightning, CloudMoon, CloudRain, CloudSnow, CloudSun, Droplets, Equal, MapPin, Moon, Search, Snowflake, Sun, Sunrise, Sunset, Zap, type LucideIcon } from 'lucide-react';
import { useEffect, useRef, useState, type ComponentPropsWithRef } from 'react';
import { AmbientSky } from './AmbientSky';
import { useHoverPopup } from './HoverPopup';
import { useSurfacePayload } from './useSurfacePayload';
import { widgetNames, type WeatherKind } from './widgetCatalog';
import { degrees, findWeatherLocations, useForecast, type Forecast } from './weather';
import { useWidgetContent, writeWidgetContent, type WeatherLocation, type WeatherPreferences } from './widgetContent';

const weatherDesigns = {
  weather: 'clean', 'weather-colorful': 'colorful', 'weather-3d': '3d', 'weather-ambient': 'ambient',
} as const;
type WeatherDesign = typeof weatherDesigns[WeatherKind];
const sculptures = {
  sunny: '0% 0%', 'clear-night': '50% 0%', 'partly-cloudy': '100% 0%',
  'cloudy-night': '0% 50%', cloudy: '50% 50%', rain: '100% 50%',
  storm: '0% 100%', snow: '50% 100%', fog: '100% 100%',
};
type WeatherCondition = { Icon: LucideIcon; label: string; sky: keyof typeof sculptures };

function conditions(code?: number | null, day = 1): WeatherCondition {
  if (code == null) return { Icon: CloudSun, label: 'Weather', sky: 'partly-cloudy' };
  if (code === 0) return day
    ? { Icon: Sun, label: 'Sunny', sky: 'sunny' }
    : { Icon: Moon, label: 'Clear', sky: 'clear-night' };
  if (code <= 2) return { Icon: day ? CloudSun : CloudMoon, label: code === 1 ? 'Mostly clear' : 'Partly cloudy', sky: day ? 'partly-cloudy' : 'cloudy-night' };
  if (code === 3) return { Icon: Cloud, label: 'Cloudy', sky: 'cloudy' };
  if (code <= 48) return { Icon: CloudFog, label: 'Fog', sky: 'fog' };
  if (code <= 57) return { Icon: CloudDrizzle, label: 'Drizzle', sky: 'rain' };
  if (code <= 67) return { Icon: CloudRain, label: 'Rain', sky: 'rain' };
  if (code <= 77) return { Icon: CloudSnow, label: 'Snow', sky: 'snow' };
  if (code <= 82) return { Icon: CloudRain, label: 'Showers', sky: 'rain' };
  if (code <= 86) return { Icon: CloudSnow, label: 'Snow showers', sky: 'snow' };
  return { Icon: CloudLightning, label: 'Thunderstorms', sky: 'storm' };
}

const weatherDetails: Partial<Record<WeatherCondition['sky'], LucideIcon>> = {
  rain: Droplets, snow: Snowflake, storm: Zap, fog: Equal,
};

function WeatherSymbol({ condition: { Icon, label, sky }, design, decorative = true }: {
  condition: WeatherCondition; design: WeatherDesign; decorative?: boolean;
}) {
  const a11y = decorative ? { 'aria-hidden': true as const } : { role: 'img', 'aria-label': label };
  if (design === 'colorful') {
    const Detail = weatherDetails[sky];
    return <span className="colorful-weather-icon" data-sky={sky} {...a11y}>
      {(sky === 'sunny' || sky === 'partly-cloudy') && <span className="colorful-sun" />}
      {(sky === 'clear-night' || sky === 'cloudy-night') && <Moon className="colorful-moon" fill="currentColor" />}
      {sky !== 'sunny' && sky !== 'clear-night' && <>
        <Cloud className="colorful-cloud-back" fill="currentColor" strokeWidth={0} />
        <Cloud className="colorful-cloud-front" fill="currentColor" strokeWidth={0} />
      </>}
      {Detail && <Detail className="colorful-icon-detail" strokeWidth={2.4} />}
    </span>;
  }
  return design === '3d'
    ? <span className="weather-sculpture" data-sky={sky} style={{ backgroundPosition: sculptures[sky] }} {...a11y} />
    : <Icon className="weather-symbol" data-sky={sky} {...a11y} />;
}

export function WeatherWidget({ kind, widget, specimen, onError }: {
  kind: WeatherKind; widget?: string; specimen: boolean; onError?: (error: unknown) => void;
}) {
  const face = useRef<HTMLButtonElement>(null);
  const timer = useRef<number>(undefined);
  const preferences = useSurfacePayload<WeatherPreferences>('weather-preferences-changed', 'read_weather_preferences', !specimen);
  const { forecast } = useForecast(preferences?.location, !specimen);
  const open = useHoverPopup(face, specimen ? undefined : widget, kind, onError);
  useEffect(() => () => window.clearTimeout(timer.current), []);

  const design = weatherDesigns[kind];
  const daylight = forecast?.current.is_day ?? 1;
  const condition = conditions(specimen ? (design === 'colorful' ? 0 : 2) : forecast?.current.weather_code, daylight);
  const city = preferences?.location;
  const hasReading = specimen || !!city;
  const unit = preferences?.fahrenheit ? 'F' : 'C';
  const temperature = specimen ? '16°' : degrees(forecast?.current.temperature_2m, unit === 'F');
  const summary = city ? `${city.name}${forecast ? `, ${temperature}${unit}, ${condition.label}` : ''}` : 'Set location';
  const reading = design === '3d'
    ? hasReading && <strong className="weather-tile-temperature">{temperature}</strong>
    : <span className="weather-face-reading">
      <strong className={hasReading ? undefined : 'weather-face-label'}>{hasReading ? temperature : 'Weather'}</strong>
      <small>{specimen ? (design === 'clean' ? 'Weather' : condition.label) : city?.name ?? 'Set location'}</small>
    </span>;
  const symbol = <WeatherSymbol condition={condition} design={design} />;
  const contents = design === 'ambient' ? <AmbientSky location={city} specimen={specimen} />
    : design === 'colorful' ? symbol : <>{symbol}{reading}</>;
  const appearance = { 'data-weather-design': design, 'data-sky': condition.sky, 'data-daylight': daylight === 0 ? 'night' : 'day' };
  if (specimen) return <span className="widget-face weather-face" {...appearance}>{contents}</span>;
  return <button ref={face} type="button" className="widget-face weather-face" {...appearance} aria-haspopup="dialog"
    aria-label={`${widgetNames[kind]}, ${summary}`}
    onPointerEnter={(event) => {
      if (event.pointerType === 'touch') return;
      const element = event.currentTarget;
      window.clearTimeout(timer.current);
      timer.current = window.setTimeout(() => open(element), 130);
    }} onPointerLeave={() => window.clearTimeout(timer.current)}
    onClick={(event) => {
      window.clearTimeout(timer.current);
      const touch = event.nativeEvent instanceof PointerEvent && event.nativeEvent.pointerType === 'touch';
      open(event.currentTarget, true, touch || event.detail === 0);
    }}>
    {contents}
  </button>;
}

function LocationSearch({ focus, saving, onPick, onCancel }: {
  focus: boolean; saving: boolean; onPick: (location: WeatherLocation) => void; onCancel?: () => void;
}) {
  const [query, setQuery] = useState('');
  const [matches, setMatches] = useState<WeatherLocation[]>();
  const [status, setStatus] = useState('');
  const search = useRef<AbortController>(undefined);
  useEffect(() => () => search.current?.abort(), []);

  async function lookup() {
    const name = query.trim();
    if (name.length < 2) return;
    search.current?.abort();
    const controller = new AbortController();
    search.current = controller;
    setMatches(undefined);
    setStatus('Searching…');
    try {
      const cities = await findWeatherLocations(name, controller.signal);
      if (controller.signal.aborted) return;
      setMatches(cities);
      setStatus(cities.length ? '' : 'No places found. Try a nearby city.');
    } catch {
      if (!controller.signal.aborted) setStatus('Couldn’t search for cities. Try again.');
    }
  }

  return <div className="weather-search">
    <form className="utility-form" onSubmit={(event) => { event.preventDefault(); void lookup(); }}>
      <input type="search" placeholder="City or town" aria-label="City or town" autoFocus={focus}
        value={query} maxLength={100} onChange={(event) => {
          search.current?.abort();
          setMatches(undefined);
          setStatus('');
          setQuery(event.currentTarget.value);
        }} />
      <button type="submit" className="utility-icon-button" aria-label="Search for a city" title="Search"
        disabled={query.trim().length < 2 || status === 'Searching…'}><Search size={17} aria-hidden="true" /></button>
    </form>
    {status && <p className="utility-message" role="status">{status}</p>}
    {!!matches?.length && <ul className="utility-list weather-results" aria-label="Matching places">
      {matches.map((location) => <li key={`${location.latitude},${location.longitude}`}>
        <button type="button" disabled={saving} onClick={() => onPick(location)}>
          <MapPin size={16} aria-hidden="true" />
          <span><strong>{location.name}</strong>{location.detail && <small>{location.detail}</small>}</span>
        </button>
      </li>)}
    </ul>}
    {onCancel && <button type="button" className="weather-text-button" onClick={onCancel}>Cancel</button>}
  </div>;
}

type ClassicWeatherDesign = 'clean' | '3d';

function HourlyForecast({ forecast, fahrenheit, design }: { forecast: Forecast; fahrenheit: boolean; design: ClassicWeatherDesign }) {
  const hourly = forecast.hourly;
  const formatter = new Intl.DateTimeFormat(undefined, { hour: 'numeric', timeZone: forecast.timezone });
  const upcoming = hourly.time.map((time, index) => ({ time, index })).filter(({ time }) => time > forecast.current.time).slice(0, 5);
  if (!upcoming.length) return null;
  return <section className="weather-hourly" data-weather-design={design} aria-labelledby="weather-hours">
    <h2 id="weather-hours" className="utility-section-title">Next few hours</h2>
    <ul>
      {upcoming.map(({ time, index }) => {
        const daylight = hourly.is_day?.[index];
        const condition = conditions(hourly.weather_code?.[index], daylight);
        const precipitation = hourly.precipitation_probability?.[index];
        return <li key={time}>
          <time dateTime={new Date(time * 1000).toISOString()}>{formatter.format(time * 1000)}</time>
          <WeatherSymbol condition={condition} design={design} decorative={design === '3d'} />
          {design === '3d' && <span className="weather-hour-condition">{condition.label}</span>}
          <strong>{degrees(hourly.temperature_2m?.[index], fahrenheit)}</strong>
          {precipitation > 0 && <small className="weather-rain" aria-label={`${precipitation}% chance of precipitation`}>
            <Droplets size={11} aria-hidden="true" />{precipitation}%
          </small>}
        </li>;
      })}
    </ul>
  </section>;
}

function CurrentWeather({ forecast, fahrenheit, design, condition }: {
  forecast: Forecast; fahrenheit: boolean; design: ClassicWeatherDesign; condition: WeatherCondition;
}) {
  const temperature = <div className="weather-temperature">{degrees(forecast.current.temperature_2m, fahrenheit)}<small>{fahrenheit ? 'F' : 'C'}</small></div>;
  const range = <small>High {degrees(forecast.daily.temperature_2m_max?.[0], fahrenheit)} · Low {degrees(forecast.daily.temperature_2m_min?.[0], fahrenheit)}</small>;
  const symbol = <WeatherSymbol condition={condition} design={design} />;
  if (design === 'clean') return <div className="weather-current">
    {temperature}
    <div className="weather-current-info">{symbol}<strong>{condition.label}</strong>{range}</div>
  </div>;

  const reading = <div className="weather-hero-reading">{temperature}<strong>{condition.label}</strong>{range}</div>;
  return <div className="weather-hero" data-weather-design={design}>{symbol}{reading}</div>;
}

function hasReading(value: number | null | undefined): value is number {
  return value != null && Number.isFinite(value);
}

function DaylightBand({ forecast, day, clock }: { forecast: Forecast; day: number; clock: Intl.DateTimeFormat }) {
  const sunrise = forecast.daily.sunrise?.[day];
  const sunset = forecast.daily.sunset?.[day];
  const duration = forecast.daily.daylight_duration?.[day];
  if (!hasReading(sunrise) && !hasReading(sunset) && !hasReading(duration)) return null;
  const minutes = hasReading(duration) ? Math.round(duration / 60) : undefined;

  return <div className="colorful-daylight" role="group" aria-label="Daylight">
    {hasReading(sunrise) && <span><Sunrise size={19} aria-hidden="true" />
      <time dateTime={new Date(sunrise * 1000).toISOString()} aria-label={`Sunrise at ${clock.format(sunrise * 1000)}`}>{clock.format(sunrise * 1000)}</time>
    </span>}
    {minutes !== undefined && <span className="colorful-daylight-duration" aria-label={`${minutes} minutes of daylight`}>
      {Math.floor(minutes / 60)} h {minutes % 60} m
    </span>}
    {hasReading(sunset) && <span><time dateTime={new Date(sunset * 1000).toISOString()}
      aria-label={`Sunset at ${clock.format(sunset * 1000)}`}>{clock.format(sunset * 1000)}</time><Sunset size={19} aria-hidden="true" />
    </span>}
  </div>;
}

function ColorfulWeather({ forecast, condition, location, fahrenheit, saving, onLocation, onUnits }: {
  forecast: Forecast; condition: WeatherCondition; location: WeatherLocation; fahrenheit: boolean;
  saving: boolean; onLocation: () => void; onUnits: () => void;
}) {
  const { current, daily, timezone: timeZone } = forecast;
  const clock = new Intl.DateTimeFormat(undefined, { hour: 'numeric', minute: '2-digit', timeZone });
  const weekday = new Intl.DateTimeFormat(undefined, { weekday: 'short', timeZone });
  const date = new Intl.DateTimeFormat(undefined, { weekday: 'long', hour: 'numeric', minute: '2-digit', timeZone });
  const dateKey = new Intl.DateTimeFormat('en-CA', { year: 'numeric', month: '2-digit', day: '2-digit', timeZone });
  const today = dateKey.format(current.time * 1000);
  const days = (daily.time ?? []).flatMap((time, index) => hasReading(time)
    ? [{ time, index, today: dateKey.format(time * 1000) === today }] : []);
  const todayIndex = days.find((entry) => entry.today)?.index ?? 0;
  const chance = daily.precipitation_probability_max?.[todayIndex];
  const { relative_humidity_2m: humidity, wind_speed_10m: wind } = current;
  const place = [location.name, location.detail].filter(Boolean).join(', ');
  const temperature = degrees(current.temperature_2m, fahrenheit);
  const units = fahrenheit ? 'F' : 'C';
  const switchUnits = `Switch to ${fahrenheit ? 'Celsius' : 'Fahrenheit'}`;

  return <>
    <section className="colorful-current" aria-label="Current conditions">
      <div className="colorful-current-reading">
        <WeatherSymbol condition={condition} design="colorful" />
        <div>
          <time dateTime={new Date(current.time * 1000).toISOString()}>{date.format(current.time * 1000)}</time>
          <h1>{condition.label} <button type="button" className="colorful-temperature" title={switchUnits} disabled={saving}
            aria-label={`${temperature}${units}. ${switchUnits}`} onClick={onUnits}>{temperature}<small>{units}</small></button></h1>
          <button type="button" className="colorful-location" title={place}
            aria-label={`Change weather location, ${place}`} onClick={onLocation}>{place}</button>
        </div>
      </div>
      <DaylightBand forecast={forecast} day={todayIndex} clock={clock} />
    </section>
    {hasReading(chance) && <div className="colorful-precipitation">
      <Droplets size={17} aria-hidden="true" /><span>Precipitation today</span><strong>{Math.round(chance)}%</strong>
    </div>}
    {(hasReading(humidity) || hasReading(wind)) && <dl className="colorful-weather-stats">
      {hasReading(humidity) && <div><dt>Humidity</dt><dd>{Math.round(humidity)}%</dd></div>}
      {hasReading(wind) && <div><dt>Wind</dt><dd>{Math.round(wind)} km/h</dd></div>}
    </dl>}
    {!!days.length && <ol className="colorful-week" aria-label="Daily forecast">
      {days.map(({ time, index, today: isToday }) => {
        const high = degrees(daily.temperature_2m_max?.[index], fahrenheit);
        const low = degrees(daily.temperature_2m_min?.[index], fahrenheit);
        return <li key={time} aria-current={isToday ? 'date' : undefined}>
          <time dateTime={new Date(time * 1000).toISOString()}>{isToday ? 'Today' : weekday.format(time * 1000)}</time>
          <WeatherSymbol condition={conditions(daily.weather_code?.[index])} design="colorful" decorative={false} />
          <strong aria-label={`High ${high}`}>{high}</strong>
          <small aria-label={`Low ${low}`}>{low}</small>
        </li>;
      })}
    </ol>}
  </>;
}

export function WeatherPanel({ kind, frame, keyboard, error, onError }: {
  kind: WeatherKind; frame: ComponentPropsWithRef<'main'>; keyboard: boolean; error: string;
  onError: (error: unknown) => void;
}) {
  const { value, setValue, loading } = useWidgetContent('weather', onError);
  const [editing, setEditing] = useState(false);
  const [saving, setSaving] = useState(false);
  const location = value?.location;
  const { forecast, failed, retry } = useForecast(location);
  const fahrenheit = !!value?.fahrenheit;

  async function save(next: WeatherPreferences) {
    if (saving) return;
    setSaving(true);
    onError('');
    try {
      await writeWidgetContent('weather', next);
      setValue(next);
      setEditing(false);
    } catch (error) {
      onError(error);
    } finally { setSaving(false); }
  }

  const selecting = value && (editing || !location);
  const design = weatherDesigns[kind];
  const current = forecast?.current;
  const condition = conditions(current?.weather_code, current?.is_day);
  const panel = {
    ...frame,
    className: 'system-panel utility-panel weather-popup widget-popup-motion',
    role: 'dialog', 'aria-label': widgetNames[kind],
    'data-weather-design': design, 'data-sky': condition.sky, 'data-daylight': current?.is_day === 0 ? 'night' : 'day',
  };
  const alert = error && <p className="system-panel-error" role="alert" title={error}>{error}</p>;
  const retryNotice = failed && <div className="weather-retry">
    <p className="utility-message" role="status">{forecast ? 'Couldn’t refresh the forecast. Showing the last update.' : 'Couldn’t load the forecast.'}</p>
    <button type="button" className="weather-text-button" onClick={retry}>Retry</button>
  </div>;

  if (design === 'ambient' && location && !editing) {
    const temperature = degrees(current?.temperature_2m, fahrenheit);
    const switchUnits = `Switch to ${fahrenheit ? 'Celsius' : 'Fahrenheit'}`;
    return <main {...panel} data-ambient-scene="true">
      <AmbientSky location={location} />
      <header className="ambient-weather-heading">
        <div>
          <h1 role={current ? undefined : 'status'}>{current ? condition.label : failed ? 'Forecast unavailable' : 'Loading forecast…'}</h1>
          <button className="ambient-location" type="button" title="Change weather location"
            aria-label={`Change weather location, ${location.name}`} onClick={() => setEditing(true)}>
            <MapPin size={13} aria-hidden="true" /><span>{location.name}</span>
          </button>
        </div>
        <button className="ambient-temperature" type="button" disabled={saving} title={switchUnits}
          aria-label={temperature === '—' ? switchUnits : `${temperature}${fahrenheit ? 'F' : 'C'}. ${switchUnits}`}
          onClick={() => void save({ location, fahrenheit: !fahrenheit })}>
          {temperature}{temperature !== '—' && <small>{fahrenheit ? 'F' : 'C'}</small>}
        </button>
      </header>
      {failed && <footer className="ambient-weather-footer">
        <button type="button" className="weather-text-button" onClick={retry}>Retry forecast</button>
      </footer>}
      {alert}
    </main>;
  }

  if (design === 'colorful' && location && !editing && forecast) {
    return <main {...panel}>
      <div className="system-panel-body colorful-weather-body">
        <ColorfulWeather forecast={forecast} condition={condition} location={location} fahrenheit={fahrenheit} saving={saving}
          onLocation={() => setEditing(true)} onUnits={() => void save({ location, fahrenheit: !fahrenheit })} />
        {retryNotice}
      </div>
      {alert}
    </main>;
  }

  return <main {...panel}>
    <header className="system-panel-header weather-header" data-weather-design={design}>
      {design === 'clean' && <CloudSun size={19} aria-hidden="true" />}
      <h1>{selecting || !location ? 'Weather' : location.name}</h1>
    </header>
    <div className="system-panel-body weather-body">
      {loading && <p className="utility-message" role="status">Loading Weather…</p>}
      {selecting ? <>
        <p className="utility-message">Choose a city to show its forecast.</p>
        <LocationSearch focus={keyboard || editing} saving={saving} onPick={(city) => void save({ location: city, fahrenheit })}
          onCancel={location ? () => setEditing(false) : undefined} />
      </> : location && design !== 'ambient' && <>
        {forecast && (design === 'clean' || design === '3d') ? <>
          <CurrentWeather forecast={forecast} fahrenheit={fahrenheit} design={design} condition={condition} />
          <HourlyForecast forecast={forecast} fahrenheit={fahrenheit} design={design} />
        </> : !failed && <p className="utility-message" role="status">Loading the forecast…</p>}
        {retryNotice}
        <footer className="weather-footer">
          <button type="button" className="weather-text-button" onClick={() => setEditing(true)}><MapPin size={14} aria-hidden="true" />Change location</button>
          <div className="weather-units" role="group" aria-label="Temperature units">
            {[false, true].map((imperial) => <button key={String(imperial)} type="button" aria-pressed={fahrenheit === imperial} disabled={saving}
              aria-label={imperial ? 'Fahrenheit' : 'Celsius'} onClick={() => { if (imperial !== fahrenheit) void save({ location, fahrenheit: imperial }); }}>
              {imperial ? '°F' : '°C'}
            </button>)}
          </div>
        </footer>
      </>}
    </div>
    {alert}
  </main>;
}
