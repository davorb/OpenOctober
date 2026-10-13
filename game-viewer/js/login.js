// Log-in / sign-up screen. Opens the game server's websocket as soon as the page loads (so it is ready when the player has typed) and resolves
// `loggedIn` with { ws, creds, auth } once the server accepted the account; main.js has been downloading the world meanwhile.
//   player page: tabs LOG IN / SIGN UP (username, email, password and the character: name, race, body, armour)
//   admin page:  log in only, and the account has to be an administrator
// The password is kept in memory only, to log in again after a dropped connection; it is never stored.
import { serverUrl, factory } from './net.js';
import { CharacterPicker } from './charpicker.js';

const $ = id => document.getElementById(id);
const ADMIN = document.body.dataset.mode === 'admin';
const el = (tag, attrs = {}, html = '') => Object.assign(document.createElement(tag), attrs, { innerHTML: html });

let ws = null, waiter = null, picker = null, resolveLoggedIn;
export const loggedIn = new Promise(r => (resolveLoggedIn = r));
const remembered = () => { try { return localStorage.getItem('dfUser') || ''; } catch { return ''; } };

// ---- socket
function netStatus(text, up = false) { const n = $('authnet'); if (n) { n.textContent = text; n.classList.toggle('up', up); } }
function open() {
  let s; try { s = new WebSocket(serverUrl()); } catch { netStatus('Server address invalid'); return; }
  ws = s; netStatus('Connecting to the server…');
  s.onopen = () => netStatus('● Server reachable', true);
  s.onmessage = e => {
    if (typeof e.data !== 'string') return;
    let m; try { m = JSON.parse(e.data); } catch { return; }
    if (waiter && waiter.types.includes(m.t)) { const w = waiter; waiter = null; w.resolve(m); }
  };
  s.onclose = () => { if (ws !== s) return; ws = null; if (waiter) { const w = waiter; waiter = null; w.resolve({ t: 'autherr', msg: 'Connection lost' }); } netStatus('○ Server not reachable — retrying…'); setTimeout(open, 3000); };
  s.onerror = () => {};
}
const ready = () => new Promise(res => { const t = setInterval(() => { if (ws?.readyState === 1) { clearInterval(t); res(true); } }, 100); setTimeout(() => { clearInterval(t); res(false); }, 8000); });
async function request(msg, types) {
  if (!(await ready())) return { t: 'autherr', msg: 'Cannot reach the server' };
  return new Promise(resolve => { waiter = { types, resolve }; ws.send(JSON.stringify(msg)); });
}
/** the socket has a session (a wrong kind of account logged in): throw it away and start over on a fresh one */
function resetSocket() { const s = ws; ws = null; s?.close(); open(); }

// ---- forms
function build() {
  const box = $('auth');
  box.innerHTML = `${ADMIN ? '' : '<div class="tabs"><div class="tab on" data-tab="login">LOG IN</div><div class="tab" data-tab="signup">SIGN UP</div></div>'}
    <form id="f_login" autocomplete="on">
      <label>USERNAME<input name="username" autocomplete="username" maxlength="20" spellcheck="false" required></label>
      <label>PASSWORD<input name="password" type="password" autocomplete="current-password" maxlength="128" required></label>
      <button type="submit">${ADMIN ? 'ADMIN LOG IN' : 'ENTER THE WORLD'}</button>
    </form>
    ${ADMIN ? '' : `<form id="f_signup" class="wide" autocomplete="on" hidden>
      <div class="signrow">
        <label>USERNAME<input name="username" autocomplete="username" maxlength="20" spellcheck="false" required></label>
        <label>EMAIL<input name="email" type="email" autocomplete="email" maxlength="100" required></label>
      </div>
      <div class="signrow">
        <label>PASSWORD (8+ CHARACTERS)<input name="password" type="password" autocomplete="new-password" minlength="8" maxlength="128" required></label>
        <label>REPEAT PASSWORD<input name="again" type="password" autocomplete="new-password" minlength="8" maxlength="128" required></label>
      </div>
      <div id="picker"></div>
      <button type="submit">CREATE CHARACTER</button>
    </form>`}
    <div class="msg" id="authmsg"></div><div class="net" id="authnet"></div>`;
  const msg = (t, ok = false) => { const m = $('authmsg'); m.textContent = t; m.classList.toggle('ok', ok); };
  const busy = on => box.querySelectorAll('button').forEach(b => (b.disabled = on));
  const tabs = [...box.querySelectorAll('.tab')];
  const profile = { name: '', race: 'hum', gender: 'm', gear: '' };
  const show = which => {
    tabs.forEach(t => t.classList.toggle('on', t.dataset.tab === which)); $('f_login').hidden = which !== 'login'; if ($('f_signup')) $('f_signup').hidden = which !== 'signup'; msg('');
    if (which === 'signup' && !picker) picker = new CharacterPicker({ box: $('picker'), factory, profile });
  };
  tabs.forEach(t => t.addEventListener('click', () => show(t.dataset.tab)));

  async function logIn(username, password) {
    const r = await request({ t: 'login', username, password }, ['auth', 'autherr']);
    if (r.t === 'autherr') { msg(r.msg); return false; }
    if (ADMIN && !r.admin) { msg('This account is not an administrator.'); resetSocket(); return false; }
    try { localStorage.setItem('dfUser', username); } catch {}
    picker?.dispose();
    const s = ws; s.onclose = null; ws = null; waiter = null;   // handed over to the game
    resolveLoggedIn({ ws: s, creds: { username, password }, auth: r });
    return true;
  }
  $('f_login').addEventListener('submit', async e => {
    e.preventDefault(); const f = new FormData(e.target); busy(true); msg('Logging in…', true);
    if (!await logIn(f.get('username').trim(), f.get('password'))) busy(false);
  });
  $('f_login').elements.username.value = remembered();
  if (remembered()) $('f_login').elements.password.focus(); else $('f_login').elements.username.focus();
  if (!ADMIN) $('f_signup').addEventListener('submit', async e => {
    e.preventDefault(); const f = new FormData(e.target), username = f.get('username').trim(), password = f.get('password');
    if (password !== f.get('again')) return msg('The passwords differ.');
    if (profile.name.trim().length < 2) return msg('Give your character a name.');
    busy(true); msg('Creating your character…', true);
    const r = await request({ t: 'register', username, email: f.get('email').trim(), password, ...profile, name: profile.name.trim() }, ['registered', 'autherr']);
    if (r.t === 'autherr') { msg(r.msg); busy(false); return; }
    if (!await logIn(username, password)) busy(false);
  });
}

build();
open();
