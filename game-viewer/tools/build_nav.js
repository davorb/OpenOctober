#!/usr/bin/env node
// Bakes walkability data for the game-server's NPCs, one file per land tile: game-server/data/nav/<tile>.nav
//
//   node tools/build_nav.js [tile ...]        (default: every tile that has land)
//
// Inputs (converted/maps): <tile>_empty.glb = terrain only, <tile>.glb / <tile>_full.glb = the placed objects of the tile (same choice as the viewer).
// Output per tile (little endian):
//   "DNAV" u32 version=1 | i32 x0 | i32 z0 (tile corner, wire space: the viewer's three.js x / z) | u16 G=256 | u16 N=512
//   heights  (G+1)^2 u16, row-major by z then x, 4 m spacing, value = (h + 500) * 20 (so 5 cm steps)
//   flags    N*N u8,      row-major by z then x, 2 m cells: 1 = too steep, 2 = water, 4 = wall of a solid object at body height
// Terrain heights are rasterised from the terrain triangles exactly like game-viewer/js/terrain-worker.js (4 m grid, topmost surface).
// Walls are the near-vertical triangles of solid objects (same solid/alpha-tested decision as World._shaderIsSolid) whose vertical span
// overlaps the band between 0.6 m and 3 m above the ground. Floors, roofs, trees (alpha-tested leaves) and tree trunks are not obstacles here.
const fs = require('fs'), path = require('path');

const root = path.resolve(__dirname, '../..');
const MAPS = path.join(root, 'converted', 'maps'), OUT = path.join(root, 'game-server', 'data', 'nav');
const DATA = path.join(root, 'game-viewer', 'data');
const G = 256, N = 512, CELL = 1024 / N, W = G + 1;
const SEA_LEVEL = 215, MAX_SLOPE = 1.2, BODY_LO = 0.6, BODY_HI = 3.0;
const F_STEEP = 1, F_WATER = 2, F_WALL = 4;

const alpha = JSON.parse(fs.readFileSync(path.join(DATA, 'alphamode.json'), 'utf8'));
const solidAll = new Set(alpha.all || []), solidSub = new Set(alpha.solid || []), solidKeep = new Set(alpha.keep || []);
const shaderIsSolid = name => {
  const n = (name || '').toLowerCase();
  if (!n || solidKeep.has(n)) return false;
  return solidAll.has(n.replace(/_\d+$/, '')) || solidSub.has(n);
};

function readGlb(file) {
  const buf = fs.readFileSync(file), jlen = buf.readUInt32LE(12);
  const json = JSON.parse(buf.subarray(20, 20 + jlen).toString('utf8')), bin = 20 + jlen + 8;
  const acc = i => {
    const a = json.accessors[i], bv = json.bufferViews[a.bufferView];
    const comps = { SCALAR: 1, VEC3: 3, VEC4: 4 }[a.type];
    const T = { 5126: Float32Array, 5125: Uint32Array, 5123: Uint16Array, 5121: Uint8Array }[a.componentType];
    const off = buf.byteOffset + bin + (bv.byteOffset || 0) + (a.byteOffset || 0);
    const raw = new Uint8Array(a.count * comps * T.BYTES_PER_ELEMENT); raw.set(new Uint8Array(buf.buffer, off, raw.length));
    return new T(raw.buffer);
  };
  return { json, acc };
}

/** 4 m height grid of a terrain GLB (topmost surface per grid point, holes filled from their neighbours). */
function terrainHeights(file) {
  const { json, acc } = readGlb(file);
  const mesh = json.meshes[0], P = acc(mesh.primitives[0].attributes.POSITION), cell = 1024 / G;
  const hg = new Float32Array(W * W).fill(-1e9);
  for (const prim of mesh.primitives) {
    const ix = acc(prim.indices);
    for (let t = 0; t < ix.length; t += 3) {
      const a = ix[t] * 3, b = ix[t + 1] * 3, c = ix[t + 2] * 3;
      const ax = (P[a] + 512) / cell, az = (P[a + 2] + 512) / cell, bx = (P[b] + 512) / cell, bz = (P[b + 2] + 512) / cell, cx = (P[c] + 512) / cell, cz = (P[c + 2] + 512) / cell;
      const x0 = Math.max(0, Math.ceil(Math.min(ax, bx, cx))), x1 = Math.min(G, Math.floor(Math.max(ax, bx, cx)));
      const z0 = Math.max(0, Math.ceil(Math.min(az, bz, cz))), z1 = Math.min(G, Math.floor(Math.max(az, bz, cz)));
      const den = (bz - cz) * (ax - cx) + (cx - bx) * (az - cz); if (Math.abs(den) < 1e-9) continue;
      for (let z = z0; z <= z1; z++) for (let x = x0; x <= x1; x++) {
        const l1 = ((bz - cz) * (x - cx) + (cx - bx) * (z - cz)) / den, l2 = ((cz - az) * (x - cx) + (ax - cx) * (z - cz)) / den, l3 = 1 - l1 - l2;
        if (l1 < -1e-4 || l2 < -1e-4 || l3 < -1e-4) continue;
        const y = l1 * P[a + 1] + l2 * P[b + 1] + l3 * P[c + 1], k = z * W + x;
        if (y > hg[k]) hg[k] = y;
      }
    }
  }
  for (let pass = 0; pass < 12; pass++) {
    let holes = 0; const src = hg.slice();
    for (let z = 0; z <= G; z++) for (let x = 0; x <= G; x++) {
      const k = z * W + x; if (src[k] > -1e8) continue; let sum = 0, n = 0;
      for (let dz = -1; dz <= 1; dz++) for (let dx = -1; dx <= 1; dx++) { const xx = x + dx, zz = z + dz; if (xx < 0 || zz < 0 || xx > G || zz > G) continue; const v = src[zz * W + xx]; if (v > -1e8) { sum += v; n++; } }
      if (n) hg[k] = sum / n; else holes++;
    }
    if (!holes) break;
  }
  const t = json.nodes[0].translation || [0, 0, 0];
  return { hg, node: t };
}

