import * as THREE from 'three';
import { GLTFLoader } from 'three/addons/loaders/GLTFLoader.js';
import { OrbitControls } from 'three/addons/controls/OrbitControls.js';

const DATA = 'data';
const $ = id => document.getElementById(id);

// ───────────────────────── catalog / list ─────────────────────────
const TABS = [
  { id: 'maps', label: 'Maps', dir: 'maps' },
  { id: 'models', label: 'Models', dir: 'models' },
  { id: 'animated', label: 'Characters', dir: 'animated' },
  { id: 'terrain', label: 'Terrain', dir: 'terrain' },
];
let catalog = null;
let tab = 'maps';
let filtered = [];
let current = null; // { tab, name, variant }
const ROW_H = 28;

const fmtSize = b => b > 1048576 ? (b / 1048576).toFixed(1) + ' MB' : Math.max(1, Math.round(b / 1024)) + ' KB';

function entryName(e) { return e[0]; }
function entrySize(e) {
  if (tab === 'maps') return Math.max(...Object.values(e[1]));
  return e[1];
}

function buildTabs() {
  const el = $('tabs');
  el.innerHTML = '';
  for (const t of TABS) {
    const b = document.createElement('button');
    b.className = 'tab' + (t.id === tab ? ' on' : '');
    b.textContent = t.label;
    b.onclick = () => { tab = t.id; $('q').value = ''; buildTabs(); applyFilter(); $('list').scrollTop = 0; };
    el.appendChild(b);
  }
}

function applyFilter() {
  const terms = $('q').value.toLowerCase().split(/\s+/).filter(Boolean);
  filtered = catalog[tab].filter(e => terms.every(t => e[0].toLowerCase().includes(t)));
  $('count').textContent = `${filtered.length.toLocaleString()} of ${catalog[tab].length.toLocaleString()}`;
  $('spacer').style.height = filtered.length * ROW_H + 'px';
  renderRows();
}

function renderRows() {
  const list = $('list'), sp = $('spacer');
  const first = Math.max(0, Math.floor(list.scrollTop / ROW_H) - 5);
  const last = Math.min(filtered.length, Math.ceil((list.scrollTop + list.clientHeight) / ROW_H) + 5);
  const frag = document.createDocumentFragment();
  for (let i = first; i < last; i++) {
    const e = filtered[i];
    const r = document.createElement('div');
    r.className = 'row' + (current && current.tab === tab && current.name === e[0] ? ' on' : '');
    r.style.top = i * ROW_H + 'px';
    r.innerHTML = `<span class="n"></span><span class="s">${fmtSize(entrySize(e))}</span>`;
    r.firstChild.textContent = e[0];
    r.title = e[0];
    r.onclick = () => select(tab, e[0]);
    frag.appendChild(r);
  }
  sp.replaceChildren(frag);
}

$('list').addEventListener('scroll', renderRows);
window.addEventListener('resize', renderRows);
$('q').addEventListener('input', () => { applyFilter(); $('list').scrollTop = 0; });

// ───────────────────────── three.js setup ─────────────────────────
const canvas = $('cv');
const renderer = new THREE.WebGLRenderer({ canvas, antialias: true, preserveDrawingBuffer: false });
renderer.setPixelRatio(Math.min(devicePixelRatio, 2));
const scene = new THREE.Scene();
const camera = new THREE.PerspectiveCamera(55, 1, 0.1, 1000);
const controls = new OrbitControls(camera, canvas);
controls.enableDamping = true;
controls.dampingFactor = 0.12;
controls.screenSpacePanning = true;

const hemi = new THREE.HemisphereLight(0xffffff, 0x667766, 1.7);
const sun = new THREE.DirectionalLight(0xfff4e0, 1.6);
sun.position.set(0.5, 1, 0.35);
scene.add(hemi, sun);
const grid = new THREE.GridHelper(1, 20, 0x666666, 0x333333);
grid.visible = false;
scene.add(grid);

const BGS = { sky: 0x87a7c4, grey: 0x6b6f76, dark: 0x14161a };
function setBg() { scene.background = new THREE.Color(BGS[$('bg').value]); }
$('bg').onchange = setBg; setBg();

