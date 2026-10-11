import * as THREE from 'three';
import { Environment } from './env.js';
import { Sky } from './sky.js';
import { WaterFactory, SEA_LEVEL } from './water.js';
import { TerrainTextures } from './terrain.js';
import { World } from './world.js';
import { Music, Ambience } from './music.js';
import { Player, SPEEDS } from './collision.js';
import { DynLights } from './lights.js';

const $ = id => document.getElementById(id);
const q = new URLSearchParams(location.search);
// ---- performance settings (persisted). View distance: fog is relative to it, like the game's FogStart/FogEnd fractions.
const SHADOWS = { Off: [0, 0], Low: [1024, 120], Medium: [2048, 200], High: [4096, 300] };   // [map size, covered radius m]
const PRESETS = { Low: { view: 2200, obj: 700, scale: 0.75, shadow: 'Off', lights: 4 }, Medium: { view: 3200, obj: 1100, scale: 1, shadow: 'Low', lights: 8 }, High: { view: 4500, obj: 1500, scale: 1.25, shadow: 'Medium', lights: 16 }, Ultra: { view: 7000, obj: 2200, scale: 1.5, shadow: 'High', lights: 16 } };
let S = { preset: 'Medium', ...PRESETS.Medium };
try { Object.assign(S, JSON.parse(localStorage.getItem('dfSettings2') || '{}')); } catch {}
if (q.get('view')) S.view = +q.get('view'); if (q.get('obj')) S.obj = +q.get('obj');
let VIEW = S.view;
const J = u => fetch(u).then(r => r.json());

const renderer = new THREE.WebGLRenderer({ canvas: $('view'), antialias: true, powerPreference: 'high-performance' });
renderer.setPixelRatio(Math.min(devicePixelRatio, S.scale));
renderer.outputColorSpace = THREE.SRGBColorSpace;
const scene = new THREE.Scene();
const camera = new THREE.PerspectiveCamera(70, 1, 1.5, VIEW * 1.05);
camera.rotation.order = 'YXZ';
function resize() { renderer.setSize(innerWidth, innerHeight, false); camera.aspect = innerWidth / innerHeight; camera.updateProjectionMatrix(); }
addEventListener('resize', resize); resize();

const [worldData, musicAreas, tileNames, envData, terrainManifest, props] = await Promise.all(
  ['world', 'music', 'tiles', 'env', 'terrain_textures', 'props'].map(n => J(`/data/${n}.json`)));

// ---- lights for ordinary (object) materials; terrain/water do their own lighting from the same uniforms
const env = new Environment(envData);
env.config = q.get('env') || env.config;
if (q.get('t')) env.time = +q.get('t');
renderer.shadowMap.enabled = true; renderer.shadowMap.type = THREE.PCFShadowMap;
const sunL = new THREE.DirectionalLight(0xffffff, Math.PI * 1.6);
sunL.shadow.bias = -0.0004; sunL.shadow.normalBias = 0.5; scene.add(sunL.target);
const moonL = new THREE.DirectionalLight(0xffffff, Math.PI * 1.6);
const hemi = new THREE.HemisphereLight(0xffffff, 0xffffff, Math.PI * 1.6);
scene.add(sunL, moonL, hemi);

const loader = new THREE.TextureLoader();
const sky = new Sky(env, loader); scene.add(sky.group);
const tex = new TerrainTextures(renderer, terrainManifest);
const water = new WaterFactory(env, sky.uniforms.uZenith);
const world = new World({ scene, env, tex, water, tiles: worldData, names: tileNames, props, opts: { terrainRadius: VIEW * 0.9, objectRadius: S.obj } });

// open ocean everywhere (terrain-only far tiles carry no water plane of their own)
const ocean = new THREE.Mesh(new THREE.PlaneGeometry(2, 2).rotateX(-Math.PI / 2), water.material('water', { value: 1 }));
ocean.frustumCulled = false; ocean.renderOrder = 1; ocean.scale.setScalar(VIEW * 1.1); scene.add(ocean);

const music = new Music(musicAreas, env, (title, area) => { const t = $('track'); t.textContent = `♪ ${title}`; t.classList.add('on'); clearTimeout(t._h); t._h = setTimeout(() => t.classList.remove('on'), 9000); });
const ambience = new Ambience(env);
const dyn = new DynLights(env, world);
const player = new Player(world);
env.u.uShadowMat.value = sunL.shadow.matrix;
let shadowSize = -1;