/** Terrain height at tile-local metres (0..1024) by bilinear interpolation of the 4 m grid. */
const heightAt = (hg, lx, lz) => {
  const fx = Math.min(G - 1e-6, Math.max(0, lx / (1024 / G))), fz = Math.min(G - 1e-6, Math.max(0, lz / (1024 / G)));
  const x0 = Math.floor(fx), z0 = Math.floor(fz), tx = fx - x0, tz = fz - z0;
  const a = hg[z0 * W + x0], b = hg[z0 * W + x0 + 1], c = hg[(z0 + 1) * W + x0], d = hg[(z0 + 1) * W + x0 + 1];
  return (a * (1 - tx) + b * tx) * (1 - tz) + (c * (1 - tx) + d * tx) * tz;
};

/** Mark the cells touched by the wall triangle (world coords p0..p2) whose y span overlaps the body band above the ground. */
function markWall(flags, hg, x0, z0, p) {
  const ymin = Math.min(p[1], p[4], p[7]), ymax = Math.max(p[1], p[4], p[7]);
  const xs = [p[0], p[3], p[6]], zs = [p[2], p[5], p[8]];
  const mark = (x, z) => {
    const lx = x - x0, lz = z - z0; if (lx < 0 || lz < 0 || lx >= 1024 || lz >= 1024) return;
    const g = heightAt(hg, lx, lz);
    if (ymax > g + BODY_LO && ymin < g + BODY_HI) flags[Math.floor(lz / CELL) * N + Math.floor(lx / CELL)] |= F_WALL;
  };
  // along the edges (a vertical wall is a line in plan) ...
  for (let e = 0; e < 3; e++) {
    const f = e, t = (e + 1) % 3, len = Math.hypot(xs[t] - xs[f], zs[t] - zs[f]), steps = Math.max(1, Math.ceil(len / 0.7));
    for (let s = 0; s <= steps; s++) mark(xs[f] + (xs[t] - xs[f]) * s / steps, zs[f] + (zs[t] - zs[f]) * s / steps);
  }
  // ... and across the footprint of steeper-but-not-vertical faces (rocks, cliffs)
  const bx0 = Math.min(...xs), bx1 = Math.max(...xs), bz0 = Math.min(...zs), bz1 = Math.max(...zs);
  if ((bx1 - bx0) * (bz1 - bz0) > 1 && bx1 - bx0 < 60 && bz1 - bz0 < 60) {
    const den = (zs[1] - zs[2]) * (xs[0] - xs[2]) + (xs[2] - xs[1]) * (zs[0] - zs[2]);
    if (Math.abs(den) > 1e-6) for (let z = bz0; z <= bz1; z += 1) for (let x = bx0; x <= bx1; x += 1) {
      const l1 = ((zs[1] - zs[2]) * (x - xs[2]) + (xs[2] - xs[1]) * (z - zs[2])) / den, l2 = ((zs[2] - zs[0]) * (x - xs[2]) + (xs[0] - xs[2]) * (z - zs[2])) / den;
      if (l1 >= 0 && l2 >= 0 && l1 + l2 <= 1) mark(x, z);
    }
  }
}

function rotate(q, v, out) {
  const [qx, qy, qz, qw] = q, [x, y, z] = v;
  const tx = 2 * (qy * z - qz * y), ty = 2 * (qz * x - qx * z), tz = 2 * (qx * y - qy * x);
  out[0] = x + qw * tx + (qy * tz - qz * ty); out[1] = y + qw * ty + (qz * tx - qx * tz); out[2] = z + qw * tz + (qx * ty - qy * tx);
}

