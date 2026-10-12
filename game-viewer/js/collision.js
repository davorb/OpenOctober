// Collision for walk mode.
//  - terrain  : 4 m height grid per tile (World.groundHeight)
//  - statics  : per-tile triangle grid (8 m cells) built from the merged object geometry (solid materials only)
//  - trees    : trunk cylinders from the level placements (leaf-card meshes are alpha-tested, so they are skipped above)
import * as THREE from "three";
import { SEA_LEVEL } from "./water.js";

// The architecture is ~1.5-2x human scale (benches have 0.5 m seats, house doors are ~3.7 m), so the player is scaled up to match.
// Base values: r_Player Races in gamelogicinfo/creaturemovementspeedlist.xml (walk 2.5, run 3.7, sprint 5.5 m/s) and worldconfig camera offset 1.54.
export const PLAYER_SCALE = 1.5;
export const SPEEDS = {
  walk: 2.5 * PLAYER_SCALE,
  run: 3.7 * PLAYER_SCALE,
  sprint: 5.5 * PLAYER_SCALE,
  swim: 2.0 * PLAYER_SCALE,
};
// Walk-mode tuning.
export const WALK = {
  // Steepest terrain the player can walk up, as rise over run (tan of the slope angle): 1 = 45 deg, 1.8 = ~61 deg, 2.5 = ~68 deg.
  // A move onto terrain that climbs faster than this from where the player stands is blocked and slides along the slope instead.
  // Raise it to climb steeper hills; lower it if the player scales faces that should be walls. Static geometry (stairs) ignores it.
  maxSlope: 2.5,
  slopeSlack: 0.04, // extra rise (m) tolerated per move, so rounding on near-limit slopes doesn't stick the player
};
const CELL = 4,
  N = 256; // dense 256x256 grid of 4 m cells per tile (CSR layout)

export function newColl(tile) {
  return {
    n: 0,
    tris: [],
    x0: tile.rect.x0,
    z0: tile.rect.z0,
    f32: null,
    start: null,
    list: null,
  };
}
// Collect triangles of one merged geometry into the tile's collision set.
export function addCollision(coll, geo) {
  const p = geo.attributes.position.array,
    ix = geo.index ? geo.index.array : null,
    n = ix ? ix.length : p.length / 3;
  for (let t = 0; t < n; t += 3) {
    const a = (ix ? ix[t] : t) * 3,
      b = (ix ? ix[t + 1] : t + 1) * 3,
      c = (ix ? ix[t + 2] : t + 2) * 3;
    coll.tris.push(
      p[a],
      p[a + 1],
      p[a + 2],
      p[b],
      p[b + 1],
      p[b + 2],
      p[c],
      p[c + 1],
      p[c + 2],
    );
  }
}
const cellOf = (coll, v, o) =>
  Math.max(0, Math.min(N - 1, Math.floor((v - o) / CELL)));
export function finishColl(coll) {
  const A = (coll.f32 = new Float32Array(coll.tris));
  coll.tris = null;
  const T = A.length / 9;
  const start = new Int32Array(N * N + 1),
    range = new Int32Array(T * 4);
  for (let t = 0; t < T; t++) {
    const i = t * 9,
      minx = Math.min(A[i], A[i + 3], A[i + 6]),
      maxx = Math.max(A[i], A[i + 3], A[i + 6]),
      minz = Math.min(A[i + 2], A[i + 5], A[i + 8]),
      maxz = Math.max(A[i + 2], A[i + 5], A[i + 8]);
    const x0 = cellOf(coll, minx, coll.x0),
      x1 = cellOf(coll, maxx, coll.x0),
      z0 = cellOf(coll, minz, coll.z0),
      z1 = cellOf(coll, maxz, coll.z0);
    if ((x1 - x0 + 1) * (z1 - z0 + 1) > 160) {
      range[t * 4] = -1;
      continue;
    } // huge slabs: terrain handles those
    range[t * 4] = x0;
    range[t * 4 + 1] = x1;
    range[t * 4 + 2] = z0;
    range[t * 4 + 3] = z1;
    for (let z = z0; z <= z1; z++)
      for (let x = x0; x <= x1; x++) start[z * N + x + 1]++;
  }
  for (let k = 0; k < N * N; k++) start[k + 1] += start[k];
  const list = new Int32Array(start[N * N]),
    fill = start.slice(0, N * N);
  for (let t = 0; t < T; t++) {
    if (range[t * 4] < 0) continue;
    for (let z = range[t * 4 + 2]; z <= range[t * 4 + 3]; z++)
      for (let x = range[t * 4]; x <= range[t * 4 + 1]; x++)
        list[fill[z * N + x]++] = t;
  }
  coll.start = start;
  coll.list = list;
}
// triangle ids in the cell containing (x, z): returns [from, to) into coll.list
function cellRange(coll, x, z) {
  const k = cellOf(coll, z, coll.z0) * N + cellOf(coll, x, coll.x0);
  return [coll.start[k], coll.start[k + 1]];
}