// ---- start position (game coordinates -> three: z flipped). Default = SFDefaultPosition from worldconfig.xml
const start = { x: +q.get('x') || -7712, y: +q.get('y') || 0, z: -(+q.get('z') || 5600) };
camera.position.set(start.x, start.y || 520, start.z);
let yaw = q.get('yaw') ? +q.get('yaw') : Math.PI * 0.85, pitch = q.get('pitch') ? +q.get('pitch') : -0.12;
const keys = new Set(); let baseSpeed = 70, vel = new THREE.Vector3(), locked = false, noclip = false;

// ---- input
addEventListener('keydown', e => {
  if (e.repeat) return;
  keys.add(e.code);
  const hit = c => e.code === c;
  if (hit('KeyT')) env.auto = !env.auto;
  if (hit('BracketRight')) env.time = (env.time + (e.shiftKey ? 120 : 30)) % 1440;
  if (hit('BracketLeft')) env.time = (env.time - (e.shiftKey ? 120 : 30) + 1440) % 1440;
  if (hit('KeyV')) toast('Atmosphere: ' + env.cycleConfig());
  if (hit('KeyM')) toast(music.toggle() ? 'Music on' : 'Music muted');
  if (hit('KeyG')) toggleMode();
  if (hit('KeyN')) { noclip = !noclip; toast(noclip ? 'No clipping' : 'Ground clamp'); }
  if (hit('KeyO')) $('cfg').classList.toggle('on');
  if (hit('KeyH')) $('help').classList.toggle('on');
  if (hit('KeyP')) console.log(JSON.stringify({ x: camera.position.x, y: camera.position.y, z: -camera.position.z, yaw, pitch }));
  if (e.code.startsWith('Arrow') || e.code === 'Space') e.preventDefault();
});
addEventListener('keyup', e => keys.delete(e.code));
addEventListener('blur', () => keys.clear());
addEventListener('mousemove', e => { if (!locked && !dragging) return; yaw -= e.movementX * 0.0022; pitch = Math.max(-1.55, Math.min(1.55, pitch - e.movementY * 0.0022)); });
let dragging = false;
$('view').addEventListener('mousedown', () => { if (!document.body.classList.contains('playing')) return; if (!locked) $('view').requestPointerLock?.(); });
document.addEventListener('pointerlockchange', () => { locked = document.pointerLockElement === $('view'); });
addEventListener('wheel', e => { baseSpeed = Math.max(4, Math.min(2500, baseSpeed * Math.exp(-e.deltaY * 0.0012))); toast(`Speed ${Math.round(baseSpeed)} m/s`); }, { passive: true });
function toast(msg) { const t = $('banner'); t.textContent = msg; t.style.fontSize = '22px'; t.style.top = '10%'; t.classList.add('on'); clearTimeout(t._h); t._h = setTimeout(() => t.classList.remove('on'), 1600); }

// ---- HUD
let lastRegion = null;
function banner(text) { const b = $('banner'); b.textContent = text; b.style.fontSize = '40px'; b.style.top = '16%'; b.classList.add('on'); clearTimeout(b._h); b._h = setTimeout(() => b.classList.remove('on'), 4200); }