function resize() {
  const m = $('main');
  renderer.setSize(m.clientWidth, m.clientHeight, false);
  camera.aspect = m.clientWidth / m.clientHeight;
  camera.updateProjectionMatrix();
}
new ResizeObserver(resize).observe($('main'));

// ───────────────────────── terrain shader ─────────────────────────
// Each terrain tile primitive blends up to 7 terrain textures using per-vertex
// 8-slot weights (_WEIGHTS0/1). `extras.groups[i]` lists textures + weight slots.
const MAXT = 7;
const terrainUniformsShared = {
  uSunDir: { value: sun.position.clone().normalize() },
  uSunCol: { value: new THREE.Color(0xfff4e0).multiplyScalar(1.1) },
  uSky: { value: new THREE.Color(0xffffff).multiplyScalar(0.95) },
  uGround: { value: new THREE.Color(0x667766).multiplyScalar(0.75) },
  uScale: { value: 1 / 16 },
  uTint: { value: 1 },
};
const sampDecl = Array.from({ length: MAXT }, (_, i) => `uniform sampler2D t${i};`).join('\n');
const blendCode = Array.from({ length: MAXT }, (_, i) =>
  `if (${i} < uCount) { float w = slotW(int(uSlot[${i}])); acc += texture2D(t${i}, uv).rgb * w; sum += w; if (${i} == 0) first = texture2D(t0, uv).rgb; }`).join('\n  ');
const terrainVS = `
attribute vec4 w0; attribute vec4 w1; attribute vec4 vcol;
varying vec4 vW0; varying vec4 vW1; varying vec3 vWorld; varying vec3 vN; varying vec3 vC;
void main() {
  vW0 = w0; vW1 = w1; vC = vcol.rgb;
  vec4 wp = modelMatrix * vec4(position, 1.0);
  vWorld = wp.xyz;
  vN = normalize(mat3(modelMatrix) * normal);
  gl_Position = projectionMatrix * viewMatrix * wp;
}`;
const terrainFS = `
${sampDecl}
uniform float uSlot[${MAXT}]; uniform int uCount; uniform float uScale; uniform float uTint;
uniform vec3 uSunDir; uniform vec3 uSunCol; uniform vec3 uSky; uniform vec3 uGround;
varying vec4 vW0; varying vec4 vW1; varying vec3 vWorld; varying vec3 vN; varying vec3 vC;
float slotW(int s) { return s < 4 ? vW0[s] : vW1[s - 4]; }
void main() {
  vec2 uv = vWorld.xz * uScale;
  vec3 acc = vec3(0.0); vec3 first = vec3(0.5); float sum = 0.0;
  ${blendCode}
  vec3 col = sum > 1e-4 ? acc / sum : first;
  col *= mix(vec3(1.0), clamp(pow(vC, vec3(2.2)) * 3.0, 0.0, 1.5), uTint);
  vec3 n = normalize(vN);
  vec3 light = mix(uGround, uSky, n.y * 0.5 + 0.5) + uSunCol * max(dot(n, uSunDir), 0.0);
  gl_FragColor = vec4(col * light * 0.62, 1.0);
  #include <colorspace_fragment>
}`;

const texLoader = new THREE.TextureLoader();
const terrainTexCache = new Map();
function terrainTexture(ddsName) {
  const file = ddsName.replace(/\.dds$/i, '.png');
  if (!terrainTexCache.has(file)) {
    const t = texLoader.load(`${DATA}/textures/${encodeURIComponent(file)}`, undefined, undefined, () => missing.add(file));
    t.wrapS = t.wrapT = THREE.RepeatWrapping;
    t.colorSpace = THREE.SRGBColorSpace;
    t.anisotropy = renderer.capabilities.getMaxAnisotropy();
    terrainTexCache.set(file, t);
  }
  return terrainTexCache.get(file);
}

const blank = new THREE.DataTexture(new Uint8Array([128, 128, 128, 255]), 1, 1);
blank.needsUpdate = true;

function makeTerrainMaterial(group) {
  // some material ids have no texture name (null); drop them, keeping each texture paired with its weight slot
  const pairs = group.textures.map((t, i) => [t, group.slots[i]]).filter(([t, s]) => t && s != null).slice(0, MAXT);
  const uniforms = {
    ...terrainUniformsShared,
    uCount: { value: pairs.length },
    uSlot: { value: Array.from({ length: MAXT }, (_, i) => pairs[i]?.[1] ?? 0) },
  };
  for (let i = 0; i < MAXT; i++)
    uniforms['t' + i] = { value: pairs[i] ? terrainTexture(pairs[i][0]) : blank };
  return new THREE.ShaderMaterial({ vertexShader: terrainVS, fragmentShader: terrainFS, uniforms, side: THREE.DoubleSide });
}

