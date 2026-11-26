import { invoke } from '@tauri-apps/api/core';
import { ExternalLink, Plus, Trash2 } from 'lucide-react';
import { useCallback, useEffect, useRef, useState, type FormEvent } from 'react';
import { useAgendaTime, useWidgetContent, writeWidgetContent, type AgendaEvent } from './widgetContent';

type PanelProps = { onError: (error: unknown) => void };

export function QuickNote({ onError }: PanelProps) {
  const { value, setValue, loading } = useWidgetContent('quick-note', onError);
  const pending = useRef<string>(undefined);
  const timer = useRef<number>(undefined);
  const flush = useCallback(() => {
    window.clearTimeout(timer.current);
    const note = pending.current;
    if (note === undefined) return;
    pending.current = undefined;
    void writeWidgetContent('quick-note', note).then(() => onError(''), onError);
  }, [onError]);
  useEffect(() => flush, [flush]);

  if (value === undefined) return loading ? <p className="system-panel-note" role="status">Loading note…</p> : null;
  return <textarea className="quick-note-editor" aria-label="Note" placeholder="Write a note…" rows={7}
    value={value} maxLength={32_000} autoFocus onBlur={flush} onChange={(event) => {
      const note = event.currentTarget.value;
      setValue(note);
      pending.current = note;
      window.clearTimeout(timer.current);
      timer.current = window.setTimeout(flush, 220);
    }} />;
}

const clock = new Intl.DateTimeFormat(undefined, { hour: 'numeric', minute: '2-digit' });
const shortDate = new Intl.DateTimeFormat(undefined, { weekday: 'short', month: 'short', day: 'numeric' });
const onSameDay = (a: Date, b: Date) =>
  a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate();

function dateTimeValue(time: number) {
  const local = new Date(time);
  return new Date(time - local.getTimezoneOffset() * 60_000).toISOString().slice(0, 16);
}

interface EventDraft { id?: string; title: string; time: string; link: string }

export function AgendaPanel({ onError }: PanelProps) {
  const { value, setValue, loading } = useWidgetContent('agenda', onError);
  const now = useAgendaTime();
  const [draft, setDraft] = useState<EventDraft>();
  const [saving, setSaving] = useState(false);
  const entries = value ?? [];

  function edit(event?: AgendaEvent) {
    onError('');
    const startsAt = event?.startsAt ?? Math.ceil((Date.now() + 60_000) / 900_000) * 900_000;
    setDraft({ id: event?.id, title: event?.title ?? '', time: dateTimeValue(startsAt), link: event?.link ?? '' });
  }

  async function save(next: AgendaEvent[]) {
    if (saving) return;
    onError('');
    setSaving(true);
    try {
      await writeWidgetContent('agenda', next);
      setValue(next);
      setDraft(undefined);
    } catch (error) { onError(error); }
    finally { setSaving(false); }
  }

  function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!draft) return;
    const title = draft.title.trim();
    const startsAt = new Date(draft.time).getTime();
    if (!title || !Number.isFinite(startsAt)) {
      onError('Enter an event name and start time.');
      return;
    }
    const entry = { id: draft.id ?? crypto.randomUUID(), title, startsAt, link: draft.link.trim() || null };
    void save([...entries.filter((event) => event.id !== entry.id), entry]);
  }

  if (!value) return loading ? <p className="system-panel-note" role="status">Loading agenda…</p> : null;
  if (draft) return <form className="agenda-editor" onSubmit={submit}>
    <label>Event name<input value={draft.title} autoFocus required maxLength={200}
      onChange={(event) => setDraft({ ...draft, title: event.currentTarget.value })} /></label>
    <label>Starts<input type="datetime-local" value={draft.time} required
      onChange={(event) => setDraft({ ...draft, time: event.currentTarget.value })} /></label>
    <label><span>Web link <small>Optional</small></span><input type="url" value={draft.link} placeholder="https://" maxLength={2048}
      onChange={(event) => setDraft({ ...draft, link: event.currentTarget.value })} /></label>
    <div className="agenda-editor-actions">
      {draft.id && <button type="button" className="agenda-remove" disabled={saving} onClick={() =>
        void save(entries.filter((event) => event.id !== draft.id))}><Trash2 size={13} />Remove</button>}
      <button type="button" disabled={saving} onClick={() => setDraft(undefined)}>Cancel</button>
      <button type="submit" className="agenda-primary" disabled={saving}>{saving ? 'Saving…' : 'Save event'}</button>
    </div>
  </form>;

  const today = new Date(now);
  const sorted = [...entries].sort((a, b) => a.startsAt - b.startsAt);
  const eventsToday = sorted.filter((event) => onSameDay(new Date(event.startsAt), today));
  const otherDays = sorted.filter((event) => !onSameDay(new Date(event.startsAt), today));
  const upcoming = otherDays.filter((event) => event.startsAt >= now);
  const past = otherDays.filter((event) => event.startsAt < now).reverse();

  function eventList(events: AgendaEvent[], showDate: boolean) {
    return <ul className="agenda-events">
      {events.map((event) => {
        const start = new Date(event.startsAt);
        return <li key={event.id} className="agenda-event" data-past={event.startsAt < now}>
          <button type="button" className="agenda-event-title" title={event.title} onClick={() => edit(event)}>
            <time dateTime={start.toISOString()}>{showDate && <span>{shortDate.format(start)}</span>}{clock.format(start)}</time>
            <strong>{event.title}</strong>
          </button>
          {event.link && <button type="button" className="agenda-event-link" title="Open event link" aria-label={`Open link for ${event.title}`}
            onClick={() => void invoke('open_agenda_link', { id: event.id }).catch(onError)}><ExternalLink size={15} /></button>}
        </li>;
      })}
    </ul>;
  }

  return <div className="agenda-content">
    <div className="agenda-today">
      <span>{shortDate.format(today)}</span>
      <button type="button" onClick={() => edit()}><Plus size={14} />Add event</button>
    </div>
    <section aria-label="Today">
      {eventsToday.length ? eventList(eventsToday, false) : <p className="agenda-empty">No events today</p>}
    </section>
    {!!upcoming.length && <section><h2>Upcoming</h2>{eventList(upcoming, true)}</section>}
    {!!past.length && <details className="agenda-past"><summary>Past events</summary>{eventList(past, true)}</details>}
  </div>;
}