// ---- loop
const clock = new THREE.Clock(); let fpsAcc = 0, fpsN = 0, fps = 0, cur = null;
const fwd = new THREE.Vector3(), right = new THREE.Vector3(), up = new THREE.Vector3(0, 1, 0), mv = new THREE.Vector3();
function frame() {
  try { frameInner(); } catch (e) { console.error('frame', e.stack || e); }
  requestAnimationFrame(frame);
}
function frameInner() {
  const raw = clock.getDelta(), dt = Math.min(raw, 0.1);
  // movement
  camera.rotation.set(pitch, yaw, 0);
  const run = keys.has('ShiftLeft') || keys.has('ShiftRight');
  let gh = world.groundHeight(camera.position.x, camera.position.z);
  if (mode === 'walk') {
    // WASD relative to where you look, flattened onto the ground plane
    let fx = 0, fz = 0;
    if (keys.has('KeyW')) fz -= 1; if (keys.has('KeyS')) fz += 1; if (keys.has('KeyD')) fx += 1; if (keys.has('KeyA')) fx -= 1;
    const l = Math.hypot(fx, fz) || 1, s = Math.sin(yaw), c = Math.cos(yaw);
    // game speeds: W = run, Ctrl = walk, Shift = sprint (see SPEEDS)
    const sp = player.swim ? (run ? SPEEDS.swim * 1.75 : SPEEDS.swim) : keys.has('ControlLeft') ? SPEEDS.walk : run ? SPEEDS.sprint : SPEEDS.run;
    const tx = (fx * c + fz * s) / l * sp, tz = (-fx * s + fz * c) / l * sp;
    const k = 1 - Math.exp(-dt * (player.onGround ? 12 : 2.5));          // less air control
    vel.x += (tx - vel.x) * k; vel.z += (tz - vel.z) * k;
    let left = dt; const jump = keys.has('Space');
    while (left > 0) { const h = Math.min(left, 1 / 60); player.update(h, vel.x, vel.z, jump); left -= h; }
    camera.position.set(player.pos.x, player.pos.y + player.eye, player.pos.z);
  } else {
    camera.getWorldDirection(fwd); right.crossVectors(fwd, up).normalize();
    mv.set(0, 0, 0);
    if (keys.has('KeyW')) mv.add(fwd); if (keys.has('KeyS')) mv.sub(fwd);
    if (keys.has('KeyD')) mv.add(right); if (keys.has('KeyA')) mv.sub(right);
    if (keys.has('Space') || keys.has('KeyE')) mv.y += 1; if (keys.has('KeyC') || keys.has('KeyQ')) mv.y -= 1;
    if (mv.lengthSq()) mv.normalize();
    const sp = baseSpeed * (run ? 5 : 1) * (keys.has('ControlLeft') ? 0.2 : 1);
    vel.lerp(mv.multiplyScalar(sp), 1 - Math.exp(-dt * 6));
    camera.position.addScaledVector(vel, dt);
    if (!noclip && gh != null) camera.position.y = Math.max(camera.position.y, Math.max(gh, SEA_LEVEL) + 1.8);
  }
  camera.updateMatrixWorld(true);

  env.update(dt, VIEW);
  env.u.uSunDirView.value.copy(env.u.uSunDir.value).transformDirection(camera.matrixWorldInverse);
  renderer.setClearColor(env.u.uFogColor.value);
  sunL.color.copy(env.u.uSunColor.value); sunL.intensity = Math.PI * 1.6 * Math.max(env.u.uSunDir.value.y > -0.05 ? 1 : 0, 0);
    sunL.intensity *= Math.min(1, Math.max(0, (env.u.uSunDir.value.y + 0.04) / 0.16));
  moonL.color.set(0, 0, 0); { let b = 0; env.u.uMoonColor.value.forEach((c, i) => { moonL.color.add(c); }); moonL.position.copy(env.u.uMoonDir.value[0]).multiplyScalar(100).add(camera.position); moonL.target.position.copy(camera.position); moonL.target.updateMatrixWorld(); }
  hemi.color.copy(env.u.uAmbSky.value); hemi.groundColor.copy(env.u.uAmbGround.value);
  updateShadow();
  dyn.count = S.lights; dyn.update(dt, camera);
  sky.update(camera);
  ocean.position.set(Math.round(camera.position.x / 64) * 64, SEA_LEVEL - 3.0, Math.round(camera.position.z / 64) * 64);

  world.update(Math.min(raw, 0.5), camera);

  // place / music / ambience
  const t = world.tileAt(camera.position.x, camera.position.z);
  if (t !== cur) {
    cur = t;
    if (t && t.display !== lastRegion) { if (lastRegion !== null) banner(t.display); lastRegion = t.display; }
  }
  const tileName = t?.name;
  music.setTile(tileName, dt); music.update(dt);
  const agl = gh != null ? camera.position.y - gh : 50;
  ambience.update(dt, { y: camera.position.y, agl: Math.max(0, agl), treeDensity: t ? Math.min(1, (t.info.trees || 0) / 450) : 0, riverDist: 1e9, muted: music.muted });

  renderer.render(scene, camera);

  fpsAcc += dt; fpsN++;
  if (fpsAcc > 0.5) { fps = Math.round(fpsN / fpsAcc); fpsAcc = 0; fpsN = 0; hud(t, gh); }
}
// ---- shadows: one sun shadow map following the camera (the game uses a ~380 m ortho range), snapped to texels to avoid shimmer
const sRight = new THREE.Vector3(), sUp = new THREE.Vector3(), sCen = new THREE.Vector3();
function updateShadow() {
  const [size, R] = SHADOWS[S.shadow] || SHADOWS.Off, sd = env.u.uSunDir.value;
  const on = size > 0 && sd.y > 0.07;
  if (size !== shadowSize && size > 0) { shadowSize = size; sunL.shadow.map?.dispose(); sunL.shadow.map = null; sunL.shadow.mapSize.set(size, size); }
  sunL.castShadow = on; env.u.uShadowOn.value = on && sunL.shadow.map ? 1 : 0;
  if (size > 0) env.u.uShadowTexel.value = 1 / size;
  sRight.crossVectors(up, sd).normalize(); sUp.crossVectors(sd, sRight);
  const texel = 2 * R / Math.max(size, 1), cu = camera.position.dot(sRight), cv = camera.position.dot(sUp);
  sCen.copy(camera.position).addScaledVector(sRight, Math.round(cu / texel) * texel - cu).addScaledVector(sUp, Math.round(cv / texel) * texel - cv);
  const cam = sunL.shadow.camera; cam.left = -R; cam.right = R; cam.top = R; cam.bottom = -R; cam.near = 1; cam.far = R * 6; cam.updateProjectionMatrix();
  sunL.target.position.copy(sCen); sunL.target.updateMatrixWorld();
  sunL.position.copy(sCen).addScaledVector(sd, R * 3);
  env.u.uShadowMap.value = sunL.shadow.map ? sunL.shadow.map.texture : null;
}
// ---- walk / fly
let mode = 'fly';
$('modebtn').addEventListener('click', () => { toggleMode(); $('view').requestPointerLock?.()?.catch?.(() => {}); });
function setModeLabel() { $('modebtn').textContent = mode === 'walk' ? 'Mode: Walk (G) — click to fly' : 'Mode: Fly (G) — click to walk'; }
function toggleMode() {
  if (mode === 'fly') {
    mode = 'walk'; player.pos.set(camera.position.x, camera.position.y - player.eye, camera.position.z); player.vy = 0; vel.set(0, 0, 0);
    toast('Walking — G to fly'); setModeLabel();
  } else { mode = 'fly'; vel.set(0, 0, 0); toast('Flying — G to walk'); setModeLabel(); }
}
function hud(t, gh) {
  const p = camera.position, area = music.area?.name;
  $('place').textContent = t ? t.display : 'The Open Sea';
  $('sub').textContent = `${mode === 'walk' ? 'WALK' : 'FLY'} · ${t ? t.name.toUpperCase() : '—'}${area ? ' · ' + area : ''} · ${env.hhmm()}${env.auto ? '' : ' (paused)'}`;
  $('stats').textContent = `${fps} fps   ${world.stats.terrain} terrain · ${world.stats.objects} object tiles${world.stats.loading ? '   loading ' + world.stats.loading : ''}\n` +
    `x ${p.x.toFixed(0)}  y ${p.y.toFixed(0)}  z ${(-p.z).toFixed(0)}   ${Math.round(baseSpeed)} m/s\n${renderer.info.render.calls} calls · ${(renderer.info.render.triangles / 1e6).toFixed(2)}M tris`;
}
$('help').innerHTML = `<b>WASD</b> fly · <b>Shift</b> fast · <b>Ctrl</b> slow · <b>Space/E</b> up · <b>C/Q</b> down · <b>Wheel</b> base speed · <b>G</b> walk with collision (Space jumps, Shift sprints, Ctrl slow walk)<br><b>T</b> pause/resume time · <b>[ ]</b> ±30 min (Shift ±2 h) · <b>V</b> atmosphere · <b>M</b> music · <b>N</b> noclip · <b>H</b> hide this`;
setTimeout(setModeLabel, 0);

