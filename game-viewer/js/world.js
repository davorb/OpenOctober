// Streams the world tile by tile so it reads as one continuous landscape.
//  - terrain  : <tile>_empty.glb parsed in workers -> splat-blended terrain, loaded out to the fog horizon
//  - objects  : <tile>.glb (props, trees, rocks, buildings, water planes), loaded around the camera; terrain node replaced by the above
// Everything fades up out of the fog as it arrives so nothing pops.
import * as THREE from 'three';
import { GLTFLoader } from 'three/addons/loaders/GLTFLoader.js';
import { buildTerrain } from './terrain.js';
import { mergeGeometries } from 'three/addons/utils/BufferGeometryUtils.js';
import { kindOf } from './water.js';
import { fogUniforms, fogFn } from './chunks.js';
import { newColl, addCollision, finishColl } from './collision.js';

const OBJ_DIR = '/converted/maps/';

export class World {
  constructor({ scene, env, tex, water, tiles, names, props = {}, opts = {} }) {
    this.scene = scene; this.env = env; this.tex = tex; this.water = water;
    this.opts = { terrainRadius: 3600, objectRadius: 1500, workers: 3, ...opts };
    this.tiles = new Map();
    for (const [name, t] of Object.entries(tiles)) {
      // game (x, z) -> three (x, -z)
      this.tiles.set(name, { name, info: t, display: names[name] || name,
        rect: { x0: t.x0, x1: t.x1, z0: -t.z1, z1: -t.z0 }, hasTerrain: t.empty > 0, hasObjects: t.base > 330000,
        hasWater: !!t.liquids, tState: 0, oState: 0, terrain: null, objects: null, fade: { value: 0 }, fadeO: { value: 0 }, heights: null, dist: 1e9,
        lights: props[name]?.l?.length ? new Float32Array(props[name].l) : null, trees: props[name]?.t?.length ? new Float32Array(props[name].t) : null, coll: null });
    }
    this.byXZ = new Map(); for (const t of this.tiles.values()) this.byXZ.set(Math.round(t.rect.x0 / 1024) + ',' + Math.round(t.rect.z0 / 1024), t);
    this.workers = Array.from({ length: this.opts.workers }, () => ({ w: new Worker('/js/terrain-worker.js'), busy: null }));
    this.workers.forEach(wk => { wk.w.onmessage = e => this._terrainDone(wk, e.data); });
    this.qT = []; this.qM = []; this.meshBusy = false; this.objBusy = false; this.gltf = new GLTFLoader();
    this.texCache = new Map();   // uri -> {tex, refs}
    this.sceneObjects = new THREE.Group(); scene.add(this.sceneObjects);
    this.objMats = new Map();
    this.timer = 0; this.stats = { terrain: 0, objects: 0, loading: 0 };
    this.group = new THREE.Group(); scene.add(this.group);
  }

  tileAt(x, z) { return this.byXZ.get(Math.floor(x / 1024) + ',' + Math.floor(z / 1024)); }
  groundHeight(x, z) {
    const t = this.tileAt(x, z); if (!t || !t.heights) return null;
    const G = 256, c = 1024 / G, fx = (x - (t.rect.x0 + 512)) / c + G / 2, fz = (z - (t.rect.z0 + 512)) / c + G / 2;
    const x0 = Math.max(0, Math.min(G - 1, Math.floor(fx))), z0 = Math.max(0, Math.min(G - 1, Math.floor(fz))), tx = fx - x0, tz = fz - z0, h = t.heights, W = G + 1;
    const a = h[z0 * W + x0], b = h[z0 * W + x0 + 1], d = h[(z0 + 1) * W + x0], e = h[(z0 + 1) * W + x0 + 1];
    return (a * (1 - tx) + b * tx) * (1 - tz) + (d * (1 - tx) + e * tx) * tz;
  }

  rectDist(t, x, z) { const r = t.rect, dx = Math.max(r.x0 - x, 0, x - r.x1), dz = Math.max(r.z0 - z, 0, z - r.z1); return Math.hypot(dx, dz); }

