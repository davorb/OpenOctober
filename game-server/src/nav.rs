//! Walkability of the world, from the files `game-viewer/tools/build_nav.js` bakes (one per land tile, see that script for the format).
//! Tiles are loaded on first use and kept; a tile that was not baked (open sea) is blocked everywhere.
//!
//! Cells are 2 m squares in wire space (the viewer's x / z); `flags == 0` means walkable. `height` is the terrain's topmost surface,
//! interpolated from the 4 m grid. `find_path` is A* over the cells (8 neighbours, no squeezing between two blocked corners) followed by
//! string pulling, so the result is a few waypoints with a clear line between each.
use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const F_STEEP: u8 = 1;
pub const F_WATER: u8 = 2;
pub const F_WALL: u8 = 4;
pub const BLOCKED: u8 = 0xFF;

const TILE: f32 = 1024.0;
const G: usize = 256;
const N: usize = 512;
const W: usize = G + 1;
pub const CELL: f32 = TILE / N as f32;
const HEADER: usize = 20;

pub struct NavTile {
    pub x0: i32,
    pub z0: i32,
    heights: Vec<u16>,
    flags: Vec<u8>,
}

impl NavTile {
    /// Parse a baked file.
    pub fn parse(b: &[u8]) -> Result<NavTile, String> {
        if b.len() != HEADER + W * W * 2 + N * N || &b[0..4] != b"DNAV" || u32::from_le_bytes(b[4..8].try_into().unwrap()) != 1 {
            return Err("not a v1 nav tile".into());
        }
        let i32_at = |o: usize| i32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        let heights = b[HEADER..HEADER + W * W * 2].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        Ok(NavTile { x0: i32_at(8), z0: i32_at(12), heights, flags: b[HEADER + W * W * 2..].to_vec() })
    }

    /// A tile of flat ground at `height` with the given blocked cells (local cell x, z, flag); for tests.
    pub fn flat(x0: i32, z0: i32, height: f32, blocked: &[(usize, usize, u8)]) -> NavTile {
        let mut flags = vec![0u8; N * N];
        for &(x, z, f) in blocked {
            flags[z * N + x] = f;
        }
        NavTile { x0, z0, heights: vec![((height + 500.0) * 20.0) as u16; W * W], flags }
    }
}

#[derive(Default)]
pub struct Nav {
    /// Tile corner (x / 1024, z / 1024) -> baked file.
    files: HashMap<(i32, i32), PathBuf>,
    tiles: RefCell<HashMap<(i32, i32), Option<Arc<NavTile>>>>,
    /// The tile looked up last: searches stay in one tile for long stretches.
    last: RefCell<Option<((i32, i32), Option<Arc<NavTile>>)>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Open {
    f: u32,
    cell: (i32, i32),
}
impl Ord for Open {
    fn cmp(&self, o: &Self) -> Ordering {
        o.f.cmp(&self.f).then_with(|| self.cell.cmp(&o.cell))
    }
}
impl PartialOrd for Open {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

fn tile_key(x: f32, z: f32) -> (i32, i32) {
    ((x / TILE).floor() as i32, (z / TILE).floor() as i32)
}

impl Nav {
    /// Index the baked tiles in `dir` (headers only: the tiles themselves load on demand).
    pub fn open(dir: &str) -> Result<Nav, String> {
        let mut files = HashMap::new();
        for e in std::fs::read_dir(dir).map_err(|e| format!("{dir}: {e}"))?.flatten() {
            let p = e.path();
            if p.extension().map_or(true, |x| x != "nav") {
                continue;
            }
            let mut head = [0u8; HEADER];
            let ok = std::fs::File::open(&p).and_then(|mut f| std::io::Read::read_exact(&mut f, &mut head)).is_ok();
            if ok && &head[0..4] == b"DNAV" {
                let (x0, z0) = (i32::from_le_bytes(head[8..12].try_into().unwrap()), i32::from_le_bytes(head[12..16].try_into().unwrap()));
                files.insert((x0.div_euclid(TILE as i32), z0.div_euclid(TILE as i32)), p);
            }
        }
        if files.is_empty() {
            return Err(format!("{dir}: no baked .nav tiles (run game-viewer/tools/build_nav.js)"));
        }
        Ok(Nav { files, tiles: RefCell::new(HashMap::new()), last: RefCell::new(None) })
    }

    /// A nav made of the given tiles (tests).
    pub fn from_tiles(tiles: Vec<NavTile>) -> Nav {
        let mut map = HashMap::new();
        for t in tiles {
            map.insert((t.x0.div_euclid(TILE as i32), t.z0.div_euclid(TILE as i32)), Some(Arc::new(t)));
        }
        Nav { files: HashMap::new(), tiles: RefCell::new(map), last: RefCell::new(None) }
    }