// ---- boot: preload around the start, then wait for the click (also unlocks audio)
window.dbg = { player, tp: (x, y, z) => { camera.position.set(x, y, -z); player.pos.set(x, y - player.eye, -z); player.vy = 0; }, camera, world, env, music, scene, renderer, ambience, set: (x, y, z) => camera.position.set(x, y, -z), look: (yw, pt) => { yaw = yw; pitch = pt; } };
const intro = $('intro');
let ready = false;
function bootProgress() {
  world.update(0.25, camera); env.update(0, VIEW);
  const near = [...world.tiles.values()].filter(t => t.hasTerrain && t.dist < 1500);
  const done = near.filter(t => t.tState === 2).length;
  $('load').textContent = `Reading the world… ${done}/${near.length || 1}`;
  if (!ready && (done >= near.length || performance.now() > 30000)) { ready = true; $('load').style.display = 'none'; $('enter').style.display = ''; }
  else if (!ready) setTimeout(bootProgress, 250);
}
bootProgress();
let started = false;
function begin() {
  if (started || !ready) return; started = true;
  document.body.classList.add('playing'); intro.classList.add('hide');
  music.enable(); ambience.enable();
  $('view').requestPointerLock?.()?.catch?.(() => {});
  clock.getDelta(); frame();
}
intro.addEventListener('click', begin);
if (q.get('auto')) { const iv = setInterval(() => { if (ready) { clearInterval(iv); begin(); } }, 200); }

