// World landmarks (cities, villages, hamlets, wilderness banks, ...): billboarded name tags with a glyph and a light
// pillar, from data/pois.json (tools/build_pois.js). Positions are game coords; three.js z is negated.
import * as THREE from 'three';

const STYLE = {   // colour, glyph, tag size multiplier, visible distance (m)
  'NPC City':        { c: '#f5d77a', g: '♛', k: 1.5, far: 9000 },
  'Player City':     { c: '#e8a35c', g: '♜', k: 1.35, far: 7000 },
  'Chaos City':      { c: '#d0584a', g: '☠', k: 1.35, far: 7000 },
  'Fortress':        { c: '#c9a0e8', g: '⚔', k: 1.35, far: 7000 },
  'Player Hamlet':   { c: '#b7d98a', g: '⌂', k: 1.05, far: 4000 },
  'Village':         { c: '#8fd0c4', g: '⌂', k: 1.05, far: 4000 },
  'Wilderness Bank': { c: '#ffe27a', g: '¤', k: 1, far: 3000 },
  'Dungeon':         { c: '#9aa3b5', g: '▼', k: 0.9, far: 2500 },
};
const FONT = '600 28px Palatino, Georgia, serif';

export class Pois {
  constructor({ scene, world, data }) {
    this.scene = scene; this.world = world; this.enabled = true; this.t = 0; this.mats = new Map();
    this.group = new THREE.Group(); scene.add(this.group);
    this.items = data.map(p => {
      const st = STYLE[p.type]; if (!st) return null;
      const tag = new THREE.Sprite(this.matFor(p, st)); tag.renderOrder = 19; tag.center.set(0.5, 0);
      const aspect = tag.material.userData.aspect;
      const beam = new THREE.Mesh(BEAM, new THREE.MeshBasicMaterial({ color: st.c, transparent: true, opacity: 0.35, depthWrite: false, fog: false, blending: THREE.AdditiveBlending }));
      beam.renderOrder = 18;
      return { p, st, tag, aspect, beam, x: p.x, z: -p.z, y: 230, grounded: false, shown: false };
    }).filter(Boolean);
  }
  matFor(p, st) {
    const label = `${st.g} ${p.name}`, key = st.c + label;
    if (!this.mats.has(key)) {
      const c = document.createElement('canvas'), x = c.getContext('2d'); x.font = FONT;
      const w = Math.ceil(x.measureText(label).width) + 24; c.width = w; c.height = 52; x.font = FONT;
      x.textAlign = 'center'; x.textBaseline = 'middle'; x.fillStyle = '#0009'; x.beginPath(); x.roundRect(0, 5, w, 42, 12); x.fill();
      x.strokeStyle = st.c + '88'; x.lineWidth = 2; x.stroke(); x.fillStyle = st.c; x.fillText(label, w / 2, 27);
      const tex = new THREE.CanvasTexture(c); tex.colorSpace = THREE.SRGBColorSpace;
      const m = new THREE.SpriteMaterial({ map: tex, transparent: true, depthWrite: false, depthTest: false, fog: false }); m.userData.aspect = w / 52;
      this.mats.set(key, m);
    }
    return this.mats.get(key);
  }
  setEnabled(on) { this.enabled = on; if (!on) for (const it of this.items) if (it.shown) { this.group.remove(it.tag, it.beam); it.shown = false; } }
  update(dt, camera) {
    if (!this.enabled) return;
    this.t -= dt; const reground = this.t <= 0; if (reground) this.t = 0.5;
    const cx = camera.position.x, cz = camera.position.z, fovK = Math.tan(camera.fov * Math.PI / 360) * 2 / innerHeight;
    for (const it of this.items) {
      const dx = it.x - cx, dz = it.z - cz, d = Math.hypot(dx, dz);
      if (d > it.st.far) { if (it.shown) { this.group.remove(it.tag, it.beam); it.shown = false; } continue; }
      if (reground && d < 2500) { const h = this.world.groundHeight(it.x, it.z); if (h != null && h > -1e8) { it.y = Math.max(h, 215); it.grounded = true; } }
      if (!it.shown) { this.group.add(it.tag, it.beam); it.shown = true; }
      const dist = Math.hypot(d, camera.position.y - it.y);
      const th = Math.max(26, 20 * it.st.k) * fovK * dist;   // constant on-screen size, never below 26 px tall
      it.tag.position.set(it.x, it.y + 60, it.z); it.tag.scale.set(th * it.aspect, th, 1);
      it.tag.material.opacity = 1;
      it.beam.position.set(it.x, it.y + 30, it.z);
    }
  }
}
const BEAM = new THREE.CylinderGeometry(1.2, 1.2, 60, 8, 1, true);
