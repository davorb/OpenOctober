// Build runtime JSON for the viewer from the unpacked game data in ../data
//   music.json  - areas (tile lists) with per-time-of-day track lists, from zones/env_music_settings.xml
//   tiles.json  - tile display names, from gamelogicinfo/tileinfomap.xml
//   env.json    - time-of-day environment configs + keyframes (sun/moons, fog, atmosphere), from objectspace/environment
const fs = require('fs'), path = require('path');
const D = path.resolve(__dirname, '../data');
const rd = f => fs.readFileSync(path.join(D, f), 'latin1');
const AUDIO = path.resolve(__dirname, '../../export/audio/ambience');
const haveAudio = new Set(fs.readdirSync(AUDIO));

// ---- music
{
  const xml = rd('zones/env_music_settings.xml');
  const areas = [];
  for (const m of xml.matchAll(/<Area Name="([^"]*)"([^>]*)>([\s\S]*?)<\/Area>/g)) {
    const tiles = ((m[2].match(/Tiles="([^"]*)"/) || [])[1] || '').split(',').map(s => s.trim().toLowerCase()).filter(Boolean);
    const a = { name: m[1], tiles };
    for (const t of ['Sunrise', 'Midday', 'Sunset', 'Midnight']) {
      const blk = (m[3].match(new RegExp('<' + t + '>([\\s\\S]*?)</' + t + '>')) || [])[1] || '';
      a[t.toLowerCase()] = [...blk.matchAll(/<Music File="([^"]*)"/g)].map(x => x[1]).filter(f => /\.ogg$/i.test(f) && haveAudio.has(f));
    }
    areas.push(a);
  }
  const miss = new Set(); for (const m of xml.matchAll(/<Music File="([^"]*)"/g)) if (!haveAudio.has(m[1])) miss.add(m[1]);
  console.log('music areas', areas.length, 'tiles', areas.reduce((n, a) => n + a.tiles.length, 0), 'unresolved tracks', [...miss]);
  fs.writeFileSync(path.join(D, 'music.json'), JSON.stringify(areas));
}
// ---- tile names
{
  const xml = rd('gamelogicinfo/tileinfomap.xml'), t = {};
  for (const m of xml.matchAll(/<TileInfo DisplayName="([^"]*)" Filename="([^"]*)"/g)) t[m[2]] = m[1];
  console.log('tile names', Object.keys(t).length);
  fs.writeFileSync(path.join(D, 'tiles.json'), JSON.stringify(t));
}
// ---- environments
{
  const base = 'objectspace/environment/';
  const cfgs = {};
  for (const f of ['human_clearsky', 'human_cloudy', 'human_test', 'cc_ork', 'cc_alfar', 'cc_dwarf', 'cc_mahirim', 'cc_mirdain', 'cc_human']) {
    const x = rd('object/environment/' + f + '.xml');
    const e = [...x.matchAll(/<EnvironmentOS File="([^"]+)" StartTime="(\d+)" InterpolationTime="(\d+)"/g)].map(m => ({ file: m[1], start: +m[2], interp: +m[3] }));
    if (e.length) cfgs[f] = e;
  }
  // regional full-day cycle: the volcano set mirrors the clear-sky letters/times
  cfgs.volcano_clear = cfgs.human_clearsky.map(e => ({ ...e, file: e.file.replace('env_human_clear01_', 'env_volcano_clear01_') }));
  const num = (s, tag) => { const m = s.match(new RegExp('<' + tag + '>\\s*<SFFloat Value="([-\\d.e]+)"')); return m ? +m[1] : null; };
  const vec = (s, tag) => { const m = s.match(new RegExp('<' + tag + '>\\s*<SFVector3 X="([-\\d.e]+)" Y="([-\\d.e]+)" Z="([-\\d.e]+)"')); return m ? [+m[1], +m[2], +m[3]] : null; };
  const col = (s, tag) => { const m = s.match(new RegExp('<' + tag + '>\\s*<SFColor A="([-\\d.e]+)" B="([-\\d.e]+)" G="([-\\d.e]+)" R="([-\\d.e]+)"')); return m ? [+m[4], +m[3], +m[2]] : null; };
  const frames = {};
  for (const f of new Set([...Object.values(cfgs).flat().map(e => e.file), ...fs.readdirSync(path.join(D, base)).filter(f => /\.xml$/.test(f))])) {
    let s; try { s = rd(base + f); } catch { continue; }
    const planets = [...s.matchAll(/<SFPlanet>([\s\S]*?)<\/SFPlanet>/g)].map(m => m[1]).map(p => ({
      xml: ((p.match(/<XMLFilename>\s*<SFString Value="([^"]*)"/) || [])[1] || '').replace(/\\/g, '/'),
      color: col(p, 'LightColor'), intensity: num(p, 'Intensity'), dist: vec(p, 'Distance'), rot: vec(p, 'Rotation'), local: vec(p, 'LocalRotation'),
      logical: num(p, 'LogicalDistance'), visible: /<Visible>\s*<SFBool Value="TRUE"/.test(p),
    }));
    const atm = (s.match(/<SFAtmosphere>([\s\S]*?)<\/SFAtmosphere>/) || [])[1] || '';
    const fog = (s.match(/<SFWorldGlobalLinearFogObject>([\s\S]*?)<\/SFWorldGlobalLinearFogObject>/) || [])[1] || '';
    const wind = (s.match(/<SFWind>([\s\S]*?)<\/SFWind>/) || [])[1] || '';
    frames[f] = { planets, clouds: [...s.matchAll(/<HemisphereTexture>\s*<SFString Value="([^"]*)"/g)].map(m => m[1]),
      cloudColor: col(atm, 'CloudColor'), cloudCover: num(atm, 'CloudCover'), cloudThickness: num(atm, 'CloudThickness'), ambient: col(atm, 'AtmosphericLight'),
      fogColor: col(fog, 'FogColor'), fogStart: num(fog, 'FogStart'), fogEnd: num(fog, 'FogEnd'), fogDensity: num(fog, 'Density'),
      wind: vec(wind, 'WindDirection'), windStrength: num(wind, 'WindStrength') };
  }
  console.log('env configs', Object.keys(cfgs), 'keyframes', Object.keys(frames).length);
  fs.writeFileSync(path.join(D, 'env.json'), JSON.stringify({ configs: cfgs, frames }));
  console.log(JSON.stringify(frames['env_human_clear01_a.xml'].planets.map(p => [p.xml, p.color, p.intensity])));
}

// ---- terrain texture manifest (which materials ship a normal map with height/AO payload)
{
  const dds = fs.readdirSync(path.resolve(__dirname, '../../export/texture/dds'));
  const set = new Set(dds), out = {};
  for (const f of dds) { const m = f.match(/^(\d+_tex_terrain_.*?)\.dds$/); if (!m || /_(normal|mads)$/.test(m[1]) || / copy/.test(m[1])) continue; out[f] = set.has(m[1] + '_normal.dds') ? m[1] + '_normal.dds' : null; }
  console.log('terrain textures', Object.keys(out).length, 'with normal', Object.values(out).filter(Boolean).length);
  fs.writeFileSync(path.join(D, 'terrain_textures.json'), JSON.stringify(out));
}
