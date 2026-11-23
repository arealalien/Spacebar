import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { FolderOpen, Image as ImageIcon, ImageUp, RotateCcw, X } from 'lucide-react';
import { useEffect, useRef, useState, type CSSProperties, type PointerEvent } from 'react';
import { contrastingInk, defaultAppAppearance, desktop, readAppAppearance } from './model';
import { readImageWidget, saveImageWidget, type ImageCrop, type ImageRatio, type ImageWidgetOptions } from './imageWidgetStorage';
import { useSurfacePayload } from './useSurfacePayload';
import { allWidgets, configureImageWidget, editWidgets, useWidgets, type WidgetItem } from './widgets';

const ratios = [
  { ratio: 1, label: 'Square' },
  { ratio: 2, label: 'Rectangle' },
  { ratio: 3, label: 'Wide' },
] as const;
const acceptedImages = 'image/png,image/jpeg,image/gif,image/webp,.png,.jpg,.jpeg,.gif,.webp,.apng';

let browserEditor: Window | null = null;

function imageEditorUrl(id?: string) {
  const url = new URL(window.location.href);
  url.search = '';
  url.searchParams.set('surface', 'image-editor');
  if (id) url.searchParams.set('widget', id);
  return url;
}

export async function openImageEditor(id?: string) {
  if (desktop) {
    await invoke('open_image_editor', { id });
    return;
  }
  if (browserEditor && !browserEditor.closed) {
    browserEditor.focus();
    return;
  }
  browserEditor = window.open(imageEditorUrl(id), 'spacebar-image-editor', 'popup,width=816,height=560');
  if (!browserEditor) throw new Error('Allow pop-ups to open the image editor.');
}

function closeImageEditor() {
  if (desktop) return getCurrentWindow().close();
  if (window.opener) window.close();
  else window.location.replace(window.location.pathname);
  return Promise.resolve();
}

function croppedImageStyle(crop: ImageCrop): CSSProperties {
  return {
    left: `${-crop.x / crop.width * 100}%`, top: `${-crop.y / crop.height * 100}%`,
    width: `${100 / crop.width}%`, height: `${100 / crop.height}%`,
  };
}

export function ImageWidgetFace({ item, specimen = false, onError }: {
  item?: WidgetItem; specimen?: boolean; onError?: (error: unknown) => void;
}) {
  const image = item?.image;
  const id = image?.asset;
  const [loaded, setLoaded] = useState<{ id: string; url: string }>();

  useEffect(() => {
    if (!id) return;
    let active = true;
    let url: string | undefined;
    void readImageWidget(id, false).then((blob) => {
      if (!active) return;
      url = URL.createObjectURL(blob);
      setLoaded({ id, url });
    }).catch(() => { if (active) setLoaded(undefined); });
    return () => {
      active = false;
      if (url) URL.revokeObjectURL(url);
    };
  }, [id]);

  const url = loaded?.id === id ? loaded?.url : undefined;
  const crop = image?.crop;
  const style = !desktop && crop ? croppedImageStyle(crop) : undefined;
  const content = url ? <img src={url} style={style} alt="" draggable={false} /> : <ImageIcon aria-hidden="true" />;
  if (specimen || !item) return <span className="widget-face image-widget-face" data-loaded={!!url}>{content}</span>;
  const label = image ? `Edit image: ${image.name}` : 'Choose an image or GIF';
  return <button type="button" className="widget-face image-widget-face" data-loaded={!!url} aria-label={label}
    onClick={() => void openImageEditor(item.id).catch(onError)}>{content}</button>;
}

type Point = { x: number; y: number };
type Dimensions = { width: number; height: number };
const center: Point = { x: .5, y: .5 };
const maxZoom = 4;
const cropCorners = [
  { name: 'top-left', x: -1, y: -1 },
  { name: 'top-right', x: 1, y: -1 },
  { name: 'bottom-left', x: -1, y: 1 },
  { name: 'bottom-right', x: 1, y: 1 },
] as const;
type CropDrag = {
  id: number;
  source: HTMLDivElement;
  offset: Point;
  resize?: { corner: typeof cropCorners[number]; anchor: Point };
};

