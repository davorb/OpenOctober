// 2D art for the loot / pack windows, converted from your own unpacked GUI textures (export/texture/dds/gui) -> data/ui/*.png + data/ui/icons.json
//   scroll.png      the parchment the tombstone window is drawn on       bag.png     pack window background (dark leather/cloth)
//   border-*.png    the bank's metal frame pieces                        icons/*.png the item icons that belong to goods in data/items.json
//   icons.json      { "r_Apple": "icons/apple.png", ... }   (goods without an icon of their own get a bag in the client)
// Needs ImageMagick (`magick`, or `convert`) to read the DDS files. run: node tools/build_ui.js
const fs = require('fs'), path = require('path'), { execFileSync } = require('child_process');
const ROOT = path.join(__dirname, '../..'), GUI = path.join(ROOT, 'export/texture/dds/gui'), OUT = path.join(__dirname, '../data/ui');
if (!fs.existsSync(GUI)) { console.log('no export/texture/dds/gui, skipping the UI art'); process.exit(0); }
let im; for (const c of ['magick', 'convert']) { try { execFileSync(c, ['-version'], { stdio: 'ignore' }); im = c; break; } catch {} }
if (!im) { console.log('ImageMagick not found, skipping the UI art (the windows fall back to plain colours)'); process.exit(0); }
fs.mkdirSync(path.join(OUT, 'icons'), { recursive: true });
const conv = (src, dst) => { try { execFileSync(im, [src, '-strip', dst], { stdio: 'ignore' }); return true; } catch { return false; } };
const jobs = [['resources/sword_01.dds', 'weapon.png'], ['background/background2.dds', 'bag.png'], ['resources/misc_money_bag_01.dds', 'gold.png'], ['resources/item_bag_small.dds', 'generic.png']];
for (const side of ['top', 'bottom', 'left', 'right']) jobs.push([`border/bank/edge_${side}.dds`, `border-${side}.png`]);
for (const c of ['top_left', 'top_right', 'bottom_left', 'bottom_right']) jobs.push([`border/bank/corner_${c}.dds`, `border-${c}.png`]);
fs.copyFileSync(path.join(GUI, 'tutorial/blank_scroll.png'), path.join(OUT, 'scroll.png'));   // already a PNG in the export
for (const [s, d] of jobs) if (!conv(path.join(GUI, s), path.join(OUT, d))) console.log('missing', s);
// icons: gui/resources/<name>.dds; an item "r_Iron Sword" is looked for as iron_sword, iron_sword_01, then the shortest icon that starts with it
const dir = path.join(GUI, 'resources'), have = new Map(fs.readdirSync(dir).filter(f => f.endsWith('.dds')).map(f => [f.slice(0, -4).toLowerCase(), f]));
const names = [...have.keys()].sort((a, b) => a.length - b.length);
const items = JSON.parse(fs.readFileSync(path.join(__dirname, '../data/items.json'), 'utf8')), map = {}; let n = 0;
for (const it of items) {
  const k = it.id.replace(/^r_/, '').toLowerCase().replace(/[^a-z0-9]+/g, '_').replace(/^_|_$/g, '');
  const hit = have.has(k) ? k : have.has(k + '_01') ? k + '_01' : names.find(x => x.startsWith(k + '_'));
  if (!hit) continue;
  const dst = `icons/${hit}.png`;
  if (!fs.existsSync(path.join(OUT, dst)) && !conv(path.join(dir, have.get(hit)), path.join(OUT, dst))) continue;
  map[it.id] = dst; n++;
}
fs.writeFileSync(path.join(OUT, 'icons.json'), JSON.stringify(map));
console.log(`ui art: ${jobs.length} pieces, icons for ${n} of ${items.length} goods`);
