export function magnification(count: number, position: number | undefined, size: number, zoom: number, falloff: number, room: number, inset: number, alignment: 'start' | 'center' | 'end' = 'center') {
  const gains = Array.from({ length: count }, (_, index) => {
    if (position === undefined) return 0;
    const distance = Math.abs(index - position) / (falloff + 0.5);
    const focus = Math.max(0, 1 - distance ** 2);
    return size * (zoom - 1) * focus ** 2;
  });
  const desired = gains.reduce((sum, width) => sum + width, 0);
  const fit = desired > 0 ? Math.min(1, Math.max(0, room) / desired) : 0;
  let spread = 0;
  const icons = gains.map((gain) => {
    const width = gain * fit;
    const shift = spread + width / 2;
    spread += width;
    return { scale: 1 + width / size, shift };
  });
  const center = Math.min(count - 1, Math.max(0, position ?? 0));
  const before = Math.floor(center);
  const left = icons[before]?.shift ?? 0;
  const right = icons[before + 1]?.shift ?? left;
  const anchor = left + (right - left) * (center - before);
  const start = alignment === 'start' ? 0 : alignment === 'end' ? spread : Math.max(spread - inset, Math.min(inset, anchor), 0);
  return { icons: icons.map((icon) => ({ ...icon, shift: icon.shift - start })), start, end: spread - start };
}

type PopupRect = Pick<DOMRect, 'left' | 'top' | 'right' | 'bottom'>;
interface PopupGeometry { anchor: PopupRect; perimeter?: PopupRect }

function samePixels(first?: PopupRect, second?: PopupRect) {
  if (!first || !second) return first === second;
  const pixel = (value: number) => Math.round(value * devicePixelRatio);
  return pixel(first.left) === pixel(second.left) && pixel(first.top) === pixel(second.top)
    && pixel(first.right) === pixel(second.right) && pixel(first.bottom) === pixel(second.bottom);
}

export function followPopupMotion<Geometry extends PopupGeometry>(
  measure: () => Geometry | undefined,
  move: (geometry: Geometry) => Promise<boolean>,
  initial?: Geometry,
) {
  let previous = initial;
  let active = true;
  let pending = false;
  const follow = () => {
    if (!active) return;
    const next = measure();
    if (!next) return;
    if (!pending && (!samePixels(previous?.anchor, next.anchor) || !samePixels(previous?.perimeter, next.perimeter))) {
      previous = next;
      pending = true;
      void move(next).then((open) => {
        if (!open) active = false;
      }, (error) => {
        if (active) console.error(error);
        active = false;
      }).finally(() => { pending = false; });
    }
    frame = requestAnimationFrame(follow);
  };
  let frame = requestAnimationFrame(follow);
  return () => { active = false; cancelAnimationFrame(frame); };
}