// The tile mesh is split into groups by material set and vertices aren't shared between groups, so a vertex on a
// group boundary only carries weights for its own group's materials: the neighbour's materials are dropped and
// the transition becomes a hard, triangle-shaped edge. Fix: merge weights of coincident vertices (world space, so
// it also works across tiles), then give every group the union of materials its vertices touch.
const comp = (a, i, c) => [a.getX, a.getY, a.getZ, a.getW][c].call(a, i);
function stitchTerrain(meshes) {
  const key = (v, m) => { v.applyMatrix4(m); return Math.round(v.x * 50) + ',' + Math.round(v.y * 50) + ',' + Math.round(v.z * 50); };
  const matTex = new Map();
  const field = new Map(); // position key -> Map(materialId -> weight)
  const per = [];
  const v = new THREE.Vector3();
  for (const { o, grp } of meshes) {
    o.updateWorldMatrix(true, false);
    const g = o.geometry, pos = g.getAttribute('position'), w0 = g.getAttribute('w0'), w1 = g.getAttribute('w1');
    grp.materials.forEach((m, i) => { if (grp.textures[i]) matTex.set(m, grp.textures[i]); });
    const keys = new Map(); // vertex index -> key (only vertices this primitive uses)
    const idx = g.index ? g.index.array : null;
    const n = idx ? idx.length : pos.count;
    for (let t = 0; t < n; t++) {
      const vi = idx ? idx[t] : t;
      if (keys.has(vi)) continue;
      const k = key(v.fromBufferAttribute(pos, vi), o.matrixWorld);
      keys.set(vi, k);
      let f = field.get(k); if (!f) field.set(k, f = new Map());
      grp.materials.forEach((m, i) => {
        const s = grp.slots[i]; if (s == null) return;
        const w = s < 4 ? comp(w0, vi, s) : comp(w1, vi, s - 4);
        if (w > (f.get(m) || 0)) f.set(m, w);
      });
    }
    per.push({ o, grp, keys });
  }
  for (const { o, grp, keys } of per) {
    const g = o.geometry, count = g.getAttribute('position').count;
    const tot = new Map();
    for (const k of keys.values()) for (const [m, w] of field.get(k)) if (matTex.has(m)) tot.set(m, (tot.get(m) || 0) + w);
    const mats = [...tot].filter(([, w]) => w > 1e-3).sort((a, b) => b[1] - a[1]).slice(0, MAXT).map(([m]) => m).sort((a, b) => a - b);
    if (!mats.length) continue;
    const a0 = new Float32Array(count * 4), a1 = new Float32Array(count * 4);
    for (const [vi, k] of keys) {
      const f = field.get(k); let sum = 0;
      for (const m of mats) sum += f.get(m) || 0;
      mats.forEach((m, s) => { const w = sum > 1e-5 ? (f.get(m) || 0) / sum : (s === 0 ? 1 : 0); (s < 4 ? a0 : a1)[vi * 4 + (s % 4)] = w; });
    }
    g.setAttribute('w0', new THREE.BufferAttribute(a0, 4));
    g.setAttribute('w1', new THREE.BufferAttribute(a1, 4));
    o.userData.stitched = { materials: mats, slots: mats.map((_, i) => i), textures: mats.map(m => matTex.get(m)) };
  }
}