// ---- settings panel
function applySettings() {
  VIEW = S.view; camera.far = VIEW * 1.05; camera.updateProjectionMatrix();
  world.opts.terrainRadius = VIEW * 0.9; world.opts.objectRadius = S.obj; ocean.scale.setScalar(VIEW * 1.1);
  renderer.setPixelRatio(Math.min(devicePixelRatio, S.scale)); resize();
  try { localStorage.setItem('dfSettings2', JSON.stringify(S)); } catch {}
}
function buildPanel() {
  const c = $('cfg');
  const row = (key, label, min, max, step, fmt) => `<label><span>${label}</span><span id="v_${key}">${fmt(S[key])}</span></label><input type="range" id="r_${key}" min="${min}" max="${max}" step="${step}" value="${S[key]}">`;
  c.innerHTML = `<b>PERFORMANCE</b><label><span>Preset</span><select id="preset">${Object.keys(PRESETS).map(p => `<option${p === S.preset ? ' selected' : ''}>${p}</option>`).join('')}<option${S.preset === 'Custom' ? ' selected' : ''}>Custom</option></select></label>` +
    row('view', 'View distance', 1200, 9000, 100, v => (v / 1000).toFixed(1) + ' km') + row('obj', 'Object distance', 300, 3000, 100, v => (v / 1000).toFixed(1) + ' km') + row('scale', 'Render scale', 0.5, 2, 0.05, v => v.toFixed(2) + '×') +
    `<label><span>Shadows</span><select id="sel_shadow">${Object.keys(SHADOWS).map(k => `<option${k === S.shadow ? ' selected' : ''}>${k}</option>`).join('')}</select></label><label><span>Dynamic lights</span><select id="sel_lights">${[0, 4, 8, 16].map(k => `<option${k === S.lights ? ' selected' : ''}>${k}</option>`).join('')}</select></label>` +
    '<div style="opacity:.55;margin-top:8px">Lower view/object distance if the frame rate drops. Settings are remembered.</div>';
  const sync = () => { for (const k of ['view', 'obj', 'scale']) { $('r_' + k).value = S[k]; $('v_' + k).textContent = { view: v => (v / 1000).toFixed(1) + ' km', obj: v => (v / 1000).toFixed(1) + ' km', scale: v => v.toFixed(2) + '×' }[k](S[k]); } $('preset').value = S.preset; $('sel_shadow').value = S.shadow; $('sel_lights').value = String(S.lights); };
  for (const k of ['view', 'obj', 'scale']) $('r_' + k).addEventListener('input', e => { S[k] = +e.target.value; S.preset = 'Custom'; sync(); applySettings(); });
  $('sel_shadow').addEventListener('change', e => { S.shadow = e.target.value; S.preset = 'Custom'; sync(); applySettings(); });
  $('sel_lights').addEventListener('change', e => { S.lights = +e.target.value; S.preset = 'Custom'; sync(); applySettings(); });
  $('preset').addEventListener('change', e => { const p = PRESETS[e.target.value]; if (p) { Object.assign(S, p, { preset: e.target.value }); sync(); applySettings(); } else S.preset = 'Custom'; });
  $('cfgbtn').addEventListener('click', () => { c.classList.toggle('on'); if (document.pointerLockElement) document.exitPointerLock(); });
}
buildPanel();
