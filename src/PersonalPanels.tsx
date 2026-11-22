import { invoke } from '@tauri-apps/api/core';
import { Eraser, ExternalLink, Paintbrush, Plus, Redo2, Trash2, Undo2 } from 'lucide-react';
import { useCallback, useEffect, useId, useRef, useState, type FormEvent, type PointerEvent } from 'react';
import { ColorPicker } from './ColorPicker';
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

const canvasWidth = 960;
const canvasHeight = 640;
const brushWidth = 8;
const eraserWidth = 28;
const initialBrush = '#303246';
const maxDrawingSteps = 25;
const maxDrawingHistorySize = 12 * 1024 * 1024;

type DrawingTool = 'brush' | 'eraser';
interface DrawingHistory {
  frames: (string | null)[];
  index: number;
}

// Keep undo available when the pad closes while Spacebar is still running.
let savedHistory: DrawingHistory | undefined;

function canvasPoint(canvas: HTMLCanvasElement, event: PointerEvent<HTMLCanvasElement>) {
  const bounds = canvas.getBoundingClientRect();
  return {
    x: (event.clientX - bounds.left) * canvasWidth / bounds.width,
    y: (event.clientY - bounds.top) * canvasHeight / bounds.height,
  };
}

async function restoreDrawing(canvas: HTMLCanvasElement, frame: string | null) {
  let saved: HTMLImageElement | undefined;
  if (frame) {
    saved = new Image();
    saved.src = frame;
    await saved.decode();
  }
  const context = canvas.getContext('2d');
  if (!context) throw new Error('Couldn’t open the drawing canvas.');
  context.globalCompositeOperation = 'source-over';
  context.clearRect(0, 0, canvasWidth, canvasHeight);
  if (saved) context.drawImage(saved, 0, 0, canvasWidth, canvasHeight);
}

