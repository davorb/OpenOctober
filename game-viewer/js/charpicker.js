// Character creation widget (sign-up screen): name, race, body (everybody starts naked) and a live rotating preview. Edits `profile` in place.
import * as THREE from 'three';
import { RACES, AVATAR_HEIGHT, pose } from './avatars.js';

const el = (tag, cls, html) => { const e = document.createElement(tag); if (cls) e.className = cls; if (html != null) e.innerHTML = html; return e; };

export class CharacterPicker {
  constructor({ box, factory, profile }) {
    this.box = box; this.factory = factory; this.profile = profile; this.pav = null; this.pk = 0; this.alive = true;
    box.id = 'mp';
    box.innerHTML = `<canvas id="mpprev" width="190" height="262"></canvas><div class="f"><div class="t">YOUR CHARACTER</div>
      <input id="mpname" maxlength="16" placeholder="Character name" spellcheck="false" autocomplete="off"><div class="t">RACE</div><div id="mprace" class="opts"></div>
      <div class="t">BODY</div><div id="mpgender" class="opts"></div></div>`;
    const P = profile, $ = id => box.querySelector('#' + id);
    const name = $('mpname'); name.value = P.name || ''; name.addEventListener('input', () => { P.name = name.value; });
    this.opts = (id, list, key) => {
      const c = $(id); c.innerHTML = '';
      list.forEach(([v, label]) => {
        const b = el('div', 'o' + (P[key] === v ? ' on' : ''), label);
        b.addEventListener('click', () => { P[key] = v; [...c.children].forEach(x => x.classList.toggle('on', x === b)); this.refreshPreview(); });
        c.appendChild(b);
      });
    };
    this.opts('mprace', RACES.map(r => [r.id, r.name]), 'race'); this.opts('mpgender', [['m', 'Male'], ['f', 'Female']], 'gender'); 
    const cv = $('mpprev'); this.pr = new THREE.WebGLRenderer({ canvas: cv, antialias: true, alpha: true }); this.pr.setPixelRatio(Math.min(devicePixelRatio, 2)); this.pr.setSize(190, 262, false);
    cv.style.width = '190px'; cv.style.height = '262px'; this.pr.outputColorSpace = THREE.SRGBColorSpace;
    this.ps = new THREE.Scene(); this.pc = new THREE.PerspectiveCamera(28, 190 / 262, 0.5, 50); this.pc.position.set(0, AVATAR_HEIGHT * 0.52, 7.4);
    this.ps.add(new THREE.HemisphereLight(0xfff2dc, 0x6a5a48, 3.2)); const d = new THREE.DirectionalLight(0xffe2b0, 3.2); d.position.set(2, 4, 6); this.ps.add(d);
    this.loop(); this.refreshPreview();
  }
  async refreshPreview() {
    const k = ++this.pk, { race, gender } = this.profile, gear = '';
    try {
      const av = await this.factory.build(race, gender, gear);
      if (k !== this.pk || !this.alive) return;
      if (this.pav) this.ps.remove(this.pav.root);
      this.pav = av; av.root.position.y = -0.05; this.ps.add(av.root);
    } catch (e) { console.warn('preview', e); }
  }
  loop() {
    if (!this.alive) return;
    requestAnimationFrame(() => this.loop());
    if (!this.box.offsetParent) return;   // hidden (the log-in tab): nothing to draw
    if (this.pav) { this.pav.root.rotation.y += 0.012; pose(this.pav, 0, 0, false); }
    this.pr.render(this.ps, this.pc);
  }
  dispose() { this.alive = false; this.pr.dispose(); }
}
