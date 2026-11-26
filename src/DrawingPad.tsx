import { invoke } from '@tauri-apps/api/core';
import { Eraser, Image as ImageIcon, Paintbrush, Plus, Redo2, Save, Trash2, Undo2 } from 'lucide-react';
import { useCallback, useEffect, useId, useRef, useState, type PointerEvent } from 'react';
import { ColorPicker } from './ColorPicker';

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
const savedHistories = new Map<number, DrawingHistory>();
const drawingWrites = new Map<number, Promise<void>>();
let lastDrawing: number | undefined;

interface DrawingDocument { color: string; image: string | null }

function writeDrawing(id: number, drawing: DrawingDocument) {
  const previous = drawingWrites.get(id);
  const request = (previous?.catch(() => {}) ?? Promise.resolve()).then(() =>
    invoke<void>('save_drawing', { id, drawing }));
  drawingWrites.set(id, request);
  const release = () => { if (drawingWrites.get(id) === request) drawingWrites.delete(id); };
  void request.then(release, release);
  return request;
}

export function DrawingPad({ onError }: { onError: (error: unknown) => void }) {
  const [drawings, setDrawings] = useState<number[]>();
  const [selected, setSelected] = useState<number>();
  const [creating, setCreating] = useState(false);
  const [menu, setMenu] = useState<{ id: number; x: number; y: number; button: HTMLButtonElement }>();
  const [deleteTarget, setDeleteTarget] = useState<number>();
  const [deleting, setDeleting] = useState(false);
  const newButton = useRef<HTMLButtonElement>(null);
  const menuItem = useRef<HTMLButtonElement>(null);
  const deleteDialog = useRef<HTMLDialogElement>(null);
  const deleteTitle = useId();

  function select(id: number | undefined) {
    lastDrawing = id;
    onError('');
    setSelected(id);
  }

  useEffect(() => {
    let alive = true;
    void invoke<number[]>('list_drawings').then((ids) => {
      if (!alive) return;
      setDrawings(ids);
      const id = lastDrawing !== undefined && ids.includes(lastDrawing) ? lastDrawing : ids.at(-1);
      setSelected(id);
    }, (error) => {
      if (alive) { setDrawings([]); onError(error); }
    });
    return () => { alive = false; };
  }, [onError]);

  useEffect(() => {
    if (!menu) return;
    menuItem.current?.focus();
    function dismiss(event: globalThis.PointerEvent) {
      if (!menuItem.current?.contains(event.target as Node)) setMenu(undefined);
    }
    window.addEventListener('pointerdown', dismiss);
    return () => window.removeEventListener('pointerdown', dismiss);
  }, [menu]);

  useEffect(() => {
    if (deleteTarget !== undefined) deleteDialog.current?.showModal();
  }, [deleteTarget]);

  async function create() {
    if (creating || deleteTarget !== undefined) return;
    setMenu(undefined);
    setCreating(true);
    try {
      const id = await invoke<number>('create_drawing');
      setDrawings((ids) => [...(ids ?? []), id]);
      select(id);
    } catch (error) { onError(error); }
    finally { setCreating(false); }
  }

  function openMenu(id: number, button: HTMLButtonElement, x: number, y: number) {
    if (creating || deleteTarget !== undefined) return;
    button.focus({ preventScroll: true });
    setMenu({
      id, button,
      x: Math.max(6, Math.min(x, window.innerWidth - 172)),
      y: Math.max(6, Math.min(y, window.innerHeight - 46)),
    });
  }

  async function removeDrawing() {
    if (deleteTarget === undefined || !drawings || deleting) return;
    setDeleting(true);
    try {
      await drawingWrites.get(deleteTarget)?.catch(() => {});
      await invoke('delete_drawing', { id: deleteTarget });
      const index = drawings.indexOf(deleteTarget);
      const remaining = drawings.filter((id) => id !== deleteTarget);
      savedHistories.delete(deleteTarget);
      setDrawings(remaining);
      if (selected === deleteTarget) select(remaining[index] ?? remaining.at(-1));
      deleteDialog.current?.close();
      setDeleteTarget(undefined);
      newButton.current?.focus();
    } catch (error) {
      deleteDialog.current?.close();
      setDeleteTarget(undefined);
      onError(error);
    } finally {
      setDeleting(false);
    }
  }

  return <div className="drawing-library">
    <aside className="drawing-sidebar" aria-label="Drawings">
      <button type="button" ref={newButton} className="drawing-new" disabled={!drawings || creating} onClick={() => void create()}>
        <Plus size={15} aria-hidden="true" />New drawing
      </button>
      <ul aria-label="Saved drawings">
        {drawings?.map((id) => <li key={id}><button type="button" aria-current={selected === id || undefined}
          aria-haspopup="menu" aria-expanded={menu?.id === id} onClick={() => select(id)}
          onContextMenu={(event) => {
            event.preventDefault();
            openMenu(id, event.currentTarget, event.clientX, event.clientY);
          }}
          onKeyDown={(event) => {
            if (event.key !== 'ContextMenu' && !(event.shiftKey && event.key === 'F10')) return;
            event.preventDefault();
            const bounds = event.currentTarget.getBoundingClientRect();
            openMenu(id, event.currentTarget, bounds.left, bounds.bottom);
          }}><ImageIcon size={15} aria-hidden="true" />Drawing {id}</button></li>)}
      </ul>
    </aside>
    {selected !== undefined ? <DrawingCanvas key={selected} id={selected} onError={onError} />
      : <p className="system-panel-note" role="status">{drawings ? 'Create a drawing to get started.' : 'Loading drawings…'}</p>}
    {menu && <div className="drawing-item-menu" role="menu" aria-label={`Drawing ${menu.id} actions`}
      style={{ left: menu.x, top: menu.y }} onKeyDown={(event) => {
        if (event.key !== 'Escape' && event.key !== 'Tab') return;
        event.preventDefault();
        event.stopPropagation();
        menu.button.focus();
        setMenu(undefined);
      }}>
      <button type="button" role="menuitem" ref={menuItem} onClick={() => {
        menu.button.focus();
        setDeleteTarget(menu.id);
        setMenu(undefined);
      }}><Trash2 size={15} aria-hidden="true" />Delete drawing</button>
    </div>}
    <dialog className="drawing-confirm" ref={deleteDialog} aria-labelledby={deleteTitle}
      onCancel={(event) => { if (deleting) event.preventDefault(); }}
      onClose={() => setDeleteTarget(undefined)}
      onKeyDown={(event) => { if (event.key === 'Escape') event.stopPropagation(); }}>
      <h2 id={deleteTitle}>Delete Drawing {deleteTarget}?</h2>
      <p>This permanently removes the drawing from Drawing Pad.</p>
      <div>
        <button type="button" autoFocus disabled={deleting} onClick={() => deleteDialog.current?.close()}>Cancel</button>
        <button type="button" className="drawing-clear-confirm" disabled={deleting} onClick={() => void removeDrawing()}>
          {deleting ? 'Deleting…' : 'Delete drawing'}
        </button>
      </div>
    </dialog>
  </div>;
}

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

