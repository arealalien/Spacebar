import { invoke } from '@tauri-apps/api/core';
import { CalendarDays, ListChecks, Paintbrush, Pipette, StickyNote } from 'lucide-react';
import { widgetNames } from './widgetCatalog';
import { useAgendaTime, type AgendaEvent, type QuickTask } from './widgetContent';
import { useSurfacePayload } from './useSurfacePayload';
import { dockPopupAnchor } from './widgets';

const icons = { 'quick-note': StickyNote, 'drawing-pad': Paintbrush, agenda: CalendarDays, 'quick-tasks': ListChecks, 'color-picker': Pipette };
type PersonalKind = keyof typeof icons;
export function isPersonalWidget(kind: string): kind is PersonalKind {
  return Object.hasOwn(icons, kind);
}

export function PersonalGlyph({ kind }: { kind: PersonalKind }) {
  const Icon = icons[kind];
  return <Icon aria-hidden="true" />;
}

const relativeTime = new Intl.RelativeTimeFormat(undefined, { numeric: 'always', style: 'short' });
function timeUntil(milliseconds: number) {
  const minutes = Math.ceil(milliseconds / 60_000);
  if (minutes < 60) return relativeTime.format(Math.max(1, minutes), 'minute');
  if (minutes < 1440) return relativeTime.format(Math.round(minutes / 60), 'hour');
  return relativeTime.format(Math.round(minutes / 1440), 'day');
}

export function PersonalWidget({ kind, specimen, onError = console.error }: {
  kind: PersonalKind; specimen: boolean; onError?: (error: unknown) => void;
}) {
  const agenda = kind === 'agenda';
  const tasks = kind === 'quick-tasks';
  const events = useSurfacePayload<AgendaEvent[]>('agenda-changed', 'read_agenda', agenda && !specimen);
  const checklist = useSurfacePayload<QuickTask[]>('quick-tasks-changed', 'read_quick_tasks', tasks && !specimen);
  const now = useAgendaTime(agenda && !specimen);
  const next = events?.reduce<AgendaEvent | undefined>((soonest, event) =>
    event.startsAt >= now && (!soonest || event.startsAt < soonest.startsAt) ? event : soonest, undefined);

  const name = widgetNames[kind];
  const remaining = checklist?.filter((task) => !task.done);
  const label = remaining?.[0]?.title ?? next?.title ?? name;
  const urgency = tasks
    ? specimen ? 'Next task' : !checklist ? '' : remaining?.length ? `${remaining.length} to do` : checklist.length ? 'All done' : 'No tasks'
    : next ? timeUntil(next.startsAt - now) : specimen ? 'Next event' : 'No upcoming events';
  const showText = agenda || tasks;
  const face = <><PersonalGlyph kind={kind} />{showText && <span className="agenda-face-text">
    <strong>{label}</strong><small>{urgency}</small>
  </span>}</>;
  const className = `widget-face personal-face${showText ? ' agenda-face' : ''}`;
  if (specimen) return <span className={className}>{face}</span>;
  return <button type="button" className={className} aria-label={`Open ${name}${label !== name ? `, ${label}, ${urgency}` : ''}`}
    title={label !== name ? `${label} · ${urgency}` : name} aria-haspopup="dialog" onClick={(event) =>
      void invoke('open_system_panel', { panel: kind, anchor: dockPopupAnchor(event.currentTarget) }).catch(onError)}>
    {face}
  </button>;
}
