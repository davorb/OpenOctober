// Stamina: sprinting drains it continuously, each jump costs a chunk, and it regenerates after a short pause.
// Running out locks sprint until it recovers past RECOVER, so it can't be feathered at zero.
export const STAMINA = {
  max: 100,
  sprintDrain: 20,   // per second while sprinting (about 5 s from full)
  jumpCost: 15,
  regen: 14,         // per second
  regenDelay: 1.0,   // seconds after spending before regen starts
  recover: 25,       // after exhaustion, sprint stays locked until stamina reaches this
};

export class Stamina {
  constructor() {
    this.value = STAMINA.max; this.delay = 0; this.exhausted = false;
    const css = document.createElement('style');
    css.textContent = `#stam{position:fixed;left:50%;bottom:76px;transform:translateX(-50%);width:240px;height:6px;background:#0009;border:1px solid #000a;border-radius:2px;overflow:hidden;pointer-events:none;display:none;z-index:6;transition:opacity .4s}
      body.playing #stam{display:block}#stam.full{opacity:0}
      #stamfill{height:100%;width:100%;background:linear-gradient(#cdb24a,#7a6a22)}#stam.low #stamfill{background:linear-gradient(#d86a2a,#7a2a12)}`;
    document.head.appendChild(css);
    this.el = document.createElement('div'); this.el.id = 'stam'; this.el.innerHTML = '<div id="stamfill"></div>';
    document.body.appendChild(this.el); this.fill = this.el.firstChild;
  }
  get canSprint() { return !this.exhausted && this.value > 0; }
  canJump() { return this.value >= STAMINA.jumpCost; }
  spend(n) { this.value = Math.max(0, this.value - n); this.delay = STAMINA.regenDelay; if (this.value <= 0) this.exhausted = true; }
  /** sprinting: true while the player is actually sprinting this frame */
  update(dt, sprinting) {
    if (sprinting) this.spend(STAMINA.sprintDrain * dt);
    else if (this.delay > 0) this.delay -= dt;
    else this.value = Math.min(STAMINA.max, this.value + STAMINA.regen * dt);
    if (this.exhausted && this.value >= STAMINA.recover) this.exhausted = false;
    this.fill.style.width = (100 * this.value / STAMINA.max) + '%';
    this.el.classList.toggle('low', this.exhausted || this.value < STAMINA.recover);
    this.el.classList.toggle('full', this.value >= STAMINA.max);
  }
  reset() { this.value = STAMINA.max; this.delay = 0; this.exhausted = false; }
}
