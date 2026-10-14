// Mob type -> model: data/mobmodels.json  { "<mob id>": { name, models: ["<converted/animated glb>", ...] } }
// Chain in the game data: mob name ("Goblin fighter") = resource "r_Goblin Fighter" in gamelogicinfo/resourcetree.xml -> its SFMeshFileName
// ("goblin_fighter.xml") = animation template in export/xml/animations -> Meshes Set0.. (*.sfam) = converted/animated/<name>.glb.
// Mob names that don't match a resource (the community site names some mobs differently) go through OVERRIDES / fuzzy matching below.
const fs = require('fs'), path = require('path');
const D = p => path.join(__dirname, '..', p), R = p => path.join(__dirname, '../..', p);
const norm = s => s.toLowerCase().replace(/^r_/, '').replace(/\([^)]*\)/g, '').replace(/[^a-z0-9]/g, '');
const glbs = new Set(fs.readdirSync(R('converted/animated')).filter(f => f.endsWith('.glb')).map(f => f.slice(0, -4)));

const glbByNorm = new Map([...glbs].map(g => [norm(g), g]));   // 'handofkhamset' -> hand_of_khamset
const tree = fs.readFileSync(D('data/gamelogicinfo/resourcetree.xml'), 'latin1');
const res = new Map();   // normalised resource name -> mesh xml base
for (const m of tree.matchAll(/<SFListItem>\s*<SFName>\s*<SFString Value="(r_[^"]*)"[\s\S]*?<SFMeshFileName>\s*<SFString Value="([^"]*)"/g))
  if (m[2]) res.set(norm(m[1]), m[2].replace(/\.xml$/i, ''));

// mob name (normalised) -> resource / glb to use when there is no resource of that name
const OVERRIDES = {
  zombie: 'zombie_01', zombieshambler: 'zombie_02', zombiewarrior: 'zombie_warrior_01', direzombie: 'dire_zombie',
  skeletonwarrior: 'skeleton_warrior', skeletonarcher: 'skeleton_archer', brittleskeleton: 'skeleton_brittle', giantskeleton: 'skeleton_giant', boneknight: 'skeleton_roman_armor',
  ghost: 'ghost', ghostspider: 'spider_ghost', giantspider: 'spider', hairyspider: 'spider_hairy', direspider: 'spider_dire', doomspider: 'spider_doom',
  brownbear: 'bear_brown', arcticbear: 'bear_polar', wolfrevenant: 'revenant_wolf', hawkrevenant: 'revenant_hawk', jackalrevenant: 'revenant_jackal', leopardrevenant: 'revenant_leopard', snakerevenant: 'revenant_snake',
  minorsarnakh: 'sarnakh_minor', vilesarnakh: 'sarnakh_vile', greatermatayin: 'greater_matayin', greatermummy: 'mummy_greater', lesserfireelemental: 'fire_elemental', waterelemental: 'ice_elemental', lesserwaterelemental: 'ice_elemental',
  greatercelestial: 'elemental_celestial', celestial: 'elemental_celestial', infernal: 'elemental_infernal', greatermalautservant: 'servant_of_malaut_greater', heielhim: 'heielhim', gravelord: 'ghoul_sorcerer', ghouldevourer: 'ghoul_devourer',
  moonbeastdiciple: 'moonbeast_disciple', graywarrior: 'gray_ork', darkdragon: 'dragon_dark', firedragon: 'fire_dragon', greatwhite: 'shark', kraken: 'sea_serpent', seaserpent: 'sea_serpent',
  seidboundicegolem: 'golem_ice', winterboundicegolem: 'golem_ice', oaklord: 'oaklord', deathlessmage: 'deathless_mage', deathlessservant: 'deathless_servant', hoodedone: 'hooded_one',
  severiiarcher: 'severii_archer', severiicenturion: 'severii_centurion', severiilegionnaire: 'severii_legionnaire', undeaddwarfguardian: 'undead_dwarf_guardian', undeaddwarfforeman: 'undead_dwarf_foreman', undeaddwarfminer: 'undead_dwarf_miner',
  hivekinsoldier: 'hivekin_guard', hivekinforager: 'hivekin', hivekinqueen: 'hivekin_queen', hivekinbroodguard: 'hivekin_guard',
  mercianoutlaw: 'human_bandit_01', mercianhighwayman: 'human_bandit_highway_01', mercianbrawler: 'human_bandit_02',
  boneexarch: 'skeleton_commander_egyptian', sunexarch: 'skeleton_commander_egyptian', chosenofkhamset: 'hand_of_khamset', cryptguard: 'skeleton_giant_egyptian',
  tombiklit: 'tomb_iklit', grayiklit: 'iklit', nightiklit: 'iklit', sandiklit: 'iklit', wraith: 'shadow_spirit', demonqueen: 'demon', demonlord: 'demon', darkshade: 'shadow_spirit',
  irontheancient: 'granite_golem', ancient: 'forest_golem', sonoforchran: 'oaklord', oaksage: 'forest_prophet_male', shroudknight: 'shadow_knight', guardian: 'stoneguard',
  bonecrewcaptain: 'undead_pirate_captain_01', bonecrewmariner: 'undead_pirate_01', bonecrewmutineer: 'undead_pirate_02', arthainhunchback: 'arthain_hump', twoheadedarthain: 'arthain_2headed',
  ghoulfleshcrafter: 'ghoul_sorcerer', sadayelchieftain: 'sadayel_chief', chieftankhalos: 'sadayel_chief', fenristhealpha: 'wolf',
  greaterbelow: 'below_uber', garadoroc: 'gardoroc', gorradar: 'gorra_dar', gorraslave: 'gorra_slave', grayorkassasin: 'gray_ork', cavetroll: 'troll_cave', oathbound: 'oathbound', oathliege: 'oathliege',
};

function meshesOf(base) {   // animation template -> glb names
  const f = R(`export/xml/animations/${base}.xml`); if (!fs.existsSync(f)) return null;
  const x = fs.readFileSync(f, 'latin1'), n = x.match(/<Normal>[\s\S]*?<\/Normal>/); if (!n) return null;
  const meshes = (n[0].match(/<Meshes>[\s\S]*?<\/Meshes>/) || [''])[0];
  const out = [...meshes.matchAll(/<Set\d+>\s*<SFString Value="([^"]+)"/g)].map(m => m[1].replace(/\.sfam$/i, '').toLowerCase()).filter(g => glbs.has(g));
  return out.length ? out : null;
}
function modelsFor(name) {
  const k = norm(name), tries = [res.get(k), OVERRIDES[k], k];
  const byNorm = glbByNorm.get(k); if (byNorm) tries.push(byNorm);
  for (const t of tries) if (t) { const m = meshesOf(t.toLowerCase()) || (glbs.has(t.toLowerCase()) ? [t.toLowerCase()] : null); if (m) return m; }
  return null;
}

const seen = new Map();
for (const s of JSON.parse(fs.readFileSync(D('data/mobspawns.json')))) for (const m of s.mobs) seen.set(m.id, m.name);
const out = {}, miss = [];
for (const [id, name] of seen) { const models = modelsFor(name); if (models) out[id] = { name, models }; else miss.push(name); }
console.log(`${Object.keys(out).length}/${seen.size} mob types have a model; missing: ${miss.join(', ') || 'none'}`);
fs.writeFileSync(D('data/mobmodels.json'), JSON.stringify(out));
