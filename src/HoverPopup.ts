import { invoke } from '@tauri-apps/api/core';
import { useEffect, type RefObject } from 'react';
import { followPopupMotion } from './dockMotion';
import { useSurfacePayload } from './useSurfacePayload';
import { dockPopupAnchor } from './widgets';
import type { WeatherKind } from './widgetCatalog';

export type HoverPopupKind = 'calendar' | 'spotify' | WeatherKind;

export function useHoverPopup(face: RefObject<HTMLElement | null>, widget: string | undefined, kind: HoverPopupKind,
  onError: (error: unknown) => void = console.error) {
  const target = useSurfacePayload<{ id: number; widget: string }>('widget-popup-anchor-changed', 'read_widget_popup_anchor', !!widget);

  useEffect(() => {
    const element = face.current;
    if (!element || !target || target.widget !== widget) return;
    const perimeter = element.closest<HTMLElement>('.dock-body, .folder-panel');
    if (!perimeter) return;
    return followPopupMotion(
      () => element.isConnected && perimeter.isConnected
        ? { anchor: dockPopupAnchor(element), perimeter: dockPopupAnchor(perimeter) }
        : undefined,
      (geometry) => invoke<boolean>('move_widget_popup', { id: target.id, ...geometry }),
    );
  }, [face, target, widget]);

  return (element: HTMLElement, toggle = false, keyboard = false) => {
    const perimeter = element.closest<HTMLElement>('.dock-body, .folder-panel');
    if (!widget || !perimeter) return;
    void invoke('show_widget_popup', {
      kind, widget, anchor: dockPopupAnchor(element), perimeter: dockPopupAnchor(perimeter), toggle, keyboard,
    }).catch(onError);
  };
}