export function DrawingPad({ onError }: PanelProps) {
  const { value, loading } = useWidgetContent('drawing-pad', onError);
  const [color, setColor] = useState(initialBrush);
  const [tool, setTool] = useState<DrawingTool>('brush');
  const [history, setHistory] = useState<DrawingHistory | null>();
  const [restoring, setRestoring] = useState(false);
  const colorId = useId();
  const canvas = useRef<HTMLCanvasElement>(null);
  const brushColor = useRef(initialBrush);
  const current = useRef<DrawingHistory>(undefined);
  const restorePending = useRef(false);
  const stroke = useRef<{ id: number; x: number; y: number; width: number; color: string }>(undefined);
  const remember = useCallback((next: DrawingHistory) => {
    current.current = next;
    savedHistory = next;
    setHistory(next);
  }, []);

  useEffect(() => {
    const sheet = canvas.current;
    if (!value || !sheet) return;
    brushColor.current = value.color;
    setColor(value.color);
    const next = savedHistory && savedHistory.frames[savedHistory.index] === value.image
      ? savedHistory : { frames: [value.image], index: 0 };
    let alive = true;
    void restoreDrawing(sheet, value.image).then(() => {
      if (alive) remember(next);
    }).catch(() => {
      if (!alive) return;
      setHistory(null);
      onError('Couldn’t load the saved drawing.');
    });
    return () => { alive = false; };
  }, [value, onError, remember]);

  function save(color: string) {
    const history = current.current;
    if (!history) return;
    void writeWidgetContent('drawing-pad', { color, image: history.frames[history.index] }).then(() => onError(''), onError);
  }

  async function moveHistory(step: -1 | 1) {
    const history = current.current;
    const sheet = canvas.current;
    if (!history || !sheet || stroke.current || restorePending.current) return;
    const index = history.index + step;
    if (index < 0 || index >= history.frames.length) return;
    restorePending.current = true;
    setRestoring(true);
    try {
      await restoreDrawing(sheet, history.frames[index]);
      if (canvas.current !== sheet) return;
      remember({ frames: history.frames, index });
      save(brushColor.current);
    } catch (error) { onError(error); }
    finally {
      restorePending.current = false;
      if (canvas.current === sheet) setRestoring(false);
    }
  }

  function begin(event: PointerEvent<HTMLCanvasElement>) {
    const history = current.current;
    if (!history || restorePending.current || event.button !== 0 || stroke.current) return;
    const erasing = tool === 'eraser';
    if (erasing && history.frames[history.index] === null) return;
    const sheet = event.currentTarget;
    const context = sheet.getContext('2d');
    if (!context) return;
    const width = erasing ? eraserWidth : brushWidth;
    const point = canvasPoint(sheet, event);
    sheet.setPointerCapture(event.pointerId);
    stroke.current = { id: event.pointerId, ...point, width, color: brushColor.current };
    context.globalCompositeOperation = erasing ? 'destination-out' : 'source-over';
    context.fillStyle = brushColor.current;
    context.beginPath();
    context.arc(point.x, point.y, width / 2, 0, 2 * Math.PI);
    context.fill();
  }

  function draw(event: PointerEvent<HTMLCanvasElement>) {
    const last = stroke.current;
    if (last?.id !== event.pointerId) return;
    const sheet = event.currentTarget;
    const context = sheet.getContext('2d');
    if (!context) return;
    const point = canvasPoint(sheet, event);
    context.strokeStyle = last.color;
    context.lineWidth = last.width;
    context.lineCap = 'round';
    context.lineJoin = 'round';
    context.beginPath();
    context.moveTo(last.x, last.y);
    context.lineTo(point.x, point.y);
    context.stroke();
    last.x = point.x;
    last.y = point.y;
  }

  function finish(event: PointerEvent<HTMLCanvasElement>) {
    const history = current.current;
    if (!history || stroke.current?.id !== event.pointerId) return;
    stroke.current = undefined;
    try {
      const frame = event.currentTarget.toDataURL('image/png');
      if (frame === history.frames[history.index]) return;
      const frames = [...history.frames.slice(0, history.index + 1), frame];
      let size = frames.reduce((total, image) => total + (image?.length ?? 0), 0);
      while (frames.length > 1 && (frames.length > maxDrawingSteps + 1 || size > maxDrawingHistorySize)) {
        size -= frames.shift()?.length ?? 0;
      }
      remember({ frames, index: frames.length - 1 });
      save(brushColor.current);
    } catch (error) { onError(error); }
  }

  return <div className="drawing-editor">
    <div className="drawing-tools">
      <div className="drawing-actions">
        <div className="drawing-tool-group" role="group" aria-label="Drawing tool">
          <button type="button" aria-label="Brush" title="Brush" aria-pressed={tool === 'brush'} disabled={!history || restoring}
            onClick={() => setTool('brush')}><Paintbrush size={15} /></button>
          <button type="button" aria-label="Eraser" title="Eraser" aria-pressed={tool === 'eraser'} disabled={!history || restoring}
            onClick={() => setTool('eraser')}><Eraser size={16} /></button>
        </div>
        <div className="drawing-tool-group" role="group" aria-label="Drawing history">
          <button type="button" aria-label="Undo" title="Undo" disabled={!history?.index || restoring}
            onClick={() => void moveHistory(-1)}><Undo2 size={17} /></button>
          <button type="button" aria-label="Redo" title="Redo" disabled={!history || history.index >= history.frames.length - 1 || restoring}
            onClick={() => void moveHistory(1)}><Redo2 size={17} /></button>
        </div>
      </div>
      <div className="drawing-color">
        <label htmlFor={colorId}>Brush color</label>
        {history && <ColorPicker id={colorId} label="Brush color" value={color}
          onInput={(next) => { brushColor.current = next; setColor(next); }} onCommit={save} />}
      </div>
    </div>
    <div className="drawing-sheet" aria-busy={loading || (!!value && history === undefined) || restoring}>
      <canvas ref={canvas} aria-label="Drawing canvas" width={canvasWidth} height={canvasHeight} data-ready={!!history && !restoring}
        onPointerDown={begin} onPointerMove={draw} onPointerUp={(event) => { draw(event); finish(event); }}
        onPointerCancel={finish} onLostPointerCapture={finish} />
    </div>
  </div>;
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