const tmp = new THREE.Vector3();
// closest point on triangle (Ericson, Real-Time Collision Detection)
function closest(px, py, pz, A, i, out) {
  const ax = A[i],
    ay = A[i + 1],
    az = A[i + 2],
    bx = A[i + 3],
    by = A[i + 4],
    bz = A[i + 5],
    cx = A[i + 6],
    cy = A[i + 7],
    cz = A[i + 8];
  const abx = bx - ax,
    aby = by - ay,
    abz = bz - az,
    acx = cx - ax,
    acy = cy - ay,
    acz = cz - az;
  const apx = px - ax,
    apy = py - ay,
    apz = pz - az;
  const d1 = abx * apx + aby * apy + abz * apz,
    d2 = acx * apx + acy * apy + acz * apz;
  if (d1 <= 0 && d2 <= 0) return out.set(ax, ay, az);
  const bpx = px - bx,
    bpy = py - by,
    bpz = pz - bz,
    d3 = abx * bpx + aby * bpy + abz * bpz,
    d4 = acx * bpx + acy * bpy + acz * bpz;
  if (d3 >= 0 && d4 <= d3) return out.set(bx, by, bz);
  const vc = d1 * d4 - d3 * d2;
  if (vc <= 0 && d1 >= 0 && d3 <= 0) {
    const v = d1 / (d1 - d3);
    return out.set(ax + abx * v, ay + aby * v, az + abz * v);
  }
  const cpx = px - cx,
    cpy = py - cy,
    cpz = pz - cz,
    d5 = abx * cpx + aby * cpy + abz * cpz,
    d6 = acx * cpx + acy * cpy + acz * cpz;
  if (d6 >= 0 && d5 <= d6) return out.set(cx, cy, cz);
  const vb = d5 * d2 - d1 * d6;
  if (vb <= 0 && d2 >= 0 && d6 <= 0) {
    const w = d2 / (d2 - d6);
    return out.set(ax + acx * w, ay + acy * w, az + acz * w);
  }
  const va = d3 * d6 - d5 * d4;
  if (va <= 0 && d4 - d3 >= 0 && d5 - d6 >= 0) {
    const w = (d4 - d3) / (d4 - d3 + (d5 - d6));
    return out.set(bx + (cx - bx) * w, by + (cy - by) * w, bz + (cz - bz) * w);
  }
  const den = 1 / (va + vb + vc),
    v = vb * den,
    w = vc * den;
  return out.set(
    ax + abx * v + acx * w,
    ay + aby * v + acy * w,
    az + abz * v + acz * w,
  );
}