    pub fn tile_count(&self) -> usize {
        self.files.len().max(self.tiles.borrow().len())
    }

    fn tile(&self, key: (i32, i32)) -> Option<Arc<NavTile>> {
        if let Some((k, t)) = &*self.last.borrow() {
            if *k == key {
                return t.clone();
            }
        }
        let known = self.tiles.borrow().get(&key).cloned();
        let tile = match known {
            Some(t) => t,
            None => {
                let loaded = self.files.get(&key).and_then(|p: &PathBuf| load(p));
                self.tiles.borrow_mut().insert(key, loaded.clone());
                loaded
            }
        };
        *self.last.borrow_mut() = Some((key, tile.clone()));
        tile
    }

    /// Flags of cell (cx, cz) (cell = floor(coordinate / 2 m)); `BLOCKED` where there is no tile.
    pub fn cell(&self, cx: i32, cz: i32) -> u8 {
        let per = N as i32;
        match self.tile((cx.div_euclid(per), cz.div_euclid(per))) {
            Some(t) => t.flags[cz.rem_euclid(per) as usize * N + cx.rem_euclid(per) as usize],
            None => BLOCKED,
        }
    }

    pub fn walkable(&self, x: f32, z: f32) -> bool {
        self.cell((x / CELL).floor() as i32, (z / CELL).floor() as i32) == 0
    }

    /// Terrain height at (x, z), or `None` off the baked map.
    pub fn height(&self, x: f32, z: f32) -> Option<f32> {
        let t = self.tile(tile_key(x, z))?;
        let step = TILE / G as f32;
        let fx = ((x - t.x0 as f32) / step).clamp(0.0, G as f32 - 1e-4);
        let fz = ((z - t.z0 as f32) / step).clamp(0.0, G as f32 - 1e-4);
        let (x0, z0) = (fx as usize, fz as usize);
        let (tx, tz) = (fx - x0 as f32, fz - z0 as f32);
        let h = |x: usize, z: usize| t.heights[z * W + x] as f32 / 20.0 - 500.0;
        Some((h(x0, z0) * (1.0 - tx) + h(x0 + 1, z0) * tx) * (1.0 - tz) + (h(x0, z0 + 1) * (1.0 - tx) + h(x0 + 1, z0 + 1) * tx) * tz)
    }

    fn center(c: (i32, i32)) -> (f32, f32) {
        ((c.0 as f32 + 0.5) * CELL, (c.1 as f32 + 0.5) * CELL)
    }

    fn cell_of(p: (f32, f32)) -> (i32, i32) {
        ((p.0 / CELL).floor() as i32, (p.1 / CELL).floor() as i32)
    }

    /// The walkable cell centre nearest to `p` within `max_cells` rings, or `p` itself when it is walkable.
    pub fn snap(&self, p: (f32, f32), max_cells: i32) -> Option<(f32, f32)> {
        let c = Self::cell_of(p);
        if self.cell(c.0, c.1) == 0 {
            return Some(p);
        }
        for r in 1..=max_cells {
            let mut best: Option<(f32, (i32, i32))> = None;
            for dz in -r..=r {
                for dx in -r..=r {
                    if dx.abs().max(dz.abs()) != r || self.cell(c.0 + dx, c.1 + dz) != 0 {
                        continue;
                    }
                    let d = (dx * dx + dz * dz) as f32;
                    if best.map_or(true, |b| d < b.0) {
                        best = Some((d, (c.0 + dx, c.1 + dz)));
                    }
                }
            }
            if let Some((_, cell)) = best {
                return Some(Self::center(cell));
            }
        }
        None
    }