  update(dt, cam) {
    // fades
    for (const t of this.tiles.values()) {
      if (t.terrain && t.fade.value < 1) t.fade.value = Math.min(1, t.fade.value + dt / 1.6);
      if (t.objects && t.fadeO.value < 1) t.fadeO.value = Math.min(1, t.fadeO.value + dt / 1.2);
    }
    this.timer -= dt; if (this.timer > 0) { this._pump(); return; }
    this.timer = 0.2;
    const { terrainRadius: TR, objectRadius: OR } = this.opts;
    const cx = cam.position.x, cz = cam.position.z;
    let nT = 0, nO = 0;
    for (const t of this.tiles.values()) {
      const d = t.dist = this.rectDist(t, cx, cz);
      if (t.hasTerrain) {
        if (t.tState === 0 && d < TR) { t.tState = 1; (t.info.terrain === 'mesh' ? this.qM : this.qT).push(t); }
        else if (t.tState === 2 && d > TR + 600) this._dropTerrain(t);
        if (t.tState === 2) nT++;
      }
      if (t.hasObjects || t.hasWater) {
        const need = d < OR;
        if (t.oState === 0 && need) t.oState = 1;           // queued (picked in _pump by distance)
        else if (t.oState === 1 && !need && d > OR + 300) t.oState = 0;
        else if (t.oState === 2 && d > OR + 500) this._dropObjects(t);
        if (t.oState === 2) nO++;
      }
    }
    this.qT.sort((a, b) => a.dist - b.dist); this.qM.sort((a, b) => a.dist - b.dist);
    this.stats.terrain = nT; this.stats.objects = nO;
    this._pump();
  }

  _pump() {
    // terrain: hand tiles to idle workers, nearest first
    for (const wk of this.workers) {
      if (wk.busy) continue;
      let t; while ((t = this.qT.shift()) && t.tState !== 1) { /* skip cancelled */ }
      if (!t) break;
      wk.busy = t; wk.w.postMessage({ id: t.name, url: OBJ_DIR + t.name + '_empty.glb' });
    }
    if (!this.meshBusy) { let t; while ((t = this.qM.shift()) && t.tState !== 1) { /* skip cancelled */ } if (t) this._loadMeshTerrain(t); }
    // objects: one at a time, nearest first
    if (!this.objBusy) {
      let best = null; for (const t of this.tiles.values()) if (t.oState === 1 && (!best || t.dist < best.dist)) best = t;
      if (best) this._loadObjects(best);
    }
    this.stats.loading = this.qT.length + this.workers.filter(w => w.busy).length + (this.objBusy ? 1 : 0);
  }

  _terrainDone(wk, data) {
    const t = wk.busy; wk.busy = null;
    if (!t || data.error) { if (t) t.tState = 4; if (data.error) console.warn('terrain', data.id, data.error); this._pump(); return; }
    if (t.tState !== 1) { this._pump(); return; }       // cancelled while loading
    t.heights = data.heights;
    t.terrain = buildTerrain(data, this.tex, this.env, t.fade);
    this.group.add(t.terrain); t.tState = 2;
    this._pump();
  }
  async _loadMeshTerrain(t) {
    this.meshBusy = true;
    try {
      const gltf = await this.gltf.loadAsync(OBJ_DIR + t.name + '_empty.glb');
      if (t.tState !== 1) { this._disposeScene(gltf.scene); this.meshBusy = false; return; }
      const root = gltf.scene, seen = new Map();
      root.traverse(o => {
        if (!o.isMesh) return; o.matrixAutoUpdate = false;
        const mats = Array.isArray(o.material) ? o.material : [o.material], out = [];
        for (const src of mats) { if (!seen.has(src)) { if (src.map) src.map = this._sharedTexture(src.map, gltf.parser.associations, gltf.parser.json, t); seen.set(src, this._objMaterial(src, t.fade)); } out.push(seen.get(src)); }
        o.material = Array.isArray(o.material) ? out : out[0];
      });
      root.updateMatrixWorld(true);
      t.terrain = root; t.terrain.userData.dispose = () => this._disposeScene(root); this.group.add(root); t.tState = 2;
    } catch (e) { console.warn('mesh terrain', t.name, e); t.tState = 4; }
    this.meshBusy = false;
  }
  _dropTerrain(t) { if (t.terrain) { this.group.remove(t.terrain); t.terrain.userData.dispose(); t.terrain = null; } t.heights = null; t.tState = 0; t.fade.value = 0; for (const u of t.turis || []) { /* shared textures released with objects */ } }