export class Player {
  constructor(world) {
    this.world = world;
    this.pos = new THREE.Vector3();
    this.vy = 0;
    this.onGround = false;
    this.swim = false;
    const S = PLAYER_SCALE;
    this.radius = 0.45 * S;
    this.height = 1.8 * S;
    this.eye = 1.7 * S;
    this.step = 0.65 * S;
  }
  // tiles whose collision data may touch (x, z)
  _tiles(x, z, r) {
    const out = [],
      w = this.world;
    for (const dx of [-r, r])
      for (const dz of [-r, r]) {
        const t = w.tileAt(x + dx, z + dz);
        if (t && t.coll && !out.includes(t)) out.push(t);
      }
    return out;
  }
  // highest walkable surface under (x,z) that is not above maxY (terrain or static geometry)
  floor(x, z, maxY) {
    let best = this.world.groundHeight(x, z, maxY);
    if (best == null) best = -1e9;
    if (best > maxY) best = -1e9;
    for (const t of this._tiles(x, z, 0.3)) {
      const [r0, r1] = cellRange(t.coll, x, z),
        A = t.coll.f32,
        L = t.coll.list;
      for (let q = r0; q < r1; q++) {
        const i = L[q] * 9;
        if (Math.min(A[i + 1], A[i + 4], A[i + 7]) > maxY) continue;
        const ax = A[i],
          az = A[i + 2],
          bx = A[i + 3],
          bz = A[i + 5],
          cx = A[i + 6],
          cz = A[i + 8];
        const den = (bz - cz) * (ax - cx) + (cx - bx) * (az - cz);
        if (Math.abs(den) < 1e-6) continue;
        const l1 = ((bz - cz) * (x - cx) + (cx - bx) * (z - cz)) / den,
          l2 = ((cz - az) * (x - cx) + (ax - cx) * (z - cz)) / den,
          l3 = 1 - l1 - l2;
        if (l1 < 0 || l2 < 0 || l3 < 0) continue;
        // face must point up-ish (either winding: use the geometric normal's |y|)
        const e1x = bx - ax,
          e1y = A[i + 4] - A[i + 1],
          e1z = bz - az,
          e2x = cx - ax,
          e2y = A[i + 7] - A[i + 1],
          e2z = cz - az;
        const nx = e1y * e2z - e1z * e2y,
          ny = e1z * e2x - e1x * e2z,
          nz = e1x * e2y - e1y * e2x,
          nl = Math.hypot(nx, ny, nz) || 1;
        if (Math.abs(ny) / nl < 0.45) continue;
        const y = l1 * A[i + 1] + l2 * A[i + 4] + l3 * A[i + 7];
        if (y <= maxY && y > best) best = y;
      }
    }
    return best;
  }
  // push the player capsule (two spheres) out of walls / trunks
  _slide(p) {
    const r = this.radius;
    for (let iter = 0; iter < 3; iter++) {
      let hit = false;
      for (const h of [0.55 * PLAYER_SCALE, 1.35 * PLAYER_SCALE]) {
        const y = p.y + h;
        for (const t of this._tiles(p.x, p.z, r + 0.2)) {
          const A = t.coll.f32;
          const C = t.coll,
            L = C.list,
            gx0 = cellOf(C, p.x - r, C.x0),
            gx1 = cellOf(C, p.x + r, C.x0),
            gz0 = cellOf(C, p.z - r, C.z0),
            gz1 = cellOf(C, p.z + r, C.z0);
          for (let cz = gz0; cz <= gz1; cz++)
            for (let cx = gx0; cx <= gx1; cx++) {
              const k = cz * N + cx;
              for (let q = C.start[k]; q < C.start[k + 1]; q++) {
                const i = L[q] * 9;
                const lo = Math.min(A[i + 1], A[i + 4], A[i + 7]),
                  hi = Math.max(A[i + 1], A[i + 4], A[i + 7]);
                if (hi < y - r || lo > y + r) continue;
                const e1x = A[i + 3] - A[i],
                  e1y = A[i + 4] - A[i + 1],
                  e1z = A[i + 5] - A[i + 2],
                  e2x = A[i + 6] - A[i],
                  e2y = A[i + 7] - A[i + 1],
                  e2z = A[i + 8] - A[i + 2];
                const ny = e1z * e2x - e1x * e2z,
                  nl =
                    Math.hypot(
                      e1y * e2z - e1z * e2y,
                      ny,
                      e1x * e2y - e1y * e2x,
                    ) || 1;
                if (Math.abs(ny) / nl > 0.6) continue; // floors / ramps are handled by floor()
                closest(p.x, y, p.z, A, i, tmp);
                const dx = p.x - tmp.x,
                  dy = y - tmp.y,
                  dz = p.z - tmp.z,
                  d = Math.hypot(dx, dy, dz);
                if (d < r && d > 1e-5) {
                  const k = (r - d) / d;
                  p.x += dx * k;
                  p.z += dz * k;
                  hit = true;
                }
              }
            }
        }
      }
      if (!hit) break;
    }
  }
  // wish: world-space horizontal velocity (m/s)
  update(dt, wishX, wishZ, jump) {
    const p = this.pos,
      w = this.world;
    // horizontal move with terrain-slope / step limit, then wall resolution
    const nx = p.x + wishX * dt,
      nz = p.z + wishZ * dt,
      step = Math.hypot(wishX, wishZ) * dt;
    const reach = p.y + this.step + 0.4;
    // a position is walkable if there is a floor within reach AND the terrain doesn't climb faster than WALK.maxSlope from where we stand
    // (the old 1.3*step+0.4 slack let the player scale near-vertical faces, then pop through them when the floor query lost it)
    // t0 and t1 must use the same height cap: groundHeight drops grid corners above the cap, so a lower cap on t0 biases it low
    // on a plain slope (uphill corners excluded) and made every step look like a ~0.4 m cliff.
    const t0 = w.groundHeight(p.x, p.z, reach);
    const ok = (x, z) => {
      if (this.floor(x, z, reach) < -1e8) return false;
      const d = Math.hypot(x - p.x, z - p.z);
      if (d < 1e-6 || t0 == null || t0 < -1e8) return true;
      const t1 = w.groundHeight(x, z, reach);
      return (
        t1 == null ||
        t1 < -1e8 ||
        t1 - t0 <= WALK.maxSlope * d + WALK.slopeSlack
      );
    };
    // probe the leading edge of the body too, so the camera/torso never ends up inside a rising slope
    const lead = (x, z) => {
      const l = Math.hypot(wishX, wishZ) || 1,
        k = this.radius / l;
      return ok(x + wishX * k, z + wishZ * k);
    };
    if (ok(nx, nz) && lead(nx, nz)) {
      p.x = nx;
      p.z = nz;
    } else {
      if (ok(nx, p.z) && lead(nx, p.z)) p.x = nx;
      else if (ok(p.x, nz) && lead(p.x, nz)) p.z = nz;
    }
    this._slide(p);
    // vertical
    let fl = this.floor(p.x, p.z, p.y + this.step + 0.4);
    const th = w.groundHeight(p.x, p.z);
    if (th == null) {
      this.vy = 0;
      return;
    } // tile not streamed in yet: hold still
    if (fl < -1e8) {
      const f = w.groundHeight(p.x, p.z, p.y + 30);
      fl = p.y = f > -1e8 ? f : th;
      this.vy = 0;
    } // pushed below the surface: pop back up to the nearest floor
    const water = SEA_LEVEL - 0.2;
    this.swim =
      fl < water - 1.3 * PLAYER_SCALE &&
      fl > -1e8 &&
      !w.covered(p.x, p.y + this.eye, p.z); // no swimming up through a cavern roof
    if (this.swim) {
      this.vy = 0;
      p.y += (water - 1.2 * PLAYER_SCALE - p.y) * Math.min(1, dt * 5);
      this.onGround = false;
    } else {
      this.vy -= 9.81 * dt;
      if (this.vy < -55) this.vy = -55;
      let ny = p.y + this.vy * dt;
      if (fl > -1e8 && ny <= fl + 0.02 && this.vy <= 0) {
        ny = fl;
        this.vy = 0;
        this.onGround = true;
      } else if (
        this.onGround &&
        fl > -1e8 &&
        p.y - fl < 0.7 &&
        this.vy <= 0.0001
      ) {
        ny = fl;
        this.vy = 0;
      } // glue to slopes going down
      else this.onGround = false;
      if (jump && this.onGround) {
        this.vy = 4.4 * Math.sqrt(PLAYER_SCALE);
        ny = p.y + this.vy * dt;
        this.onGround = false;
      }
      p.y = ny;
      // ceiling
    }
  }
}
