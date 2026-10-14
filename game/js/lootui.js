// The loot window (E next to a tombstone): the tombstone's contents lie scattered on a parchment, Ultima style (no slots), next to a window with what
// you carry. Drag things from the tombstone onto your pack to take them (Shift = just one of a stack). Art: data/ui/ (tools/build_ui.js), plain colours without it.
const UI = '/data/ui/', REACH = 4.5;
const esc = s => String(s).replace(/[&<>"]/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));
const pretty = id => id.replace(/^r_/, '');
function rng(seed) { let a = seed >>> 0; return () => { a = (a + 0x6D2B79F5) >>> 0; let t = a; t = Math.imul(t ^ (t >>> 15), t | 1); t ^= t + Math.imul(t ^ (t >>> 7), t | 61); return ((t ^ (t >>> 14)) >>> 0) / 4294967296; }; }
function hash(s) { let h = 2166136261; for (const c of s) h = Math.imul(h ^ c.charCodeAt(0), 16777619); return h >>> 0; }

const CSS = `
#loot { position:fixed; inset:0; z-index:30; display:none; align-items:center; justify-content:center; gap:18px; pointer-events:none; }
#loot.on { display:flex; } #loot .win { pointer-events:auto; position:relative; user-select:none; -webkit-user-select:none; filter:drop-shadow(0 6px 18px #000a); }
#loot .tomb { width:min(440px,44vw); aspect-ratio:440/620; background:url(${UI}scroll.png) center/100% 100% no-repeat; color:#3b2512; }
#loot .pack { width:min(340px,36vw); aspect-ratio:340/420; background:#17130f url(${UI}bag.png) center/256px; box-shadow:0 0 0 2px #0008 inset; color:#e8d7b0; }
#loot .ttl { position:absolute; left:0; right:0; top:11%; text-align:center; font:600 20px/1.1 serif; letter-spacing:.04em; pointer-events:none; }
#loot .pack .ttl { top:12px; font-size:15px; }
#loot .area { position:absolute; } #loot .tomb .area { left:12%; right:12%; top:20%; bottom:16%; } #loot .pack .area { left:34px; right:34px; top:40px; bottom:34px; }
#loot .foot { position:absolute; left:0; right:0; bottom:11%; text-align:center; font:12px/1.2 sans-serif; opacity:.75; pointer-events:none; } #loot .pack .foot { bottom:8px; color:#e8d7b0; }
#loot .it { position:absolute; width:44px; height:44px; cursor:grab; transition:filter .1s; } #loot .it:hover { filter:brightness(1.25) drop-shadow(0 0 4px #ffd070); }
#loot .it img { width:100%; height:100%; object-fit:contain; pointer-events:none; } #loot .it i { position:absolute; right:-2px; bottom:-2px; font:700 11px/1 sans-serif; font-style:normal; color:#fff; background:#000b; padding:1px 3px; border-radius:3px; pointer-events:none; }
#loot .it.txt { display:flex; align-items:center; justify-content:center; font:10px/1.05 sans-serif; text-align:center; background:#0004; border:1px solid #0006; border-radius:4px; overflow:hidden; }
#loot .pack.hot, #loot .tomb.hot { outline:2px dashed #ffd070; outline-offset:-6px; }
#loot .x { position:absolute; right:10px; top:8px; width:22px; height:22px; border-radius:11px; background:#0008; color:#fff; text-align:center; font:700 14px/22px sans-serif; cursor:pointer; z-index:2; }
#loot .edge { position:absolute; pointer-events:none; background-repeat:repeat; } #loot .cor { position:absolute; width:32px; height:32px; pointer-events:none; }
#lootghost { position:fixed; z-index:40; width:44px; height:44px; pointer-events:none; opacity:.85; display:none; }
`;

export class LootUI {
  constructor({ mp, $ }) {
    this.mp = mp; this.cid = null; this.grave = null; this.pos = { tomb: new Map(), pack: new Map() }; this.icons = {};
    fetch(UI + 'icons.json').then(r => r.ok ? r.json() : {}).then(j => { this.icons = j; this.render(); }).catch(() => {});
    const st = document.createElement('style'); st.textContent = CSS; document.head.appendChild(st);
    this.root = document.createElement('div'); this.root.id = 'loot'; document.body.appendChild(this.root);
    this.ghost = document.createElement('div'); this.ghost.id = 'lootghost'; document.body.appendChild(this.ghost);
    addEventListener('keydown', e => { if (this.cid !== null && e.code === 'Escape') this.close(); }, true);
    setInterval(() => this.watch(), 250);
  }
  get isOpen() { return this.cid !== null; }
  /** The nearest tombstone within reach that still has something on it. */
  nearest() {
    const cam = this.mp.camera.position; let best = null, bd = REACH;
    for (const [cid, c] of this.mp.corpses) { if (!c.tomb || !(c.gold > 0 || c.units > 0)) continue; const d = Math.hypot(c.x - cam.x, c.z - cam.z); if (d < bd) { bd = d; best = cid; } }
    return best;
  }
  toggle() { if (this.isOpen) this.close(); else { const cid = this.nearest(); if (cid !== null) this.open(cid); } }
  open(cid) { this.cid = cid; this.grave = null; document.exitPointerLock?.(); this.mp.sendJson({ t: 'opengrave', cid }); }
  close() { if (this.cid === null) return; this.cid = null; this.grave = null; this.root.classList.remove('on'); this.root.innerHTML = ''; this.mp.$('view').requestPointerLock?.()?.catch?.(() => {}); }
  watch() {
    if (this.cid === null) return; const c = this.mp.corpses.get(this.cid), cam = this.mp.camera.position;
    if (!c || Math.hypot(c.x - cam.x, c.z - cam.z) > REACH + 1.5) this.close();
  }
  /** Server messages: `grave` (what is on it now), `graveerr`. Returns true when handled. */
  onMessage(m) {
    if (m.t === 'grave') { if (m.cid !== this.cid) return true; this.grave = m; const c = this.mp.corpses.get(m.cid); if (c) { c.gold = m.gold; c.units = m.items.reduce((a, i) => a + i[1], 0); } this.render(); return true; }
    if (m.t === 'graveerr') { if (m.cid === this.cid) { this.mp.log(m.reason === 'too heavy' ? 'You cannot carry any more' : m.reason === 'too far' ? 'Too far from the tombstone' : 'Nothing there to take', 'sys'); if (m.reason !== 'too heavy') this.close(); } return true; }
    if (m.t === 'looted' && m.cid === this.cid) { this.mp.log('The tombstone is empty', 'sys'); this.close(); return false; }
    if (m.t === 'stock' && this.cid !== null) queueMicrotask(() => this.render());
    return false;
  }
  frame(win) {
    const mk = (cls, css) => { const d = document.createElement('div'); d.className = cls; Object.assign(d.style, css); win.appendChild(d); };
    const e = (side, css) => mk('edge', { backgroundImage: `url(${UI}border-${side}.png)`, ...css });
    e('top', { left: '32px', right: '32px', top: 0, height: '32px', backgroundSize: 'auto 32px', backgroundRepeat: 'repeat-x' }); e('bottom', { left: '32px', right: '32px', bottom: 0, height: '32px', backgroundSize: 'auto 32px', backgroundRepeat: 'repeat-x' });
    e('left', { top: '32px', bottom: '32px', left: 0, width: '32px', backgroundSize: '32px auto', backgroundRepeat: 'repeat-y' }); e('right', { top: '32px', bottom: '32px', right: 0, width: '32px', backgroundSize: '32px auto', backgroundRepeat: 'repeat-y' });
    for (const [c, css] of [['top_left', { left: 0, top: 0 }], ['top_right', { right: 0, top: 0 }], ['bottom_left', { left: 0, bottom: 0 }], ['bottom_right', { right: 0, bottom: 0 }]]) mk('cor', { backgroundImage: `url(${UI}border-${c}.png)`, ...css });
  }
  /** Scatter `things` over `area` (a stable random spot per thing: same tombstone, same layout; what the player has moved stays where it was put). */
  place(kind, area, things, salt) {
    const W = area.clientWidth || 260, H = area.clientHeight || 300, map = this.pos[kind], taken = [...map.values()];
    for (const t of things) {
      if (!map.has(t.key)) {
        const r = rng(hash(salt + t.key)); let best = null, bd = -1;
        for (let i = 0; i < 30; i++) { const p = { x: r() * (W - 44), y: r() * (H - 44) }, d = taken.reduce((m, q) => Math.min(m, Math.hypot(q.x - p.x, q.y - p.y)), 1e9); if (d > bd) { bd = d; best = p; if (d > 48) break; } }
        map.set(t.key, best); taken.push(best);
      }
      const p = map.get(t.key), n = document.createElement('div'), src = t.gold ? UI + 'gold.png' : this.icons[t.id] ? UI + this.icons[t.id] : null;
      n.className = 'it' + (src ? '' : ' txt'); n.style.left = Math.min(p.x, W - 44) + 'px'; n.style.top = Math.min(p.y, H - 44) + 'px';
      n.title = t.gold ? `${Math.round(t.qty)} gold` : `${pretty(t.id)} × ${t.qty}${t.w ? ` (${(t.w * t.qty).toFixed(1)} weight)` : ''}`;
      n.innerHTML = (src ? `<img src="${src}" alt="" onerror="this.replaceWith(Object.assign(document.createElement('span'),{textContent:'${t.gold ? 'gold' : 'item'}'}))">` : `<span>${esc(t.gold ? 'gold' : pretty(t.id))}</span>`) + (t.qty > 1 ? `<i>${t.gold ? Math.round(t.qty) : t.qty}</i>` : '');
      area.appendChild(n); this.drag(n, kind, t, area);
    }
  }
  drag(n, kind, t, area) {
    n.addEventListener('pointerdown', e => {
      if (e.button !== 0) return; e.preventDefault(); const g = this.ghost, r0 = n.getBoundingClientRect(); let moved = false;
      const sx = e.clientX, sy = e.clientY, move = ev => {
        if (!moved && Math.hypot(ev.clientX - sx, ev.clientY - sy) < 4) return; moved = true;
        g.innerHTML = n.innerHTML; g.style.display = 'block'; g.style.left = ev.clientX - 22 + 'px'; g.style.top = ev.clientY - 22 + 'px'; n.style.opacity = .3;
        const over = this.winAt(ev); this.root.querySelectorAll('.win').forEach(w => w.classList.toggle('hot', kind === 'tomb' && w === over && w !== area.parentNode));
      }, up = ev => {
        removeEventListener('pointermove', move); removeEventListener('pointerup', up); g.style.display = 'none'; n.style.opacity = 1; this.root.querySelectorAll('.win').forEach(w => w.classList.remove('hot'));
        if (!moved) return; const over = this.winAt(ev), pr = area.getBoundingClientRect();
        if (over === area.parentNode) { this.pos[kind].set(t.key, { x: Math.max(0, Math.min(pr.width - 44, ev.clientX - pr.left - 22)), y: Math.max(0, Math.min(pr.height - 44, ev.clientY - pr.top - 22)) }); this.render(); }   // moved about inside its own window
        else if (over && kind === 'tomb') this.mp.sendJson({ t: 'take', cid: this.cid, item: t.gold ? '' : t.id, qty: ev.shiftKey ? 1 : t.gold ? 1 : t.qty });   // onto the pack: take it
        else if (over && kind === 'pack') this.mp.log('Things you put down are not kept for others', 'sys');
      };
      addEventListener('pointermove', move); addEventListener('pointerup', up);
    });
  }
  winAt(e) { return [...this.root.querySelectorAll('.win')].find(w => { const r = w.getBoundingClientRect(); return e.clientX >= r.left && e.clientX <= r.right && e.clientY >= r.top && e.clientY <= r.bottom; }); }
  render() {
    if (this.cid === null || !this.grave) return; const g = this.grave, mp = this.mp;
    this.root.innerHTML = ''; this.root.classList.add('on');
    const tomb = document.createElement('div'); tomb.className = 'win tomb'; tomb.innerHTML = `<div class="ttl">${esc(g.name || 'Tombstone')}</div><div class="x">×</div><div class="area"></div><div class="foot">Drag things onto your pack · Shift: just one · E or Esc: close</div>`;
    const pack = document.createElement('div'); pack.className = 'win pack'; this.frame(pack);
    const stock = mp.stock || { gold: 0, items: {} }; pack.insertAdjacentHTML('beforeend', `<div class="ttl">Your pack</div><div class="area"></div><div class="foot">${mp.stockMsg ? `weight ${mp.stockMsg.weight.toFixed(1)} / ${mp.stockMsg.limit}` : ''}</div>`);
    this.root.append(tomb, pack); tomb.querySelector('.x').onclick = () => this.close();
    const wOf = Object.fromEntries(g.items.map(([k, , w]) => [k, w]));
    const tl = g.items.map(([id, qty, w]) => ({ key: id, id, qty, w })); if (g.gold > 0) tl.unshift({ key: '$gold', gold: true, qty: g.gold });
    const pl = Object.entries(stock.items).map(([id, qty]) => ({ key: id, id, qty, w: wOf[id] })); if (stock.gold > 0) pl.unshift({ key: '$gold', gold: true, qty: stock.gold });
    this.place('tomb', tomb.querySelector('.area'), tl, 'tomb' + this.cid); this.place('pack', pack.querySelector('.area'), pl, 'pack');
    for (const k of [...this.pos.tomb.keys()]) if (!tl.some(t => t.key === k)) this.pos.tomb.delete(k);
  }
}