  // ---- objects
  _objMaterial(src, fade) {
    // MeshStandardMaterial from glTF -> cheaper Lambert with the game's fog
    const m = new THREE.MeshLambertMaterial({ map: src.map, color: src.color, side: src.side, alphaTest: src.alphaTest, transparent: src.transparent, opacity: src.opacity, emissive: src.emissive, emissiveMap: src.emissiveMap, vertexColors: src.vertexColors });
    m.name = src.name;
    m.customProgramCacheKey = () => 'dfobj';
    m.onBeforeCompile = sh => {
      Object.assign(sh.uniforms, { uDynV: this.env.u.uDynV, uDynCol: this.env.u.uDynCol, uDynRad: this.env.u.uDynRad, uFogColor: this.env.u.uFogColor, uFogRange: this.env.u.uFogRange, uFogDensity: this.env.u.uFogDensity, uSunDirView: this.env.u.uSunDirView, uSunColor: this.env.u.uSunColor, uFade: fade });
      sh.fragmentShader = 'uniform float uFade; uniform vec3 uDynV[16]; uniform vec3 uDynCol[16]; uniform float uDynRad[16];\n' + fogUniforms + fogFn + sh.fragmentShader
        .replace('#include <tonemapping_fragment>', 'gl_FragColor.rgb = dfFog(gl_FragColor.rgb, -vViewPosition, uFade);\n#include <tonemapping_fragment>')
        .replace('#include <fog_fragment>', '')
        .replace('#include <opaque_fragment>', `{ vec3 dyn = vec3(0.0); vec3 vp = -vViewPosition;
          for (int i = 0; i < 16; i++) { if (uDynRad[i] <= 0.0) continue; vec3 d = uDynV[i] - vp; float dd = length(d); float att = clamp(1.0 - dd / uDynRad[i], 0.0, 1.0); dyn += uDynCol[i] * att * att * (max(dot(normal, d / dd), 0.0) * 0.8 + 0.2); }
          outgoingLight += diffuseColor.rgb * dyn * 1.6; }
        #include <opaque_fragment>`);
    };
    return m;
  }