function upgradeTerrain(root, out) {
  const found = [];
  root.updateMatrixWorld(true);
  root.traverse(o => {
    if (!o.isMesh || !o.geometry.getAttribute('_weights0')) return;
    const g = o.geometry;
    if (!g.getAttribute('w0')) {
      g.setAttribute('w0', g.getAttribute('_weights0'));
      g.setAttribute('w1', g.getAttribute('_weights1') || g.getAttribute('_weights0'));
      g.setAttribute('vcol', g.getAttribute('color') || g.getAttribute('_weights0'));
      for (const n of ['_weights0', '_weights1', 'color']) if (g.getAttribute(n)) g.deleteAttribute(n);
    }
    // GLTFLoader copies the mesh's extras onto every primitive mesh; primitives are the node's children in order
    const groups = o.userData.groups;
    const grp = groups && groups[o.parent.children.filter(c => c.isMesh).indexOf(o)];
    if (!grp) return;
    found.push({ o, grp });
  });
  try { stitchTerrain(found); } catch (e) { console.warn('terrain stitching failed', e); }
  for (const { o, grp } of found) {
    const st = o.userData.stitched || grp;
    o.material = makeTerrainMaterial(st);
    o.userData.terrain = true;
    out.terrain.push(o);
    for (const t of st.textures) if (t) out.terrainTextures.add(t.replace(/\.dds$/i, '.png'));
  }
}

// ───────────────────────── loading ─────────────────────────
const missing = new Set();
let model = null;
let layers = { terrain: [], water: [], objects: [] };
let frameSize = 100;
let loadToken = 0;

function disposeModel() {
  if (!model) return;
  scene.remove(model);
  model.traverse(o => {
    if (o.geometry) o.geometry.dispose();
    const mats = o.material ? (Array.isArray(o.material) ? o.material : [o.material]) : [];
    for (const m of mats) {
      if (!m.isShaderMaterial) for (const k of ['map', 'normalMap', 'emissiveMap']) m[k]?.dispose();
      m.dispose();
    }
  });
  model = null;
}

function setStatus(msg) {
  const s = $('status');
  s.style.display = msg ? 'block' : 'none';
  s.textContent = msg || '';
}

async function select(t, name, variant) {
  const token = ++loadToken;
  const isMap = t === 'maps';
  const entry = catalog[t].find(e => e[0] === name);
  if (!entry) return;
  if (isMap) {
    const vs = entry[1];
    if (!variant || !(variant in vs)) variant = vs.full ? 'full' : vs.base ? 'base' : Object.keys(vs)[0];
  } else variant = null;
  current = { tab: t, name, variant };
  location.hash = `#${t}/${encodeURIComponent(name)}${variant ? '/' + variant : ''}`;
  renderRows();
  buildVariants(entry);

  const fileBase = isMap && variant !== 'base' ? `${name}_${variant}` : name;
  const dir = TABS.find(x => x.id === t).dir;
  const url = `${DATA}/${dir}/${encodeURIComponent(fileBase)}.glb`;
  $('title').textContent = fileBase;
  setStatus('Loading…');
  missing.clear();

  const manager = new THREE.LoadingManager();
  manager.onError = u => missing.add(decodeURIComponent(u.split('/').pop()));
  const loader = new GLTFLoader(manager);
  let gltf;
  try {
    gltf = await loader.loadAsync(url, ev => {
      if (ev.total && token === loadToken) setStatus(`Loading… ${(ev.loaded / 1048576).toFixed(1)} / ${(ev.total / 1048576).toFixed(1)} MB`);
    });
  } catch (err) {
    if (token === loadToken) setStatus('Failed to load: ' + err.message);
    return;
  }
  if (token !== loadToken) return;

  disposeModel();
  model = gltf.scene;
  scene.add(model);

  const info = { terrain: [], terrainTextures: new Set() };
  upgradeTerrain(model, info);
  layers = { terrain: info.terrain, water: [], objects: [] };
  let tris = 0, meshes = 0;
  model.traverse(o => {
    if (!o.isMesh) return;
    meshes++;
    const idx = o.geometry.index;
    tris += (idx ? idx.count : o.geometry.getAttribute('position').count) / 3;
    if (o.userData.terrain) return;
    const mats = Array.isArray(o.material) ? o.material : [o.material];
    for (const m of mats) {
      if (m.map) { m.alphaTest = 0.4; m.map.anisotropy = 8; }
      m.side = THREE.DoubleSide;
    }
    const isWater = /water/i.test(o.name + ' ' + o.parent?.name + ' ' + mats.map(m => m.name).join(' ')) ||
      mats.some(m => m.map && /(^|\/)water[^/]*$/i.test(m.map.name || ''));
    if (isWater) {
      for (const m of mats) { m.transparent = true; m.opacity = 0.65; m.depthWrite = false; m.alphaTest = 0; }
      layers.water.push(o);
    } else layers.objects.push(o);
  });

  // texture list straight from the glTF so every referenced image is shown
  const uris = (gltf.parser.json.images || []).map(i => decodeURIComponent((i.uri || '').split('/').pop()));
  const texFiles = [...new Set([...uris, ...info.terrainTextures])].filter(Boolean).sort();

  applyToggles();
  frame();
  showInfo({ tris, meshes, texFiles, hasTerrain: info.terrain.length > 0, isMap, objects: layers.objects.length, water: layers.water.length });
  setStatus('');

  // lazily check textures that failed (terrain textures load async after this point)
  setTimeout(() => showMissing(texFiles), 2500);
}

