// Which static-mesh subsets are solid, so the viewer does not alpha-test them.
// The glTF exporter sets alphaMode MASK on every RGBA texture. The game only clips when the
// subset shader defines ALPHATEST (foliage, banners, cages, armour-sharp). Alphametal and the
// default prop shader read alpha as metal/specular and draw the surface solid.
// -> data/alphamode.json  { all: [mesh.sfm, ...], solid: [mesh.sfm_subset, ...], keep: [mesh.sfm_subset, ...] }
//   all   : every subset is solid (a global shader), except names in keep
//   solid : this subset is solid
//   keep  : this subset alpha-tests or blends; do not force it solid
const fs = require('fs'), path = require('path');
const ROOT = path.resolve(__dirname, '../..');
const XML = path.join(ROOT, 'export/xml');
const TEQ = path.join(__dirname, '../data/shaders');
const SHADER_DIR = path.join(XML, 'shader');

function indexDir(dir, ext) {
  const m = new Map();
  for (const f of fs.readdirSync(dir)) if (f.toLowerCase().endsWith(ext)) m.set(f.toLowerCase(), path.join(dir, f));
  return m;
}
const shaderXml = indexDir(SHADER_DIR, '.xml');
const teqFiles = indexDir(TEQ, '.teq');

function readText(p) {
  const raw = fs.readFileSync(p);
  const z = raw.indexOf(0);
  return (z >= 0 ? raw.subarray(0, z) : raw).toString('latin1').replace(/\/\*[\s\S]*?\*\//g, '').replace(/\/\/.*$/gm, '');
}
function teqText(name, depth = 0, seen = new Set()) {
  const key = name.toLowerCase();
  if (depth > 6 || seen.has(key)) return '';
  seen.add(key);
  const p = teqFiles.get(key);
  if (!p) return '';
  const text = readText(p);
  let body = text;
  for (const m of text.matchAll(/#include\s+"([^"]+\.teq)"/gi)) body += '\n' + teqText(m[1], depth + 1, seen);
  return body;
}
// c = alpha test, b = blended (leave the exported mask alone), s = solid (alpha is not opacity)
function classifyShader(file) {
  const p = shaderXml.get(file.toLowerCase());
  if (!p) return null;
  const xml = readText(p);
  const fx = /<RenderEffect>[\s\S]*?<SFString Value="([^"]+)"/i.exec(xml);
  if (!fx) return null;
  const body = teqText(fx[1].trim());
  if (!body) return null;
  if (/^\s*#define\s+ALPHATEST\b/m.test(body) || /^\s*AlphaTestEnable\s*=\s*true\b/mi.test(body)) return 'c';
  if (/^\s*AlphaBlendEnable\s*=\s*true\b/mi.test(body)) return 'b';
  return 's';
}
const shaderMode = new Map();
function modeOf(file) {
  const k = file.toLowerCase();
  if (!shaderMode.has(k)) shaderMode.set(k, classifyShader(k));
  return shaderMode.get(k);
}

function walk(dir, out) {
  for (const ent of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, ent.name);
    if (ent.isDirectory()) { if (ent.name.toLowerCase() !== 'shader') walk(p, out); }
    else if (ent.name.toLowerCase().endsWith('.xml')) out.push(p);
  }
  return out;
}

// mesh -> { star: Set of modes that apply to every subset, slots: Map<index, Set> }
const meshes = new Map();
function vote(mesh, index, mode) {
  if (!mode) return;
  let e = meshes.get(mesh);
  if (!e) meshes.set(mesh, e = { star: new Set(), slots: new Map() });
  if (index == null) e.star.add(mode);
  else {
    let s = e.slots.get(index);
    if (!s) e.slots.set(index, s = new Set());
    s.add(mode);
  }
}
const globalRe = /<GlobalShader>\s*<SFBool Value="([^"]+)"\s*\/>\s*<SFString Value="([^"]+)"/i;
const fileNameRe = /<FileName>([\s\S]*?)<\/FileName>/i;
const subsetRe = /<Shader(\d+)>\s*<SFString Value="([^"]+)"/gi;
let objects = 0, voted = 0;
for (const p of walk(XML, [])) {
  const t = fs.readFileSync(p, 'latin1');
  if (!t.includes('<Shaders>')) continue;
  const fn = fileNameRe.exec(t);
  const mesh = fn && /<SFString Value="([^"]+\.sfm)"/i.exec(fn[1]);
  if (!mesh) continue;
  const g = globalRe.exec(t);
  if (!g) continue;
  objects++;
  const globalMode = modeOf(g[2].trim());
  const name = path.basename(mesh[1]).toLowerCase();
  if (g[1].toLowerCase() === 'true') { vote(name, null, globalMode); voted++; continue; }   // template SubsetShaders are not the real assignment
  const ns = /<NumberOfSubsets>\s*<SFInteger Value="(\d+)"/i.exec(t);
  const n = ns ? +ns[1] : 0;
  let any = false;
  for (const m of t.matchAll(subsetRe)) {
    const i = +m[1];
    if (i >= n) continue;                                          // Shader1/Shader2 past the subset list is boilerplate
    vote(name, i, modeOf(m[2].trim()));
    any = true;
  }
  if (!any && globalMode) vote(name, null, globalMode);
  if (any || globalMode) voted++;
}

const all = [], solid = [], keep = [];
for (const [mesh, e] of meshes) {
  const veto = new Set();
  for (const [i, modes] of e.slots) if ([...modes].some(m => m !== 's')) veto.add(i);
  const starSolid = e.star.size === 1 && e.star.has('s');
  const starOther = [...e.star].some(m => m !== 's');
  if (starSolid && !starOther) {
    all.push(mesh);
    for (const i of veto) keep.push(mesh + '_' + i);
    continue;
  }
  const idxs = new Set([...e.slots.keys()]);
  for (const i of idxs) {
    const modes = new Set(e.slots.get(i));
    for (const m of e.star) modes.add(m);
    if (modes.size === 1 && modes.has('s')) solid.push(mesh + '_' + i);
    else if ([...modes].some(m => m !== 's')) keep.push(mesh + '_' + i);
  }
}
all.sort(); solid.sort(); keep.sort();
const out = { all, solid, keep };
fs.writeFileSync(path.join(__dirname, '../data/alphamode.json'), JSON.stringify(out));
console.log(`objects ${objects}, voted ${voted}, shaders ${shaderMode.size}, all-solid meshes ${all.length}, solid subsets ${solid.length}, keep ${keep.length}`);
