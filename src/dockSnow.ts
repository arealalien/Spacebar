export interface EffectRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface SnowShape {
  width: number;
  height: number;
  radius: number;
  outline?: readonly (number | null)[];
}

export interface SnowLedge extends EffectRect {
  id: string;
  dock?: boolean;
  shape: SnowShape;
}

interface Snowbank {
  ledge: SnowLedge;
  depth: number[];
  capacity: number[];
  shake: number;
  path?: string;
}

interface SnowPoint {
  x: number;
  top: number;
  base: number;
}

export interface Snowflake {
  id: number;
  x: number;
  y: number;
  dx: number;
  dy: number;
  radius: number;
  born: number;
  drift: number;
  shedFrom?: string;
}

export interface SnowParticles {
  added: Snowflake[];
  removed: number[];
}

const maxFlakes = 150;
export const flakeLifetime = 6;
let lastFlakeId = 0;

export function flakePosition(flake: Snowflake, now: number) {
  const age = Math.max(0, (now - flake.born) / 1000);
  const accelerating = Math.min(age, (68 - flake.dy) / 55);
  const x = flake.x + flake.dx * age + (Math.cos(flake.drift) - Math.cos(flake.drift + age * 2.6)) * (9 / 2.6);
  const y = flake.y + flake.dy * accelerating + 27.5 * accelerating ** 2 + (age - accelerating) * 68;
  return { x, y, age };
}

function bankLimit(ledge: SnowLedge) {
  return ledge.dock ? 18 : Math.min(8, Math.max(3.5, ledge.shape.height * .15));
}

function ledgeTop(ledge: SnowLedge, fraction: number) {
  const { width, height, radius, outline } = ledge.shape;
  const corner = Math.min(radius, width / 2, height / 2);
  const x = fraction * width;
  const shoulder = Math.max(0, corner - Math.min(x, width - x));
  if (!ledge.dock && shoulder > corner * .88) return;
  const rounded = corner - Math.sqrt(corner * corner - shoulder * shoulder);
  if (!outline) return rounded + 1.2;

  const column = fraction * outline.length;
  const index = Math.min(outline.length - 1, Math.floor(column));
  const row = outline[index];
  if (row === null) return;
  const offset = column - index - .5;
  const neighbor = outline[index + Math.sign(offset)] ?? row;
  const contour = row + (neighbor - row) * Math.abs(offset);
  return Math.max(rounded, contour * height) + 1.2;
}

function newBank(ledge: SnowLedge): Snowbank {
  const count = Math.min(140, Math.max(9, Math.round(ledge.shape.width / (ledge.dock ? 5 : 3.5)) + 1));
  const phase = Math.random() * Math.PI * 2;
  const thickness = ledge.dock ? 13.5 : Math.min(6.3, Math.max(2.7, ledge.shape.height * .116));
  const depth = Array.from({ length: count }, (_, index) => {
    const fraction = index / (count - 1);
    if (ledgeTop(ledge, fraction) === undefined) return 0;
    const distance = Math.min(fraction, 1 - fraction);
    const taper = Math.min(1, ledge.dock ? .36 + distance * ledge.shape.width / 12 : .2 + distance * 5);
    const drift = Math.sin(index * .58 + phase) + Math.sin(index * .21 - phase) * .65;
    return taper * (thickness + drift * (ledge.dock ? 1.4 : .65));
  });
  const capacity = depth.map((value) => Math.min(bankLimit(ledge), value * 1.26 + .25));
  return { ledge, depth, capacity, shake: 0 };
}

function snowStripPath(points: readonly SnowPoint[]) {
  if (points.length < 2) return '';
  const point = (x: number, y: number) => x.toFixed(1) + ' ' + y.toFixed(1);
  const first = points[0];
  const start = point(first.x, (first.base + first.top) / 2);
  let path = 'M' + start;
  for (let index = 1; index < points.length; index++) {
    const from = points[index - 1];
    const to = points[index];
    path += 'Q' + point(from.x, from.top) + ' ' + point((from.x + to.x) / 2, (from.top + to.top) / 2);
  }
  const last = points[points.length - 1];
  path += 'Q' + point(last.x, last.top) + ' ' + point(last.x, (last.base + last.top) / 2);
  for (let index = points.length - 1; index > 0; index--) {
    const from = points[index];
    const to = points[index - 1];
    path += 'Q' + point(from.x, from.base) + ' ' + point((from.x + to.x) / 2, (from.base + to.base) / 2);
  }
  return path + 'Q' + point(first.x, first.base) + ' ' + start + 'Z';
}