function bakeTile(name, tile, useFull) {
  const { hg } = terrainHeights(path.join(MAPS, name + '_empty.glb'));
  let land = 0; for (const h of hg) if (h > SEA_LEVEL - 1) land++;
  if (land < 4) return null; // open sea
  const x0 = tile.x0, z0 = -tile.z1; // wire space: the viewer's z is the game's z negated
  const flags = new Uint8Array(N * N);
  // terrain: too steep, too deep to wade
  for (let cz = 0; cz < N; cz++) for (let cx = 0; cx < N; cx++) {
    const lx = (cx + 0.5) * CELL, lz = (cz + 0.5) * CELL, h = heightAt(hg, lx, lz);
    const dx = (heightAt(hg, lx + CELL, lz) - heightAt(hg, lx - CELL, lz)) / (2 * CELL), dz = (heightAt(hg, lx, lz + CELL) - heightAt(hg, lx, lz - CELL)) / (2 * CELL);
    let f = 0;
    if (Math.hypot(dx, dz) > MAX_SLOPE) f |= F_STEEP;
    if (h < SEA_LEVEL - 1.0) f |= F_WATER;
    flags[cz * N + cx] = f;
  }
  // solid objects: wall triangles at body height
  const file = path.join(MAPS, name + (useFull ? '_full' : '') + '.glb');
  let walls = 0;
  if (fs.existsSync(file)) {
    const { json, acc } = readGlb(file), meshes = new Map(), q = [0, 0, 0], v = [0, 0, 0];
    const prims = mi => {
      let r = meshes.get(mi); if (r) return r; r = [];
      for (const p of json.meshes[mi].primitives) {
        const m = json.materials?.[p.material], mode = m?.alphaMode;
        if ((mode === 'MASK' || mode === 'BLEND') && !shaderIsSolid(m?.name)) continue; // foliage, banners, glass: not an obstacle
        if (p.mode !== undefined && p.mode !== 4) continue;
        r.push({ P: acc(p.attributes.POSITION), ix: p.indices !== undefined ? acc(p.indices) : null });
      }
      meshes.set(mi, r); return r;
    };
    const tri = new Float64Array(9);
    for (const node of json.nodes) {
      if (node.mesh === undefined) continue;
      const t = node.translation || [0, 0, 0], r = node.rotation || [0, 0, 0, 1], sc = node.scale || [1, 1, 1];
      for (const { P, ix } of prims(node.mesh)) {
        const n = ix ? ix.length : P.length / 3;
        for (let k = 0; k < n; k += 3) {
          for (let c = 0; c < 3; c++) {
            const vi = (ix ? ix[k + c] : k + c) * 3;
            v[0] = P[vi] * sc[0]; v[1] = P[vi + 1] * sc[1]; v[2] = P[vi + 2] * sc[2];
            rotate(r, v, q);
            tri[c * 3] = q[0] + t[0]; tri[c * 3 + 1] = q[1] + t[1]; tri[c * 3 + 2] = q[2] + t[2];
          }
          const ux = tri[3] - tri[0], uy = tri[4] - tri[1], uz = tri[5] - tri[2], wx = tri[6] - tri[0], wy = tri[7] - tri[1], wz = tri[8] - tri[2];
          const nx = uy * wz - uz * wy, ny = uz * wx - ux * wz, nz = ux * wy - uy * wx, len = Math.hypot(nx, ny, nz);
          if (len < 1e-9 || Math.abs(ny) / len > 0.6) continue; // floors, roofs and gentle slopes are walkable surfaces, not walls
          markWall(flags, hg, x0, z0, tri); walls++;
        }
      }
    }
  }
  // file
  const head = Buffer.alloc(20); head.write('DNAV', 0); head.writeUInt32LE(1, 4); head.writeInt32LE(x0, 8); head.writeInt32LE(z0, 12); head.writeUInt16LE(G, 16); head.writeUInt16LE(N, 18);
  const hq = new Uint16Array(W * W); for (let i = 0; i < hq.length; i++) hq[i] = Math.max(0, Math.min(65535, Math.round((hg[i] + 500) * 20)));
  const blocked = flags.reduce((a, f) => a + (f ? 1 : 0), 0);
  return { buf: Buffer.concat([head, Buffer.from(hq.buffer), Buffer.from(flags.buffer)]), stats: { walls, blocked, cells: N * N } };
}

const world = JSON.parse(fs.readFileSync(path.join(DATA, 'world.json'), 'utf8'));
const fullTiles = new Set(JSON.parse(fs.readFileSync(path.join(DATA, 'fulltiles.json'), 'utf8')));
const wanted = process.argv.slice(2), names = (wanted.length ? wanted : Object.keys(world)).filter(n => world[n] && fs.existsSync(path.join(MAPS, n + '_empty.glb')));
fs.mkdirSync(OUT, { recursive: true });
let done = 0, skipped = 0; const t0 = Date.now();
for (const name of names) {
  const r = bakeTile(name, world[name], fullTiles.has(name));
  if (!r) { skipped++; continue; }
  fs.writeFileSync(path.join(OUT, name + '.nav'), r.buf);
  done++;
  console.log(`${name}: ${r.stats.walls} wall tris, ${(100 * r.stats.blocked / r.stats.cells).toFixed(1)}% blocked  [${done + skipped}/${names.length}, ${((Date.now() - t0) / 1000).toFixed(0)} s]`);
}
console.log(`${done} tiles baked, ${skipped} open sea skipped -> ${OUT}`);