  async _loadObjects(t) {
    this.objBusy = true; t.oState = 3;
    try {
      const gltf = await this.gltf.loadAsync(OBJ_DIR + t.name + '.glb');
      if (t.oState !== 3) { this._disposeScene(gltf.scene); this.objBusy = false; return; }
      const root = gltf.scene, json = gltf.parser.json, assoc = gltf.parser.associations;
      const liquids = t.info.liquids || {};
      for (const n of [...root.children]) {
        let isTerrain = false; n.traverse(o => { if (o.userData && o.userData.groups) isTerrain = true; });
        if (isTerrain) { root.remove(n); this._disposeScene(n); continue; }   // terrain comes from the worker path
        const base = (n.name || '').replace(/xml(_\d+)?$/, '');                // GLTFLoader strips the '.' from node names
        if (liquids[base]) n.userData.liquid = liquids[base];
      }
      const seen = new Map();
      root.traverse(o => {
        if (!o.isMesh) return;
        o.matrixAutoUpdate = false;
        let top = o; while (top.parent && top.parent !== root) top = top.parent;
        if (top.userData.liquid) { o.material = this.water.material(kindOf(top.userData.liquid), t.fadeO); o.renderOrder = 2; return; }
        const mats = Array.isArray(o.material) ? o.material : [o.material], out = [];
        for (const src of mats) {
          if (!seen.has(src)) {
            if (src.map) src.map = this._sharedTexture(src.map, assoc, json, t);
            seen.set(src, this._objMaterial(src, t.fadeO));
          }
          out.push(seen.get(src));
        }
        o.material = Array.isArray(o.material) ? out : out[0];
      });
      root.updateMatrixWorld(true);
      this._mergeStatic(root, t);
      t.objects = root; this.sceneObjects.add(root); t.oState = 2;
    } catch (e) { console.warn('objects', t.name, e); t.oState = 0; }
    this.objBusy = false;
  }
  // One draw call per material per tile instead of one per placed object
  _mergeStatic(root, tile) {
    const coll = newColl(tile), buckets = new Map(), keep = [];
    root.traverse(o => {
      if (!o.isMesh) return;
      if (o.material.userData?.water || Array.isArray(o.material) || o.material.vertexColors || !o.geometry.attributes.position) { keep.push(o); return; }
      let g = o.geometry.clone();
      for (const k of Object.keys(g.attributes)) if (!['position', 'normal', 'uv'].includes(k)) g.deleteAttribute(k);
      const n = g.attributes.position.count;
      if (!g.attributes.normal) g.setAttribute('normal', new THREE.BufferAttribute(new Float32Array(n * 3), 3));
      if (!g.attributes.uv) g.setAttribute('uv', new THREE.BufferAttribute(new Float32Array(n * 2), 2));
      if (!g.index) g = g.toNonIndexed();
      g.applyMatrix4(o.matrixWorld);
      (buckets.get(o.material) || buckets.set(o.material, []).get(o.material)).push(g);
    });
    // detach everything, then add merged meshes + untouched ones
    const kept = keep.map(o => ({ o, m: o.matrixWorld.clone() }));
    root.clear();
    for (const [mat, geos] of buckets) {
      const idx = geos.filter(g => g.index).length;
      const list = idx && idx !== geos.length ? geos.map(g => g.index ? g.toNonIndexed() : g) : geos;
      const merged = mergeGeometries(list, false); if (!merged) continue;
      merged.computeBoundingSphere();
      const m = new THREE.Mesh(merged, mat); m.matrixAutoUpdate = false; m.castShadow = true; m.receiveShadow = true; root.add(m);
      if (!mat.alphaTest && !mat.transparent) addCollision(coll, merged);
      for (const g of geos) g.dispose();
    }
    finishColl(coll); tile.coll = coll;
    for (const { o, m } of kept) { root.add(o); o.matrix.copy(m); o.matrixAutoUpdate = false; o.matrixWorld.copy(m); }
  }
  _sharedTexture(tex, assoc, json, t) {
    const a = assoc.get(tex); const img = a && json.textures?.[a.textures]?.source;
    const uri = img != null ? json.images?.[img]?.uri : null;
    if (!uri) return tex;
    let e = this.texCache.get(uri);
    if (!e) { tex.anisotropy = 4; e = { tex, refs: 0 }; this.texCache.set(uri, e); }
    else if (e.tex !== tex) tex.dispose();
    e.refs++; (t.uris ||= []).push(uri);
    return e.tex;
  }
  _disposeScene(root) {
    root.traverse(o => { o.geometry?.dispose(); const ms = o.material ? (Array.isArray(o.material) ? o.material : [o.material]) : []; for (const m of ms) m.dispose(); });
  }
  _dropObjects(t) {
    if (t.objects) { this.sceneObjects.remove(t.objects); this._disposeScene(t.objects); t.objects = null; }
    for (const u of t.uris || []) { const e = this.texCache.get(u); if (e && --e.refs <= 0) { e.tex.dispose(); this.texCache.delete(u); } }
    t.uris = []; t.oState = 0; t.fadeO.value = 0; t.coll = null;
  }
}
