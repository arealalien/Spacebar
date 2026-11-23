import { invoke } from '@tauri-apps/api/core';
import { desktop } from './model';

export type ImageRatio = 1 | 2 | 3;
export interface ImageCrop {
  x: number;
  y: number;
  width: number;
  height: number;
  ratio: ImageRatio;
}
export interface ImageWidgetOptions {
  asset: string;
  source: string;
  name: string;
  crop: ImageCrop;
}

function browserImages<T>(mode: IDBTransactionMode, operation: (store: IDBObjectStore) => IDBRequest<T>) {
  return new Promise<T>((resolve, reject) => {
    const open = indexedDB.open('spacebar-images', 1);
    open.onupgradeneeded = () => open.result.createObjectStore('images');
    open.onerror = () => reject(open.error);
    open.onsuccess = () => {
      const database = open.result;
      const transaction = database.transaction('images', mode);
      const request = operation(transaction.objectStore('images'));
      transaction.oncomplete = () => { database.close(); resolve(request.result); };
      transaction.onabort = () => { database.close(); reject(transaction.error); };
    };
  });
}

export async function readImageWidget(id: string, original: boolean) {
  if (desktop) {
    const bytes = await invoke<ArrayBuffer | number[]>('read_widget_image', { id, original });
    return new Blob([bytes instanceof ArrayBuffer ? bytes : Uint8Array.from(bytes)], original ? undefined : { type: 'image/png' });
  }
  const blob = await browserImages<Blob | undefined>('readonly', (store) => store.get(id));
  if (!blob) throw new Error('The saved image could not be found.');
  return blob;
}

export async function saveImageWidget(blob: Blob, name: string, crop: ImageCrop, previousSource?: string): Promise<ImageWidgetOptions> {
  const safeName = Array.from(name).slice(0, 255).join('');
  if (desktop) {
    const headers: Record<string, string> = { 'x-spacebar-crop': JSON.stringify(crop) };
    if (previousSource) headers['x-spacebar-source'] = previousSource;
    const bytes = previousSource ? new Uint8Array() : new Uint8Array(await blob.arrayBuffer());
    const image = await invoke<Pick<ImageWidgetOptions, 'asset' | 'source'>>('save_widget_image', bytes, { headers });
    return { ...image, name: safeName, crop };
  }
  const source = previousSource ?? crypto.randomUUID();
  if (!previousSource) await browserImages('readwrite', (store) => store.put(blob, source));
  return { asset: source, source, name: safeName, crop };
}