function buildVariants(entry) {
  const el = $('variants');
  el.innerHTML = '';
  if (current.tab !== 'maps') return;
  const vs = entry[1];
  for (const [k, label] of [['base', 'Base'], ['full', 'Full'], ['empty', 'Terrain only']]) {
    const b = document.createElement('button');
    b.textContent = label;
    b.disabled = !(k in vs);
    b.className = current.variant === k ? 'on' : '';
    b.title = k in vs ? fmtSize(vs[k]) : 'not available';
    b.onclick = () => select('maps', current.name, k);
    el.appendChild(b);
  }
}

function showInfo(i) {
  const el = $('info');
  el.hidden = false;
  el.innerHTML = '';
  const kv = (k, v) => { const d = document.createElement('div'); d.className = 'kv'; d.innerHTML = `${k}: <b>${v}</b>`; el.appendChild(d); };
  kv('Triangles', Math.round(i.tris).toLocaleString());
  kv('Meshes', i.meshes.toLocaleString());
  if (i.isMap) { kv('Terrain tiles', layers.terrain.length ? 'yes (splat-blended)' : 'no'); kv('Objects / water', `${i.objects} / ${i.water}`); }
  kv('Textures', i.texFiles.length);
  const warn = document.createElement('div'); warn.id = 'warn'; warn.className = 'kv warn'; el.appendChild(warn);
  const h = document.createElement('h3'); h.textContent = 'TEXTURES'; h.style.marginTop = '8px'; el.appendChild(h);
  const grid = document.createElement('div'); grid.id = 'texs';
  for (const f of i.texFiles.slice(0, 300)) {
    const img = document.createElement('img');
    img.loading = 'lazy'; img.title = f; img.src = `${DATA}/textures/${encodeURIComponent(f)}`;
    img.onerror = () => { img.style.opacity = .3; missing.add(f); };
    img.onclick = () => { $('zimg').src = img.src; $('zname').textContent = f; $('zoom').style.display = 'flex'; };
    grid.appendChild(img);
  }
  el.appendChild(grid);
  if (i.texFiles.length > 300) { const d = document.createElement('div'); d.className = 'kv'; d.textContent = `+ ${i.texFiles.length - 300} more`; el.appendChild(d); }
}
function showMissing() {
  const w = $('warn');
  if (w) w.textContent = missing.size ? `⚠ ${missing.size} texture file(s) missing: ${[...missing].slice(0, 3).join(', ')}${missing.size > 3 ? '…' : ''}` : '';
}
$('zoom').onclick = () => { $('zoom').style.display = 'none'; };

// ───────────────────────── view controls ─────────────────────────
function frame() {
  if (!model) return;
  // plain geometry bounds (skinned meshes: bind pose) so bone transforms can't skew the framing
  const box = new THREE.Box3(), tmp = new THREE.Box3();
  model.updateMatrixWorld(true);
  model.traverse(o => {
    if (!o.isMesh || !o.visible) return;
    if (!o.geometry.boundingBox) o.geometry.computeBoundingBox();
    box.union(tmp.copy(o.geometry.boundingBox).applyMatrix4(o.matrixWorld));
  });
  if (box.isEmpty()) return;
  const size = box.getSize(new THREE.Vector3());
  const c = box.getCenter(new THREE.Vector3());
  const r = Math.max(size.x, size.y, size.z, 1e-3);
  frameSize = r;
  const isMap = current?.tab === 'maps' || current?.tab === 'terrain';
  const dir = new THREE.Vector3(0.7, isMap ? 0.9 : 0.55, 0.9).normalize();
  const dist = r * (isMap ? 0.9 : 1.5);
  camera.position.copy(c).addScaledVector(dir, dist);
  controls.target.copy(c);
  camera.near = Math.max(r / 20000, 0.01);
  camera.far = r * 30;
  camera.updateProjectionMatrix();
  controls.minDistance = r * 0.002;
  controls.maxDistance = r * 10;
  controls.update();
  grid.scale.setScalar(r * 1.5);
  grid.position.set(c.x, box.min.y, c.z);
}