function DrawingCanvas({ id, onError }: { id: number; onError: (error: unknown) => void }) {
  const [color, setColor] = useState(initialBrush);
  const [tool, setTool] = useState<DrawingTool>('brush');
  const [history, setHistory] = useState<DrawingHistory | null>();
  const [restoring, setRestoring] = useState(false);
  const [exporting, setExporting] = useState(false);
  const [exported, setExported] = useState(false);
  const colorId = useId();
  const clearTitle = useId();
  const confirmation = useRef<HTMLDialogElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const brushColor = useRef(initialBrush);
  const current = useRef<DrawingHistory>(undefined);
  const restorePending = useRef(false);
  const stroke = useRef<{ id: number; x: number; y: number; width: number; color: string }>(undefined);
  const remember = useCallback((next: DrawingHistory) => {
    current.current = next;
    savedHistories.delete(id);
    savedHistories.set(id, next);
    if (savedHistories.size > 6) {
      const oldest = savedHistories.keys().next().value;
      if (oldest !== undefined) savedHistories.delete(oldest);
    }
    setHistory(next);
  }, [id]);

  useEffect(() => {
    const sheet = canvas.current;
    if (!sheet) return;
    let alive = true;
    async function load() {
      try {
        await drawingWrites.get(id)?.catch(() => {});
        const value = await invoke<DrawingDocument>('read_drawing', { id });
        if (!alive || !sheet) return;
        const saved = savedHistories.get(id);
        const next = saved && saved.frames[saved.index] === value.image ? saved : { frames: [value.image], index: 0 };
        await restoreDrawing(sheet, value.image);
        if (!alive) return;
        brushColor.current = value.color;
        setColor(value.color);
        remember(next);
      } catch (error) {
        if (alive) {
          setHistory(null);
          onError(error);
        }
      }
    }
    void load();
    return () => { alive = false; };
  }, [id, onError, remember]);

  function save(color: string) {
    const history = current.current;
    if (history) void writeDrawing(id, { color, image: history.frames[history.index] }).then(() => onError(''), onError);
  }

  function commit(frame: string | null) {
    const history = current.current;
    if (!history || frame === history.frames[history.index]) return;
    const frames = [...history.frames.slice(0, history.index + 1), frame];
    let size = frames.reduce((total, image) => total + (image?.length ?? 0), 0);
    while (frames.length > 1 && (frames.length > maxDrawingSteps + 1 || size > maxDrawingHistorySize)) {
      size -= frames.shift()?.length ?? 0;
    }
    remember({ frames, index: frames.length - 1 });
    setExported(false);
    save(brushColor.current);
  }

  function clearCanvas() {
    confirmation.current?.close();
    const context = canvas.current?.getContext('2d');
    if (!context || restorePending.current || stroke.current) return;
    context.clearRect(0, 0, canvasWidth, canvasHeight);
    commit(null);
  }

  async function exportImage() {
    const sheet = canvas.current;
    if (!sheet || exporting || restorePending.current || stroke.current) return;
    setExporting(true);
    setExported(false);
    onError('');
    try {
      const image = document.createElement('canvas');
      image.width = canvasWidth;
      image.height = canvasHeight;
      const context = image.getContext('2d');
      if (!context) throw new Error("Couldn't prepare the PNG.");
      context.fillStyle = '#fbf9f3';
      context.fillRect(0, 0, canvasWidth, canvasHeight);
      context.drawImage(sheet, 0, 0);
      setExported(await invoke<boolean>('export_drawing', { id, image: image.toDataURL('image/png') }));
    } catch (error) { onError(error); }
    finally { setExporting(false); }
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
      commit(event.currentTarget.toDataURL('image/png'));
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
        <div className="drawing-tool-group" role="group" aria-label="Canvas actions">
          <button type="button" aria-label="Save as PNG" title="Save as PNG" disabled={!history || restoring || exporting}
            onClick={() => void exportImage()}><Save size={16} /></button>
          <button type="button" aria-label="Clear canvas" title="Clear canvas" disabled={!history?.frames[history.index] || restoring || exporting}
            onClick={() => { if (!stroke.current && !restorePending.current) confirmation.current?.showModal(); }}><Trash2 size={16} /></button>
        </div>
      </div>
      <div className="drawing-color">
        <label htmlFor={colorId}>Brush color</label>
        {history && <ColorPicker id={colorId} label="Brush color" value={color}
          onInput={(next) => { brushColor.current = next; setColor(next); }} onCommit={save} />}
      </div>
    </div>
    <div className="drawing-sheet" aria-busy={history === undefined || restoring}>
      <canvas ref={canvas} aria-label="Drawing canvas" width={canvasWidth} height={canvasHeight} data-ready={!!history && !restoring}
        onPointerDown={begin} onPointerMove={draw} onPointerUp={(event) => { draw(event); finish(event); }}
        onPointerCancel={finish} onLostPointerCapture={finish} />
    </div>
    {exported && <span className="drawing-exported" role="status">PNG saved</span>}
    <dialog className="drawing-confirm" ref={confirmation} aria-labelledby={clearTitle}
      onKeyDown={(event) => { if (event.key === 'Escape') event.stopPropagation(); }}>
      <h2 id={clearTitle}>Clear Drawing {id}?</h2>
      <p>This clears the current canvas. You can undo it while Drawing Pad is open.</p>
      <div>
        <button type="button" autoFocus onClick={() => confirmation.current?.close()}>Cancel</button>
        <button type="button" className="drawing-clear-confirm" onClick={clearCanvas}>Clear canvas</button>
      </div>
    </dialog>
  </div>;
}