    /// Nothing blocked on the straight line a -> b: every cell the segment touches is free (exact grid walk), and passing exactly through a
    /// corner needs both neighbouring cells free.
    pub fn line_clear(&self, a: (f32, f32), b: (f32, f32)) -> bool {
        let (mut cx, mut cz) = Self::cell_of(a);
        let (ex, ez) = Self::cell_of(b);
        if self.cell(cx, cz) != 0 {
            return false;
        }
        let (px, pz) = (a.0 / CELL, a.1 / CELL);
        let (dx, dz) = ((b.0 - a.0) / CELL, (b.1 - a.1) / CELL);
        let (sx, sz) = (if dx > 0.0 { 1 } else { -1 }, if dz > 0.0 { 1 } else { -1 });
        let next = |c: i32, p: f32, d: f32, s: i32| if d == 0.0 { f32::INFINITY } else { ((if s > 0 { c + 1 } else { c }) as f32 - p) / d };
        let (mut tx, mut tz) = (next(cx, px, dx, sx), next(cz, pz, dz, sz));
        let (ddx, ddz) = (if dx == 0.0 { f32::INFINITY } else { 1.0 / dx.abs() }, if dz == 0.0 { f32::INFINITY } else { 1.0 / dz.abs() });
        // cross only the cell boundaries that lie strictly before the end of the segment (t < 1); the end cell is checked after the walk
        while tx.min(tz) < 1.0 {
            if tx < tz {
                cx += sx;
                tx += ddx;
            } else if tz < tx {
                cz += sz;
                tz += ddz;
            } else {
                if self.cell(cx + sx, cz) != 0 || self.cell(cx, cz + sz) != 0 {
                    return false;
                }
                cx += sx;
                cz += sz;
                tx += ddx;
                tz += ddz;
            }
            if self.cell(cx, cz) != 0 {
                return false;
            }
        }
        self.cell(ex, ez) == 0
    }

    /// A route from `from` to `to` (waypoints after `from`, ending at `to`'s walkable cell), or `None` when none is found within `max_expand` cells.
    pub fn find_path(&self, from: (f32, f32), to: (f32, f32), max_expand: usize) -> Option<Vec<(f32, f32)>> {
        let start = Self::cell_of(self.snap(from, 8)?);
        let goal_p = self.snap(to, 8)?;
        let goal = Self::cell_of(goal_p);
        if start == goal {
            return Some(vec![goal_p]);
        }
        let h = |c: (i32, i32)| {
            let (dx, dz) = ((c.0 - goal.0).unsigned_abs(), (c.1 - goal.1).unsigned_abs());
            10 * dx.max(dz) + 4 * dx.min(dz)
        };
        let mut best: HashMap<(i32, i32), (u32, (i32, i32))> = HashMap::new();
        let mut open = BinaryHeap::new();
        best.insert(start, (0, start));
        open.push(Open { f: h(start), cell: start });
        let mut expanded = 0;
        while let Some(Open { cell, .. }) = open.pop() {
            if cell == goal {
                let mut cells = vec![cell];
                let mut c = cell;
                while c != start {
                    c = best[&c].1;
                    cells.push(c);
                }
                cells.reverse();
                return Some(self.pull(&cells, from, goal_p));
            }
            expanded += 1;
            if expanded > max_expand {
                return None;
            }
            let g = best[&cell].0;
            for dz in -1i32..=1 {
                for dx in -1i32..=1 {
                    if dx == 0 && dz == 0 {
                        continue;
                    }
                    let n = (cell.0 + dx, cell.1 + dz);
                    if self.cell(n.0, n.1) != 0 {
                        continue;
                    }
                    let diag = dx != 0 && dz != 0;
                    if diag && (self.cell(cell.0 + dx, cell.1) != 0 || self.cell(cell.0, cell.1 + dz) != 0) {
                        continue;
                    }
                    let ng = g + if diag { 14 } else { 10 };
                    if best.get(&n).map_or(true, |b| ng < b.0) {
                        best.insert(n, (ng, cell));
                        open.push(Open { f: ng + h(n), cell: n });
                    }
                }
            }
        }
        None
    }

    /// String pulling: drop every waypoint that the previous kept one can see past.
    fn pull(&self, cells: &[(i32, i32)], from: (f32, f32), goal: (f32, f32)) -> Vec<(f32, f32)> {
        let mut pts: Vec<(f32, f32)> = cells.iter().map(|&c| Self::center(c)).collect();
        // the goal point itself is the last point to see (it is inside the last cell, not at its centre)
        if let Some(last) = pts.last_mut() {
            *last = goal;
        }
        let mut out = Vec::new();
        let mut at = from;
        let mut i = 0;
        while i < pts.len() {
            // furthest point still in sight from `at`
            let mut j = i;
            while j + 1 < pts.len() && self.line_clear(at, pts[j + 1]) {
                j += 1;
            }
            at = pts[j];
            out.push(at);
            i = j + 1;
        }
        out
    }
}

fn load(p: &Path) -> Option<Arc<NavTile>> {
    match std::fs::read(p).map_err(|e| e.to_string()).and_then(|b| NavTile::parse(&b)) {
        Ok(t) => Some(Arc::new(t)),
        Err(e) => {
            eprintln!("{}: {e}", p.display());
            None
        }
    }
}