export function panelSnowPath(width: number, headroom: number) {
  const count = Math.max(9, Math.min(140, Math.ceil(width / 8) + 1));
  const points = Array.from({ length: count }, (_, index) => {
    const x = width * index / (count - 1);
    const distance = Math.min(x, width - x);
    const taper = Math.min(1, .25 + distance / 11);
    const drift = Math.cos(x * .052 + .4) * 1.05 + Math.sin(x * .117) * .82;
    const base = headroom + .65;
    return { x, top: base - (10.25 + drift) * taper, base };
  });
  return snowStripPath(points);
}

function bankPath({ ledge, depth }: Snowbank) {
  const points: SnowPoint[] = [];
  let path = '';
  function finish() {
    path += snowStripPath(points);
    points.length = 0;
  }
  for (let index = 0; index < depth.length; index++) {
    const fraction = index / (depth.length - 1);
    const support = ledgeTop(ledge, fraction);
    if (support === undefined || depth[index] < .2) finish();
    else {
      const overhang = ledge.dock ? (fraction === 0 ? -2 : fraction === 1 ? 2 : 0) : 0;
      const underside = Math.max(-depth[index] / 3, Math.sin(index * .63) * .65);
      points.push({ x: fraction * ledge.shape.width + overhang, base: support + underside, top: support - depth[index] });
    }
  }
  finish();
  return path;
}

export class DockSnow {
  private banks = new Map<string, Snowbank>();
  private flakes = new Map<number, Snowflake>();
  private added: Snowflake[] = [];
  private removed: number[] = [];
  private spawnCredit = 0;
  private spillIn = .6;
  private spillSide: -1 | 1 = -1;

  get particles(): ReadonlyMap<number, Snowflake> { return this.flakes; }

  setLedges(ledges: readonly SnowLedge[], moving: boolean, now: number) {
    const previous = this.banks;
    const next = new Map<string, Snowbank>();
    for (const ledge of ledges) {
      const bank = previous.get(ledge.id);
      if (!bank) {
        next.set(ledge.id, newBank(ledge));
        continue;
      }
      const old = bank.ledge;
      const dx = ledge.x + (ledge.width - old.width) / 2 - old.x;
      const dy = ledge.y - old.y;
      const motion = Math.abs(dx) + Math.abs(dy) * 1.2 + Math.abs(ledge.width - old.width) * .38;
      if (old.shape !== ledge.shape) bank.path = undefined;
      bank.ledge = ledge;
      next.set(ledge.id, bank);
      if (!moving || motion < .2) continue;
      bank.shake += motion;
      const threshold = ledge.dock ? 10 : 5;
      if (bank.shake < threshold) continue;
      bank.shake %= threshold;
      if (ledge.dock) {
        this.spill(bank, dx > .65 ? -1 : dx < -.65 ? 1 : this.spillSide, now);
        continue;
      }
      const side = dx > .65 ? .23 : dx < -.65 ? .77 : .25 + Math.random() * .5;
      const fraction = side + (Math.random() - .5) * .18;
      const index = Math.floor(fraction * bank.depth.length);
      this.shed(bank, index, dx, dy, now);
    }
    if (moving) {
      for (const [id, bank] of previous) {
        if (next.has(id)) continue;
        const step = Math.max(1, Math.floor(bank.depth.length / 8));
        for (let index = 0; index < bank.depth.length; index += step) this.shed(bank, index, 0, 0, now);
      }
    } else {
      this.clearFlakes();
    }
    this.banks = next;
  }

  paintCap(id: string, element: SVGSVGElement) {
    const bank = this.banks.get(id);
    const path = element.lastElementChild;
    if (!bank || !path) return;
    const { width, height } = bank.ledge.shape;
    const viewBox = '0 0 ' + width + ' ' + height;
    if (element.getAttribute('viewBox') !== viewBox) element.setAttribute('viewBox', viewBox);
    bank.path ??= bankPath(bank);
    if (path.getAttribute('d') !== bank.path) path.setAttribute('d', bank.path);
  }