function cropSize(size: Dimensions, ratio: ImageRatio, zoom: number) {
  const height = Math.min(size.height, size.width / ratio) / zoom;
  return { width: height * ratio / size.width, height: height / size.height };
}

function clampCenter(point: Point, size: Dimensions): Point {
  return {
    x: Math.max(size.width / 2, Math.min(1 - size.width / 2, point.x)),
    y: Math.max(size.height / 2, Math.min(1 - size.height / 2, point.y)),
  };
}

export function ImageEditorWindow({ widgetId }: { widgetId: string | null }) {
  const { items, ready } = useWidgets();
  const item = widgetId ? allWidgets(items).find((item) => item.id === widgetId && item.kind === 'image') : undefined;
  const [appearance, setAppearance] = useState(defaultAppAppearance);
  const backdrop = useSurfacePayload<boolean>('image-editor-backdrop-changed', 'read_image_editor_backdrop');
  useEffect(() => {
    document.title = 'Spacebar Image Editor';
    let active = true;
    void readAppAppearance().then((value) => { if (active) setAppearance(value); }).catch(console.error);
    return () => { active = false; };
  }, []);

  return <main className="app-shell image-editor-window" data-theme={appearance.theme} data-native={desktop || undefined}
    data-backdrop={backdrop || undefined} data-blur-off={appearance.blurAmount === 0 || undefined} aria-label="Spacebar Image Editor"
    style={{
      '--app-tint': appearance.tintColor, '--app-blur': appearance.blurAmount / 100,
      '--app-tint-strength': appearance.tintAmount / 100, '--on-accent': contrastingInk(appearance.tintColor),
    } as CSSProperties}>
    {widgetId && !ready ? <div className="image-editor-pending" role="status">Opening image…</div>
      : <ImageCropEditor widgetId={widgetId} item={item} />}
  </main>;
}

