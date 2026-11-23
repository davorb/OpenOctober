// node --test game/test/*.test.mjs
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createRequire, registerHooks } from 'node:module';

const vendor = p => new URL('../vendor/three/' + p, import.meta.url).href;
registerHooks({
  resolve: (s, c, next) => s === 'three' ? { url: vendor('build/three.module.js'), format: 'module', shortCircuit: true }
    : s.startsWith('three/addons/') ? { url: vendor('examples/jsm/' + s.slice(13)), format: 'module', shortCircuit: true } : next(s, c),
  load: (u, c, next) => /\/game\/(js|vendor)\/.*\.js$/.test(u) ? next(u, { ...c, format: 'module' }) : next(u, c),
});
const THREE = await import('three');
const { World } = await import('../js/world.js');
const { Player } = await import('../js/collision.js');
const { SEA_SURFACE, newGrid, addTriangle, isBlocked } = createRequire(import.meta.url)('../tools/liquids.js');

const rect = { x0: 1024, x1: 2048, z0: 0, z1: 1024 };
const bed = h => new Float32Array(257 * 257).fill(h);
const plane = (size, x, y, z) => { const m = new THREE.Mesh(new THREE.PlaneGeometry(size, size).rotateX(-Math.PI / 2)); m.position.set(x, y, z); return m; };

/** A world of one tile, its objects loaded through the real _loadObjects. */
async function world({ ground, sea = false, zone = 'main', water = [], props = [], layers = null }) {
  const scene = new THREE.Group(), liquids = {};
  water.forEach((m, i) => { m.name = `lake_${i}xml`; liquids[`lake_${i}`] = 'r_Water'; scene.add(m); });
  props.forEach(m => scene.add(m));
  const t = { name: 't', rect, info: { liquids, zone }, sea, heights: bed(ground), layers, coll: null, liquid: null, fadeO: { value: 0 }, oState: 1, uris: [] };
  const w = Object.assign(Object.create(World.prototype), {
    byXZ: new Map([['1,0', t]]), fullTiles: new Set(), sceneObjects: new THREE.Group(), texCache: new Map(),
    gltf: { loadAsync: async () => ({ scene, parser: { json: {}, associations: new Map() } }) },
    water: { material: () => Object.assign(new THREE.MeshBasicMaterial(), { userData: { water: true } }), bind() {} }, _objMaterial: m => m,
  });
  if (water.length || props.length) await w._loadObjects(t);
  return { w, t };
}
function walk(w, y, secs, player = new Player(w)) {
  player.pos.set(1500, y, 500);
  let first = null;
  for (let i = 0; i < secs * 60; i++) { player.update(1 / 60, 0, 0, false); if (player.swim && first === null) first = player.pos.y; }
  return { player, first };
}

test('single-mesh objects keep the placement of their node', async () => {
  const { t } = await world({ ground: 230, water: [plane(100, 1500, 239.5, 500)], props: [plane(4, 1600, 231, 600)] });
  const at = o => { o.geometry.computeBoundingBox(); return o.geometry.boundingBox.clone().applyMatrix4(o.matrixWorld).getCenter(new THREE.Vector3()).toArray(); };
  assert.deepEqual(t.objects.children.map(at).sort((a, b) => a[0] - b[0]), [[1500, 239.5, 500], [1600, 231, 600]]);
});

test('a lake above sea level is swum in, at its own surface', async () => {
  const { w } = await world({ ground: 230, water: [plane(100, 1500, 239.5, 500)] });
  assert.ok(Math.abs(w.waterAt(1500, 500, 240) - 239.5) < 1e-3);
  assert.equal(w.waterAt(1700, 500, 240), -1e9, 'not beside it');
  const { player } = walk(w, 230, 5);
  assert.ok(player.swim && Math.abs(player.pos.y - (239.5 - 1.8)) < 0.1, `floats: ${player.pos.y}`);
});

test('a basin below sea level without water is dry', async () => {
  const { w } = await world({ ground: 44 });
  const { player } = walk(w, 45, 3);
  assert.ok(!player.swim && Math.abs(player.pos.y - 44) < 0.01);
});

test('swimming starts in the water, not above it', async () => {
  const { w } = await world({ ground: 180, sea: true });
  const { player, first } = walk(w, 240, 10);
  assert.ok(first < SEA_SURFACE, `first swam with feet at ${first}`);
  assert.ok(player.swim && Math.abs(player.pos.y - (SEA_SURFACE - 1.8)) < 0.1);
});

test('wading does not swim', async () => {
  const { w } = await world({ ground: SEA_SURFACE - 1.5, sea: true });
  assert.equal(walk(w, SEA_SURFACE - 1.5, 2).player.swim, false);
});

test('the sea does not reach a cave under its bed', async () => {
  const layers = new Float32Array(257 * 257 * 6).fill(-1e9);
  for (let k = 0; k < 257 * 257; k++) { layers[k * 6] = 150; layers[k * 6 + 1] = 200; }
  const { w } = await world({ ground: 200, sea: true, layers });
  assert.equal(w.waterAt(1500, 500, 152), -1e9);
  assert.ok(w.waterAt(1500, 500, 210) > 214);
});

test('nav bake: lakes, shallows, dry ground, waterfalls and lava', () => {
  const g = newGrid(16, 2, 0, 0);
  addTriangle(g, [0, 240, 0, 16, 240, 0, 0, 240, 16], 'r_Water');
  const deep = 1 * 16 + 1, outside = 14 * 16 + 14;
  assert.ok(isBlocked(g, deep, 230)); assert.ok(!isBlocked(g, deep, 239.5)); assert.ok(!isBlocked(g, outside, 44));
  addTriangle(g, [20, 200, 20, 30, 200, 20, 20, 240, 20.5], 'r_Water');
  assert.ok(!g.water.some(y => y > 200 && y < 240), 'waterfall is no surface');
  addTriangle(g, [16, 176, 16, 32, 176, 16, 16, 176, 32], 'r_Lava');
  const lava = 9 * 16 + 9;
  assert.deepEqual(g.hazard.get(lava), { y: 176, kind: 1 });
  assert.ok(isBlocked(g, lava, 175.5), 'even shallow lava'); assert.ok(!isBlocked(g, lava, 177));
});
