import { ChevronLeft, ChevronRight } from 'lucide-react';
import { useState } from 'react';

const monthLabel = new Intl.DateTimeFormat(undefined, { month: 'long', year: 'numeric' });
const dayLabel = new Intl.DateTimeFormat(undefined, { weekday: 'short' });
const fullDate = new Intl.DateTimeFormat(undefined, { dateStyle: 'full' });

export function Calendar() {
  const today = new Date();
  const [offset, setOffset] = useState(0);
  const month = new Date(today.getFullYear(), today.getMonth() + offset, 1);
  const weekday = (month.getDay() + 6) % 7;
  const days = Array.from({ length: 42 }, (_, i) => new Date(month.getFullYear(), month.getMonth(), i - weekday + 1));
  return <section className="system-calendar" aria-label="Calendar">
    <div className="system-calendar-heading">
      <strong aria-live="polite">{monthLabel.format(month)}</strong>
      <button type="button" aria-label="Previous month" onClick={() => setOffset(offset - 1)}><ChevronLeft size={16} /></button>
      <button type="button" aria-label="Next month" onClick={() => setOffset(offset + 1)}><ChevronRight size={16} /></button>
    </div>
    <div className="system-calendar-grid">
      {days.slice(0, 7).map((day) => <span key={day.getDay()} className="calendar-weekday">{dayLabel.format(day)}</span>)}
      {days.map((day) => <time key={day.getTime()} title={fullDate.format(day)} data-outside={day.getMonth() !== month.getMonth()}
        aria-current={day.toDateString() === today.toDateString() ? 'date' : undefined}>{day.getDate()}</time>)}
    </div>
  </section>;
}
