// On-screen touch controls for phones / tablets (mobile-FPS style): floating move stick on the left half of the screen,
// drag-to-look on the right half, plus jump / descend / sprint / mode / chat buttons. Only built on touch devices
// (coarse primary pointer), or with ?touch=1 for testing on a desktop; ?touch=0 forces them off.
const SENS = 0.0052;      // rad per px of drag
const STICK_R = 62;       // px of stick travel
const DEAD = 0.12;

export function touchWanted() {
  const f = new URLSearchParams(location.search).get('touch');
  if (f) return f !== '0';
  return matchMedia('(pointer: coarse)').matches && navigator.maxTouchPoints > 0;
}

export class TouchControls {
  // keys: the shared pressed-key Set. look(dx, dy): rotate the camera by pixels dragged.
  constructor({ keys, look }) {
    this.keys = keys; this.lookFn = look;
    this.x = 0; this.y = 0;          // analog move input, y < 0 = forward (like W)
    this.sprint = false;
    document.body.classList.add('touch');
    this.build();
  }

  build() {
    const root = this.root = document.createElement('div'); root.id = 'touch';
    root.innerHTML = `
      <div id="tstick"><i></i></div>
      <div class="tb" id="tb-chat" style="left:calc(170px + env(safe-area-inset-left));top:calc(10px + env(safe-area-inset-top))">Chat</div>
      <div class="tb" id="tb-players" style="left:calc(228px + env(safe-area-inset-left));top:calc(10px + env(safe-area-inset-top))">Who</div>
      <div class="tb" id="tb-menu" style="right:calc(14px + env(safe-area-inset-right));top:calc(64px + env(safe-area-inset-top))">⚙</div>
      <div class="tb big" id="tb-jump" style="right:calc(26px + env(safe-area-inset-right));bottom:calc(120px + env(safe-area-inset-bottom))">▲</div>
      <div class="tb big fly" id="tb-down" style="right:calc(26px + env(safe-area-inset-right));bottom:calc(30px + env(safe-area-inset-bottom))">▼</div>
      <div class="tb" id="tb-sprint" style="right:calc(112px + env(safe-area-inset-right));bottom:calc(60px + env(safe-area-inset-bottom))">Run</div>
      <div class="tb" id="tb-mode" style="right:calc(112px + env(safe-area-inset-right));bottom:calc(122px + env(safe-area-inset-bottom))">Fly</div>
      <div class="tb sm fly" id="tb-faster" style="right:calc(200px + env(safe-area-inset-right));bottom:calc(122px + env(safe-area-inset-bottom))">＋</div>
      <div class="tb sm fly" id="tb-slower" style="right:calc(200px + env(safe-area-inset-right));bottom:calc(60px + env(safe-area-inset-bottom))">－</div>`;
    document.body.appendChild(root);
    const $ = id => root.querySelector('#' + id);
    this.stick = $('tstick'); this.knob = this.stick.firstElementChild;

    // hold buttons -> shared key set
    const hold = (id, code) => {
      const el = $(id), up = e => { this.keys.delete(code); el.classList.remove('on'); };
      el.addEventListener('pointerdown', e => { e.preventDefault(); this.keys.add(code); el.classList.add('on'); el.setPointerCapture(e.pointerId); });
      el.addEventListener('pointerup', up); el.addEventListener('pointercancel', up);
    };
    hold('tb-jump', 'Space'); hold('tb-down', 'KeyC');
    // tap buttons
    const tap = (id, fn) => $(id).addEventListener('pointerdown', e => { e.preventDefault(); fn(e); });
    const key = code => () => dispatchEvent(new KeyboardEvent('keydown', { code, bubbles: true }));
    tap('tb-chat', key('Enter')); tap('tb-players', key('Tab')); tap('tb-menu', key('KeyO')); tap('tb-mode', key('KeyG'));
    tap('tb-faster', () => dispatchEvent(new WheelEvent('wheel', { deltaY: -250 })));
    tap('tb-slower', () => dispatchEvent(new WheelEvent('wheel', { deltaY: 250 })));
    tap('tb-sprint', () => { this.sprint = !this.sprint; $('tb-sprint').classList.toggle('on', this.sprint); });

    // stick (left half) + look (right half) on the canvas, tracked per pointer so both thumbs work at once
    const view = document.getElementById('view'); view.style.touchAction = 'none';
    let stickId = null, lookId = null, ox = 0, oy = 0, lx = 0, ly = 0;
    view.addEventListener('pointerdown', e => {
      if (e.pointerType === 'mouse' || !document.body.classList.contains('playing')) return;
      e.preventDefault(); view.setPointerCapture(e.pointerId);
      if (e.clientX < innerWidth * 0.45 && stickId === null) {
        stickId = e.pointerId; ox = e.clientX; oy = e.clientY;
        this.stick.style.cssText = `display:block;left:${ox - STICK_R}px;top:${oy - STICK_R}px`; this.knob.style.transform = '';
      } else if (lookId === null) { lookId = e.pointerId; lx = e.clientX; ly = e.clientY; }
    });
    view.addEventListener('pointermove', e => {
      if (e.pointerId === stickId) {
        let dx = e.clientX - ox, dy = e.clientY - oy; const d = Math.hypot(dx, dy);
        if (d > STICK_R) { dx *= STICK_R / d; dy *= STICK_R / d; }
        this.knob.style.transform = `translate(${dx}px,${dy}px)`;
        const m = Math.hypot(dx, dy) / STICK_R;
        if (m < DEAD) { this.x = this.y = 0; } else { const k = (m - DEAD) / (1 - DEAD) / m; this.x = dx / STICK_R * k * 1.0; this.y = dy / STICK_R * k * 1.0; }
      } else if (e.pointerId === lookId) {
        this.lookFn((e.clientX - lx) * SENS, (e.clientY - ly) * SENS); lx = e.clientX; ly = e.clientY;
      }
    });
    const end = e => {
      if (e.pointerId === stickId) { stickId = null; this.x = this.y = 0; this.stick.style.display = 'none'; }
      if (e.pointerId === lookId) lookId = null;
    };
    view.addEventListener('pointerup', end); view.addEventListener('pointercancel', end);
    addEventListener('blur', () => { this.x = this.y = 0; });
    // iOS: stop pinch-zoom / double-tap zoom / rubber-banding from fighting the game
    document.addEventListener('gesturestart', e => e.preventDefault());
    document.addEventListener('touchmove', e => { if (e.target === view) e.preventDefault(); }, { passive: false });
    document.addEventListener('contextmenu', e => e.preventDefault());
  }

  setMode(mode) { document.body.classList.toggle('walk', mode === 'walk'); this.root.querySelector('#tb-mode').textContent = mode === 'walk' ? 'Walk' : 'Fly'; }
}
