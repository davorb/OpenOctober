// The player's own body, seen when a weapon is drawn (third person: the camera orbits behind; sheathed or a staff = first person, no body).  Built like everybody else's
// avatar (js/avatars.js) from the chosen character, animated with the game's clips (js/playeranim.js): locomotion from the real movement (forward, backward, sideways), attacks /
// draws / guards played on top, the weapon in the hand.
import { PlayerAnim } from './playeranim.js';

export class SelfAvatar {
  constructor({ scene, factory, profile }) { Object.assign(this, { scene, factory, profile }); this.av = null; this.anim = null; this.building = false; this.shown = false; this.w = null; this.drawn = false; }
  async ensure() {
    if (this.av || this.building) return; this.building = true;
    try {
      const P = this.profile(), av = await this.factory.build(P.race, P.gender, P.gear); av.root.traverse(n => { if (n.isMesh) n.castShadow = true; });
      this.av = av; this.anim = await PlayerAnim.create(av).catch(() => null); await this.wield(this.w, this.drawn);
    } catch (e) { console.warn('self avatar', e); } finally { this.building = false; }
  }
  /** weapon (a weapons.json entry or null) in hand, drawn or sheathed */
  async wield(w, drawn) {
    this.w = w; this.drawn = drawn; if (!this.av) return;
    await this.factory.wield(this.av, w && drawn ? w : null); this.anim?.setWeapon(w?.group || null); this.anim?.setDrawn(!!(w && drawn));
  }
  play(clip, rate = 1) { return this.anim?.play(clip, rate) || 0; }
  show(on) { if (on) this.ensure(); if (on === this.shown || !this.av) return; this.shown = on; if (on) this.scene.add(this.av.root); else this.scene.remove(this.av.root); }
  /** pos = feet (three space), yaw = camera yaw, move = { x: sideways, z: forward } in the body's frame */
  update(dt, pos, yaw, speed, air, swim, move) {
    if (!this.shown || !this.av) return;
    this.av.root.position.copy(pos); this.av.root.rotation.y = yaw + Math.PI;   // meshes face +Z, the camera looks along -Z
    this.anim?.update(dt, speed, air, swim, move);
  }
}
