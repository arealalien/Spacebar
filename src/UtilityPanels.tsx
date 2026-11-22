import { invoke } from '@tauri-apps/api/core';
import { Check, Copy, Pipette, Plus, X } from 'lucide-react';
import { useRef, useState } from 'react';
import { normalizeHexColor } from './model';
import { useWidgetContent, writeWidgetContent, type QuickTask } from './widgetContent';

type PanelProps = { onError: (error: unknown) => void };

export function QuickTasks({ onError }: PanelProps) {
  const { value: tasks, setValue, loading } = useWidgetContent('quick-tasks', onError);
  const [draft, setDraft] = useState('');
  const [saving, setSaving] = useState(false);
  const input = useRef<HTMLInputElement>(null);

  async function save(next: QuickTask[]) {
    if (saving) return false;
    setSaving(true);
    onError('');
    try {
      await writeWidgetContent('quick-tasks', next);
      setValue(next);
      return true;
    } catch (error) {
      onError(error);
      return false;
    } finally {
      setSaving(false);
    }
  }

  if (!tasks) return loading ? <p className="utility-message" role="status">Loading tasks…</p> : null;
  const remaining = tasks.filter((task) => !task.done).length;
  return <div className="utility-content">
    <form className="utility-form" onSubmit={(event) => {
      event.preventDefault();
      const title = draft.trim();
      if (!title || tasks.length >= 128) return;
      void save([...tasks, { id: crypto.randomUUID(), title, done: false }]).then((ok) => {
        if (ok) {
          setDraft((current) => current === draft ? '' : current);
          input.current?.focus();
        }
      });
    }}>
      <input ref={input} type="text" placeholder="Add a task…" aria-label="New task" autoFocus maxLength={160}
        value={draft} onChange={(event) => setDraft(event.currentTarget.value)} />
      <button type="submit" className="utility-icon-button" aria-label="Add task" title="Add task"
        disabled={!draft.trim() || saving || tasks.length >= 128}><Plus size={17} aria-hidden="true" /></button>
    </form>
    <p className="utility-message" role="status">{!tasks.length ? 'No tasks yet.' : remaining ? `${remaining} to do` : 'All tasks complete.'}</p>
    {tasks.length >= 128 && <p className="utility-message">Remove a task to make room for another.</p>}
    {!!tasks.length && <ul className="utility-list" aria-label="Tasks">
      {tasks.map((task) => <li className="utility-row task-row" data-complete={task.done} key={task.id}>
        <label><input type="checkbox" checked={task.done} aria-disabled={saving} onChange={() =>
          void save(tasks.map((item) => item.id === task.id ? { ...item, done: !item.done } : item))} />
          <span>{task.title}</span></label>
        <button type="button" className="utility-icon-button" aria-label={`Remove ${task.title}`} title="Remove task" aria-disabled={saving}
          onClick={() => void save(tasks.filter((item) => item.id !== task.id)).then((ok) => { if (ok) input.current?.focus(); })}>
          <X size={15} aria-hidden="true" />
        </button>
      </li>)}
    </ul>}
  </div>;
}

export function ScreenColors({ onError }: PanelProps) {
  const { value: colors, setValue, loading } = useWidgetContent('color-picker', onError);
  const [picking, setPicking] = useState(false);
  const [copied, setCopied] = useState('');
  const selecting = useRef(false);

  async function copy(color: string) {
    onError('');
    try {
      await invoke('copy_screen_color', { color });
      setCopied(color);
    } catch (error) {
      setCopied('');
      onError(error);
    }
  }

  async function pick() {
    if (!colors || selecting.current) return;
    selecting.current = true;
    setPicking(true);
    setCopied('');
    onError('');
    try {
      const result = await invoke<string | null>('pick_screen_color');
      if (!result) return;
      const hex = normalizeHexColor(result)?.toUpperCase();
      if (!hex) throw new Error('Couldn’t read that color.');
      const next = [hex, ...colors.filter((color) => color.toUpperCase() !== hex)].slice(0, 32);
      await Promise.all([
        copy(hex),
        writeWidgetContent('color-picker', next).then(() => setValue(next), onError),
      ]);
    } catch (error) {
      onError(error);
    } finally {
      selecting.current = false;
      setPicking(false);
    }
  }

  return <div className="utility-content">
    <button type="button" className="utility-action" disabled={picking || !colors} onClick={() => void pick()}>
      <Pipette size={17} aria-hidden="true" />{picking ? 'Picking color…' : 'Pick a screen color'}
    </button>
    <p className="utility-message" role="status">{picking ? 'Click a pixel on your screen. Press Esc to cancel.'
      : copied ? `Copied ${copied}` : loading ? 'Loading colors…' : 'Picked colors are copied to your clipboard.'}</p>
    {!!colors?.length && <>
      <h2 className="utility-section-title">Recent colors</h2>
      <ul className="utility-list" aria-label="Recent colors">
        {colors.map((color) => <li className="utility-row color-history-row" key={color}>
          <span className="picked-color-swatch" style={{ backgroundColor: color }} aria-hidden="true" />
          <code>{color}</code>
          <button type="button" className="utility-icon-button" aria-label={`Copy ${color}`} title={`Copy ${color}`} disabled={picking}
            onClick={() => void copy(color)}>
            {copied === color ? <Check size={16} aria-hidden="true" /> : <Copy size={15} aria-hidden="true" />}
          </button>
        </li>)}
      </ul>
    </>}
  </div>;
}