function ImageCropEditor({ widgetId, item }: { widgetId: string | null; item?: WidgetItem }) {
  const [saved] = useState(item?.image);
  const missing = !!widgetId && !item;
  const filePicker = useRef<HTMLInputElement>(null);
  const [source, setSource] = useState<{ blob: Blob; name: string; previous?: ImageWidgetOptions }>();
  const [url, setUrl] = useState('');
  const [dimensions, setDimensions] = useState<Dimensions>();
  const [ratio, setRatio] = useState<ImageRatio>(saved?.crop.ratio ?? 1);
  const [position, setPosition] = useState(center);
  const [zoom, setZoom] = useState(1);
  const [loading, setLoading] = useState(!!saved);
  const [saving, setSaving] = useState(false);
  const [submitted, setSubmitted] = useState(false);
  const [error, setError] = useState('');
  const [dropping, setDropping] = useState(false);
  const dropDepth = useRef(0);
  const saveInProgress = useRef(false);
  const drag = useRef<CropDrag | undefined>(undefined);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape' && !event.defaultPrevented && !saveInProgress.current) {
        event.preventDefault();
        void closeImageEditor().catch((error) => setError(String(error)));
      }
    };
    window.addEventListener('keydown', onKey);
    const subscription = desktop ? getCurrentWindow().onCloseRequested((event) => {
      if (saveInProgress.current) event.preventDefault();
    }) : undefined;
    return () => {
      window.removeEventListener('keydown', onKey);
      void subscription?.then((unlisten) => unlisten()).catch(console.error);
    };
  }, []);

  useEffect(() => {
    if (!saved) return;
    let active = true;
    void readImageWidget(saved.source, true).then((blob) => {
      if (active) setSource({ blob, name: saved.name, previous: saved });
    }).catch((error) => {
      if (active) setError(String(error));
    }).finally(() => { if (active) setLoading(false); });
    return () => { active = false; };
  }, [saved]);

  useEffect(() => {
    if (!source) return;
    const local = URL.createObjectURL(source.blob);
    setUrl(local);
    return () => URL.revokeObjectURL(local);
  }, [source]);

  const size = dimensions && cropSize(dimensions, ratio, zoom);
  const bounded = size ? clampCenter(position, size) : center;
  const crop: ImageCrop | undefined = size && {
    x: bounded.x - size.width / 2, y: bounded.y - size.height / 2, ...size, ratio,
  };
  const zoomPercent = Math.round(zoom * 100);
  const zoomLabel = `${zoomPercent}%`;
  const busy = loading || (!!url && !dimensions && !error);
  const canChoose = !loading && !saving && !missing && !submitted;
  const dismiss = () => void closeImageEditor().catch((error) => setError(String(error)));
  const browse = () => filePicker.current?.click();

  function choose(file: File) {
    setError('');
    if (file.size > 24 * 1024 * 1024) { setError('Choose an image under 24 MB.'); return; }
    if (!/\.(png|jpe?g|gif|webp|apng)$/i.test(file.name)) { setError('Choose a PNG, JPEG, GIF, or WebP image.'); return; }
    drag.current = undefined;
    setUrl('');
    setDimensions(undefined);
    setPosition(center);
    setZoom(1);
    setSource({ blob: file, name: file.name });
  }

  function loadImage(image: HTMLImageElement) {
    const size = { width: image.naturalWidth, height: image.naturalHeight };
    if (size.width > 8192 || size.height > 8192 || size.width * size.height > 32_000_000) {
      setError('Choose an image under 32 megapixels and no more than 8192 pixels on either side.');
      return;
    }
    setDimensions(size);
    const previous = source?.previous?.crop;
    if (previous) {
      setPosition({ x: previous.x + previous.width / 2, y: previous.y + previous.height / 2 });
      setZoom(cropSize(size, previous.ratio, 1).height / previous.height);
    }
  }

  function pointAt(event: PointerEvent<HTMLDivElement>, source: HTMLDivElement): Point {
    const bounds = source.getBoundingClientRect();
    return { x: (event.clientX - bounds.left) / bounds.width, y: (event.clientY - bounds.top) / bounds.height };
  }

  function startCropDrag(event: PointerEvent<HTMLDivElement>) {
    if (!crop || saving || event.button !== 0 || !event.isPrimary || drag.current) return;
    event.preventDefault();
    const source = event.currentTarget;
    source.focus({ preventScroll: true });
    source.setPointerCapture(event.pointerId);
    const point = pointAt(event, source);
    const target = event.target as HTMLElement;
    const corner = cropCorners.find(({ name }) => name === target.dataset.cropCorner);
    if (corner) {
      const x = corner.x * crop.width / 2;
      const y = corner.y * crop.height / 2;
      drag.current = {
        id: event.pointerId,
        source,
        offset: { x: point.x - bounded.x - x, y: point.y - bounded.y - y },
        resize: { corner, anchor: { x: bounded.x - x, y: bounded.y - y } },
      };
      source.style.cursor = corner.x === corner.y ? 'nwse-resize' : 'nesw-resize';
      return;
    }

    const inside = point.x >= crop.x && point.x <= crop.x + crop.width
      && point.y >= crop.y && point.y <= crop.y + crop.height;
    drag.current = {
      id: event.pointerId,
      source,
      offset: inside ? { x: point.x - bounded.x, y: point.y - bounded.y } : { x: 0, y: 0 },
    };
    if (!inside) setPosition(clampCenter(point, crop));
  }

  function moveCropDrag(event: PointerEvent<HTMLDivElement>) {
    const tracking = drag.current;
    if (!crop || !dimensions || saving || tracking?.id !== event.pointerId) return;
    if (!(event.buttons & 1)) {
      endCropDrag(event);
      return;
    }
    const pointer = pointAt(event, tracking.source);
    const point = { x: pointer.x - tracking.offset.x, y: pointer.y - tracking.offset.y };
    if (!tracking.resize) {
      setPosition(clampCenter(point, crop));
      return;
    }

    const { anchor, corner } = tracking.resize;
    const largest = cropSize(dimensions, ratio, 1);
    const availableWidth = corner.x > 0 ? 1 - anchor.x : anchor.x;
    const availableHeight = corner.y > 0 ? 1 - anchor.y : anchor.y;
    const maxScale = Math.min(1, availableWidth / largest.width, availableHeight / largest.height);
    const horizontalScale = corner.x * (point.x - anchor.x) / largest.width;
    const verticalScale = corner.y * (point.y - anchor.y) / largest.height;
    // Project the pointer onto the fixed-ratio diagonal in image pixels.
    const widthWeight = ratio * ratio;
    const scale = (horizontalScale * widthWeight + verticalScale) / (widthWeight + 1);
    const boundedScale = Math.max(1 / maxZoom, Math.min(maxScale, scale));

    setZoom(1 / boundedScale);
    setPosition({
      x: anchor.x + corner.x * largest.width * boundedScale / 2,
      y: anchor.y + corner.y * largest.height * boundedScale / 2,
    });
  }

  function endCropDrag(event: PointerEvent<HTMLDivElement>) {
    const tracking = drag.current;
    if (tracking?.id !== event.pointerId) return;
    tracking.source.style.removeProperty('cursor');
    drag.current = undefined;
  }

  async function apply() {
    if (!source || !crop || saveInProgress.current || missing || submitted) return;
    saveInProgress.current = true;
    setSaving(true);
    setError('');
    try {
      const image = await saveImageWidget(source.blob, source.name, crop, source.previous?.source);
      await editWidgets((items) => configureImageWidget(items, widgetId ?? undefined, image));
      setSubmitted(true);
    } catch (error) {
      setError(String(error));
      return;
    } finally {
      saveInProgress.current = false;
      setSaving(false);
    }
    dismiss();
  }

  return <div className="image-editor" data-dropping={dropping}
    onPointerMove={moveCropDrag} onPointerUp={endCropDrag} onPointerCancel={endCropDrag}
    onPointerLeave={(event) => {
      if (!drag.current?.source.hasPointerCapture(event.pointerId)) endCropDrag(event);
    }}
    onDragEnter={(event) => {
      if (!canChoose || !event.dataTransfer.types.includes('Files')) return;
      event.preventDefault();
      dropDepth.current++;
      setDropping(true);
    }}
    onDragLeave={() => {
      dropDepth.current = Math.max(0, dropDepth.current - 1);
      if (dropDepth.current === 0) setDropping(false);
    }}
    onDragOver={(event) => {
      event.preventDefault();
      event.dataTransfer.dropEffect = canChoose && event.dataTransfer.types.includes('Files') ? 'copy' : 'none';
    }}
    onDrop={(event) => {
      event.preventDefault();
      dropDepth.current = 0;
      setDropping(false);
      if (!canChoose) return;
      const { files } = event.dataTransfer;
      if (files.length === 1) choose(files[0]);
      else if (files.length > 1) setError('Choose one image at a time.');
    }}>
    <header className="image-editor-heading" onMouseDown={(event) => {
      if (!desktop || event.button !== 0 || (event.target as Element).closest('button')) return;
      event.preventDefault();
      void invoke('start_image_editor_move').catch((error) => setError(String(error)));
    }}>
      <div>
        <h1>{widgetId ? 'Edit dock image' : 'Add a dock image'}</h1>
        {source && <p title={source.name}>{source.name}</p>}
      </div>
      <button type="button" className="image-editor-close" disabled={saving}
        aria-label="Close image editor" onClick={dismiss}><X size={17} /></button>
    </header>
    <input type="file" ref={filePicker} accept={acceptedImages} hidden onChange={(event) => {
      const file = event.currentTarget.files?.[0];
      event.currentTarget.value = '';
      if (file && canChoose) choose(file);
    }} />
    <div className="image-editor-content" data-cropping={!!crop}>
      <div className="image-editor-canvas">
        <div className="image-crop-stage" data-cropping={!!crop} aria-busy={busy}>
          {url && <div key={url} className="image-crop-source" data-ready={!!crop} tabIndex={saving || !crop ? -1 : 0}
            aria-label="Crop area" aria-describedby="image-crop-instructions"
            style={dimensions ? { '--source-ratio': dimensions.width / dimensions.height } as CSSProperties : undefined}
            onPointerDown={startCropDrag}
            onLostPointerCapture={(event) => {
              if (!(event.buttons & 1)) endCropDrag(event);
            }}
            onKeyDown={(event) => {
              const key = event.key;
              if (!crop || saving || !['ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight'].includes(key)) return;
              event.preventDefault();
              const step = event.shiftKey ? .08 : .02;
              const x = bounded.x + (key === 'ArrowLeft' ? -step : key === 'ArrowRight' ? step : 0);
              const y = bounded.y + (key === 'ArrowUp' ? -step : key === 'ArrowDown' ? step : 0);
              setPosition(clampCenter({ x, y }, crop));
            }}>
            <img src={url} alt="" draggable={false} onLoad={(event) => loadImage(event.currentTarget)}
              onError={() => setError('This image could not be opened. Choose another file.')} />
            {crop && <span className="image-crop-guide" aria-hidden="true" style={{
              left: `${crop.x * 100}%`, top: `${crop.y * 100}%`, width: `${crop.width * 100}%`, height: `${crop.height * 100}%`,
            }}>{cropCorners.map(({ name }) => <i key={name} className="image-crop-handle" data-crop-corner={name} />)}</span>}
          </div>}
          {!crop && <div className="image-crop-empty">
            <span className="image-editor-upload-icon" aria-hidden="true"><ImageUp size={29} strokeWidth={1.55} /></span>
            {busy ? <p role="status">Loading image…</p> : missing ? <p>This image widget is no longer on the dock.</p> : <>
              <h2>{dropping ? 'Drop to open' : 'Drop an image here'}</h2>
              <p>PNG, JPG, GIF, or WebP · Up to 24 MB</p>
              <button type="button" className="image-editor-button" onClick={browse}>Choose image…</button>
            </>}
          </div>}
        </div>
        {crop && <p className="image-crop-instructions" id="image-crop-instructions">Drag a corner to resize. Drag inside or use the arrow keys to move.</p>}
      </div>

      {crop && <aside className="image-editor-controls" aria-label="Crop controls">
        <fieldset className="image-editor-shapes">
          <legend>Tile shape</legend>
          <div className="image-editor-ratios">
            {ratios.map(({ ratio: option, label }) => <button key={option} type="button" aria-pressed={ratio === option}
              aria-label={`${label}, ${option} × 1`} disabled={saving} onClick={() => {
                setPosition(bounded);
                setRatio(option);
              }}>
              <span aria-hidden="true" style={{ width: option * 10 }} />{option} × 1
            </button>)}
          </div>
        </fieldset>
        <div className="image-editor-zoom">
          <div className="image-editor-zoom-label">
            <label htmlFor="image-crop-zoom">Zoom</label>
            <output htmlFor="image-crop-zoom">{zoomLabel}</output>
            <button type="button" className="image-editor-reset" aria-label="Reset crop" title="Reset crop"
              disabled={saving || (zoom === 1 && bounded.x === .5 && bounded.y === .5)}
              onClick={() => { setPosition(center); setZoom(1); }}><RotateCcw size={14} /></button>
          </div>
          <div className="slider-field" style={{ '--progress': `${(zoom - 1) / (maxZoom - 1) * 100}%` } as CSSProperties}>
            <input type="range" id="image-crop-zoom" min={1} max={maxZoom} step=".01" value={zoomPercent / 100} disabled={saving}
              aria-valuetext={zoomLabel} onChange={(event) => {
                setPosition(bounded);
                setZoom(event.currentTarget.valueAsNumber);
              }} />
          </div>
        </div>
        <div className="image-editor-preview">
          <h2>Dock preview</h2>
          <div className="image-editor-preview-well">
            <div className="image-editor-preview-tile" style={{ '--tile-ratio': ratio } as CSSProperties}>
              <img src={url} alt="Preview of the cropped dock tile" draggable={false} style={croppedImageStyle(crop)} />
            </div>
          </div>
        </div>
      </aside>}
    </div>
    {error && <p className="image-editor-error" role="alert">{error}</p>}
    <footer className="image-editor-actions">
      {crop && <button type="button" className="image-editor-button image-editor-replace" disabled={!canChoose}
        onClick={browse}><FolderOpen size={16} aria-hidden="true" />Change image…</button>}
      <button type="button" className="image-editor-button" disabled={saving} onClick={dismiss}>Cancel</button>
      {crop && <button type="button" className="image-editor-button primary" disabled={saving || missing || submitted}
        onClick={() => void apply()}>{saving ? 'Saving…' : widgetId ? 'Save crop' : 'Add to dock'}</button>}
    </footer>
  </div>;
}