  step(seconds: number, now: number, area: EffectRect, vertical: boolean) {
    const banks = [...this.banks.values()];
    if (!banks.length) return;
    const dockBank = this.banks.get('dock');
    if (dockBank) {
      this.spillIn -= seconds;
      if (this.spillIn <= 0) {
        this.spill(dockBank, this.spillSide, now);
        this.spillSide = this.spillSide === -1 ? 1 : -1;
        this.spillIn = .65 + Math.random() * .75;
      }
    }
    const density = Math.min(36, Math.max(11, (vertical ? area.height : area.width) / 19));
    this.spawnCredit += seconds * density;
    while (this.spawnCredit >= 1) {
      this.spawnCredit--;
      const source = vertical ? banks[Math.floor(Math.random() * banks.length)].ledge : area;
      this.spawn({
        x: source.x - 14 + Math.random() * (source.width + 28),
        y: source.y - 24 - Math.random() * 39,
        dx: (Math.random() - .42) * 16, dy: 22 + Math.random() * 21,
        radius: .7 + Math.random() * 1.55, born: now, drift: Math.random() * 8,
      });
    }

    for (const flake of this.flakes.values()) {
      const position = flakePosition(flake, now);
      if (position.age > flakeLifetime || position.y > area.y + area.height + 34
        || position.x < area.x - 54 || position.x > area.x + area.width + 54) {
        this.removeFlake(flake.id);
        continue;
      }
      const bottom = position.y + flake.radius;
      const before = flakePosition(flake, now - seconds * 1000).y + flake.radius;
      if (bottom < before) continue;
      let contact: { bank: Snowbank; index: number; y: number } | undefined;
      for (const bank of banks) {
        const ledge = bank.ledge;
        const fraction = (position.x - ledge.x) / ledge.width;
        if (fraction < 0 || fraction >= 1 || flake.shedFrom === ledge.id) continue;
        const top = ledgeTop(ledge, fraction);
        if (top === undefined) continue;
        const index = Math.round(fraction * (bank.depth.length - 1));
        const y = ledge.y + (top - bank.depth[index]) * ledge.height / ledge.shape.height;
        if (before <= y + 1 && bottom >= y && (!contact || y < contact.y)) contact = { bank, index, y };
      }
      if (!contact) continue;
      const { bank, index } = contact;
      for (let cell = Math.max(0, index - 1); cell <= Math.min(index + 1, bank.depth.length - 1); cell++) {
        const capacity = Math.min(bank.capacity[cell], bankLimit(bank.ledge));
        const depth = Math.min(capacity, bank.depth[cell] + flake.radius * (cell === index ? .75 : .27));
        if (depth !== bank.depth[cell]) {
          bank.depth[cell] = depth;
          bank.path = undefined;
        }
      }
      this.removeFlake(flake.id);
    }
  }

  clearFlakes() {
    for (const id of this.flakes.keys()) this.removeFlake(id);
    this.spawnCredit = 0;
    this.spillIn = .6;
  }

  takeChanges(): SnowParticles | undefined {
    if (!this.added.length && !this.removed.length) return;
    const changes = { added: this.added, removed: this.removed };
    this.added = [];
    this.removed = [];
    return changes;
  }

  private shed(bank: Snowbank, index: number, dx: number, dy: number, now: number) {
    const fraction = index / (bank.depth.length - 1);
    const top = ledgeTop(bank.ledge, fraction);
    if (top === undefined || bank.depth[index] < 1) return;
    const radius = Math.min(2.8, .9 + bank.depth[index] * .24);
    const { ledge } = bank;
    const heightScale = ledge.height / ledge.shape.height;
    this.spawn({
      x: ledge.x + fraction * ledge.width, y: ledge.y + (top - bank.depth[index]) * heightScale,
      dx: -dx * 11 + (Math.random() - .5) * 42,
      dy: Math.max(-47, Math.min(36, dy * 7 - 15)),
      radius, born: now, drift: Math.random() * 8, shedFrom: ledge.id,
    });
    const limit = bankLimit(ledge);
    for (let cell = Math.max(0, index - 1); cell <= Math.min(index + 1, bank.depth.length - 1); cell++) {
      const removed = radius * (cell === index ? .72 : .23);
      bank.depth[cell] = Math.min(limit, Math.max(0, bank.depth[cell] - removed));
    }
    bank.path = undefined;
  }

  private spill(bank: Snowbank, side: -1 | 1, now: number) {
    const { ledge, depth } = bank;
    const fraction = side === -1 ? 0 : 1;
    const tip = depth[side === -1 ? 0 : depth.length - 1];
    const rim = ledgeTop(ledge, fraction) ?? 0;
    this.spawn({
      x: ledge.x + fraction * ledge.width + side,
      y: ledge.y + (rim - tip / 2) * ledge.height / ledge.shape.height,
      dx: side * (18 + Math.random() * 16), dy: 10 + Math.random() * 19,
      radius: 1.1 + Math.random() * 1.5, born: now, drift: Math.random() * 8, shedFrom: ledge.id,
    });
  }

  private spawn(flake: Omit<Snowflake, 'id'>) {
    if (this.flakes.size >= maxFlakes) return;
    const particle = { ...flake, id: ++lastFlakeId };
    this.flakes.set(particle.id, particle);
    this.added.push(particle);
  }

  private removeFlake(id: number) {
    this.flakes.delete(id);
    this.removed.push(id);
  }
}
