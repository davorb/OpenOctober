// Liquid surfaces of a tile on a grid of square cells (cell centres), used by build_nav.js. Same rules as World.waterAt in js/world.js.
const SEA_SURFACE = 214.84;
const HAZARD = { r_Lava: 1, r_Acid: 2, r_Tar: 3 };   // kind numbers of the .nav file, server/src/nav.rs
const WADE = 1.0;                                     // deepest liquid an NPC walks through

const newGrid = (n, cell, x0, z0) => ({ n, cell, x0, z0, water: new Float32Array(n * n).fill(-1e9), hazard: new Map() });

/** Add one triangle (world coordinates, 9 numbers) of a liquid of resource type `type`. Waterfalls are not surfaces. */
function addTriangle(g, p, type) {
  const bx = p[3] - p[0], by = p[4] - p[1], bz = p[5] - p[2], cx = p[6] - p[0], cy = p[7] - p[1], cz = p[8] - p[2];
  const ny = bz * cx - bx * cz;
  if (ny === 0 || Math.abs(ny) < 0.9 * Math.hypot(by * cz - bz * cy, ny, bx * cy - by * cx)) return;
  const lo = (v, o) => Math.max(0, Math.ceil((v - o) / g.cell - 0.5)), hi = (v, o) => Math.min(g.n - 1, Math.floor((v - o) / g.cell - 0.5));
  const ix1 = hi(Math.max(p[0], p[3], p[6]), g.x0), iz1 = hi(Math.max(p[2], p[5], p[8]), g.z0);
  for (let iz = lo(Math.min(p[2], p[5], p[8]), g.z0); iz <= iz1; iz++) for (let ix = lo(Math.min(p[0], p[3], p[6]), g.x0); ix <= ix1; ix++) {
    const px = g.x0 + (ix + 0.5) * g.cell - p[0], pz = g.z0 + (iz + 0.5) * g.cell - p[2];
    const u = (pz * cx - px * cz) / ny, v = (bz * px - bx * pz) / ny;
    if (u < -1e-4 || v < -1e-4 || u + v > 1.0001) continue;
    const y = p[1] + by * u + cy * v, k = iz * g.n + ix;
    if (y > g.water[k]) g.water[k] = y;
    if (HAZARD[type] && !(g.hazard.get(k)?.y >= y)) g.hazard.set(k, { y, kind: HAZARD[type] });
  }
}

/** Is ground at height `h` in cell `k` too deep (or too hot) for an NPC to walk? */
const isBlocked = (g, k, h) => g.water[k] - h > WADE || (g.hazard.has(k) && g.hazard.get(k).y > h + 0.05);

module.exports = { SEA_SURFACE, HAZARD, newGrid, addTriangle, isBlocked };