function applyToggles() {
  for (const m of layers.terrain) m.visible = $('tTerrain').checked;
  for (const m of layers.water) m.visible = $('tWater').checked;
  for (const m of layers.objects) m.visible = $('tObjects').checked;
  const wire = $('tWire').checked;
  model?.traverse(o => {
    if (!o.isMesh) return;
    for (const m of Array.isArray(o.material) ? o.material : [o.material]) m.wireframe = wire;
  });
  grid.visible = $('tGrid').checked;
  terrainUniformsShared.uTint.value = $('tTint').checked ? 1 : 0;
  terrainUniformsShared.uScale.value = 1 / +$('rScale').value;
  const isMap = current?.tab === 'maps';
  const hasTerrain = isMap || current?.tab === 'terrain';
  for (const id of ['lblTerrain', 'lblWater', 'lblObjects']) $(id).style.display = isMap ? '' : 'none';
  for (const id of ['lblTint', 'lblScale']) $(id).style.display = hasTerrain ? '' : 'none';
}
for (const id of ['tTerrain', 'tWater', 'tObjects', 'tWire', 'tGrid', 'tTint', 'rScale']) $(id).addEventListener('input', applyToggles);
$('shot').onclick = () => {
  renderer.render(scene, camera);
  canvas.toBlob(b => { const a = document.createElement('a'); a.href = URL.createObjectURL(b); a.download = (current?.name || 'openoctober') + '.png'; a.click(); });
};

const keys = new Set();
window.addEventListener('keydown', e => {
  if (e.target.tagName === 'INPUT' || e.target.tagName === 'SELECT') return;
  const k = e.key.toLowerCase();
  if (k === 'f') frame();
  if (k === 'escape') $('zoom').style.display = 'none';
  keys.add(k);
});
window.addEventListener('keyup', e => keys.delete(e.key.toLowerCase()));
window.addEventListener('blur', () => keys.clear());

const clock = new THREE.Clock();
const fwd = new THREE.Vector3(), right = new THREE.Vector3(), mv = new THREE.Vector3();
function animate() {
  requestAnimationFrame(animate);
  const dt = Math.min(clock.getDelta(), 0.1);
  if (keys.size && model) {
    camera.getWorldDirection(fwd);
    right.crossVectors(fwd, camera.up).normalize();
    mv.set(0, 0, 0);
    if (keys.has('w')) mv.add(fwd);
    if (keys.has('s')) mv.sub(fwd);
    if (keys.has('d')) mv.add(right);
    if (keys.has('a')) mv.sub(right);
    if (keys.has('e')) mv.y += 1;
    if (keys.has('q')) mv.y -= 1;
    if (mv.lengthSq()) {
      const speed = Math.max(camera.position.distanceTo(controls.target) * 0.9, frameSize * 0.02) * (keys.has('shift') ? 4 : 1);
      mv.normalize().multiplyScalar(speed * dt);
      camera.position.add(mv);
      controls.target.add(mv);
    }
  }
  controls.update();
  renderer.render(scene, camera);
}

// ───────────────────────── boot ─────────────────────────
async function boot() {
  const res = await fetch('catalog.json');
  if (!res.ok) { $('count').textContent = 'catalog.json missing — run build_catalog.py'; return; }
  catalog = await res.json();
  const m = location.hash.match(/^#(\w+)\/([^/]+)(?:\/(\w+))?/);
  if (m && catalog[m[1]]) tab = m[1];
  buildTabs();
  applyFilter();
  applyToggles();
  animate();
  if (m && catalog[m[1]]) {
    const name = decodeURIComponent(m[2]);
    const idx = filtered.findIndex(e => e[0] === name);
    if (idx >= 0) $('list').scrollTop = Math.max(0, idx * ROW_H - 100);
    select(m[1], name, m[3]);
  } else select('maps', 'a05');
}
boot();
