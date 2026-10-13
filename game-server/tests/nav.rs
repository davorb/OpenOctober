//! Navigation: routes around walls, unreachable goals, terrain heights, the baked file format, and NPCs that walk around obstacles.
use game_server::nav::*;
use game_server::npcs::*;

fn wall(x: usize, z0: usize, z1: usize) -> Vec<(usize, usize, u8)> {
    (z0..z1).map(|z| (x, z, F_WALL)).collect()
}

#[test]
fn a_route_goes_around_a_wall_and_every_leg_is_clear() {
    // a wall at x = 512 m from z = 400 m to 600 m
    let nav = Nav::from_tiles(vec![NavTile::flat(0, 0, 230.0, &wall(256, 200, 300))]);
    let (a, b) = ((400.0, 500.0), (620.0, 500.0));
    assert!(!nav.line_clear(a, b), "the straight line is blocked");
    let path = nav.find_path(a, b, 20000).expect("a way around");
    assert!(path.len() >= 2 && path.len() < 12, "string pulling keeps a handful of waypoints, got {}", path.len());
    let mut at = a;
    for p in &path {
        assert!(nav.line_clear(at, *p), "leg {at:?} -> {p:?} crosses the wall");
        at = *p;
    }
    assert!((at.0 - b.0).abs() < 2.0 && (at.1 - b.1).abs() < 2.0);
    let len: f32 = std::iter::once(a).chain(path.iter().copied()).collect::<Vec<_>>().windows(2).map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1)).sum();
    assert!(len < 330.0, "a detour of about 120 m extra, not a wander: {len}");
}

#[test]
fn unreachable_goals_and_missing_tiles() {
    // goal inside a closed box
    let mut blocked = Vec::new();
    for i in 300..310 {
        blocked.extend([(i, 300, F_WALL), (i, 309, F_WALL), (300, i, F_WALL), (309, i, F_WALL)]);
    }
    let nav = Nav::from_tiles(vec![NavTile::flat(0, 0, 230.0, &blocked)]);
    let inside = (604.0 + 4.0, 604.0 + 4.0);
    assert!(nav.find_path((100.0, 100.0), inside, 30000).is_none() || nav.snap(inside, 8).map_or(true, |s| (s.0 - inside.0).abs() > 1.0), "no route into a sealed box");
    assert!(!nav.walkable(5000.0, 5000.0), "no tile = blocked");
    assert!(nav.height(5000.0, 5000.0).is_none());
    // snap moves a point out of a wall to the nearest free cell
    let s = nav.snap((601.0, 601.0), 6).unwrap();
    assert!(nav.walkable(s.0, s.1));
}

#[test]
fn heights_come_from_the_grid() {
    let nav = Nav::from_tiles(vec![NavTile::flat(1024, 2048, 241.3, &[])]);
    let h = nav.height(1500.0, 2600.0).unwrap();
    assert!((h - 241.3).abs() < 0.06, "5 cm height steps, got {h}");
    assert!(nav.walkable(1500.0, 2600.0));
    assert!(nav.height(100.0, 100.0).is_none());
}

#[test]
fn baked_tile_parses_and_rejects_garbage() {
    // the real thing, when it has been baked (data/nav is generated and not in git)
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/data/nav");
    if let Ok(nav) = Nav::open(dir) {
        // Sanguine's bindstone: wire x 4776, z 2635 is on land inside tile u19
        let h = nav.height(4776.0, 2635.0).expect("u19 is baked");
        assert!(h > 200.0 && h < 300.0, "Sanguine stands at about 240 m, got {h}");
        let (cx, cz) = (4776.0, 2635.0);
        let start = nav.snap((cx, cz), 20).expect("near Sanguine there is somewhere to stand");
        // some of the spots around are sealed pockets (inside buildings): a good share of them must still be reachable, through legs that are clear
        let mut reached = 0;
        for k in 0..12 {
            let a = k as f32 * 0.52;
            let Some(goal) = nav.snap((cx + 140.0 * a.cos(), cz + 140.0 * a.sin()), 20) else { continue };
            if let Some(path) = nav.find_path(start, goal, 30000) {
                let mut at = start;
                for p in &path {
                    assert!(nav.line_clear(at, *p), "leg {at:?} -> {p:?} is blocked");
                    at = *p;
                }
                reached += 1;
            }
        }
        assert!(reached >= 6, "only {reached} of 12 spots around Sanguine are reachable");
    }
    assert!(Nav::open("/nonexistent/nav").is_err());
}

#[test]
fn active_npcs_walk_around_obstacles_and_stand_on_the_ground() {
    // two towns either side of a wall (x = 512 m, z 400..600 m); the player stands at the first
    let nav = Nav::from_tiles(vec![NavTile::flat(0, 0, 233.0, &wall(256, 150, 330))]);
    let places = vec![Place { name: "A".into(), x: 380.0, z: 500.0, town: true }, Place { name: "B".into(), x: 650.0, z: 500.0, town: true }];
    let mut w = Npcs::build(places, &[(0, (0.0, 0.0))], 3, 4);
    w.nav = Some(nav);
    let nav = w.nav.take().unwrap();
    let mut crossed = false;
    w.nav = Some(nav);
    for tick in 0..(15 * 1500) {
        w.step(1.0 / 15.0, &[(500.0, 500.0)], 900.0);
        for n in &w.npcs {
            if n.lod != Lod::Active {
                continue;
            }
            let nav = w.nav.as_ref().unwrap();
            assert!(nav.walkable(n.pos.x, n.pos.z), "npc {} stands in a blocked cell at {:?} (tick {tick})", n.info.name, (n.pos.x, n.pos.z));
            assert!((n.pos.y - 233.0).abs() < 0.1 && n.pos.flags & game_server::proto::FLAG_SNAP == 0, "an active npc has real height");
            if n.pos.x > 560.0 && n.home == 0 || n.pos.x < 460.0 && n.home == 1 {
                crossed = true;
            }
        }
    }
    assert!(crossed, "somebody should have walked from one town to the other, around the wall");
}

#[test]
fn npcs_in_the_real_world_never_stand_in_walls_or_water() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/..");
    let nav_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/data/nav");
    let Ok(nav) = Nav::open(nav_dir) else { return }; // not baked: nothing to check
    let mut w = Npcs::load(&format!("{root}/game-viewer/data/pois.json"), &format!("{root}/game-viewer/data/spawns.json"), 500, 1).unwrap();
    w.nav = Some(nav);
    // a player stands in Sanguine (wire x 4776, z 2635); five simulated minutes
    let (mut seen_active, mut moved) = (std::collections::HashSet::new(), 0u32);
    for tick in 0..(15 * 300) {
        w.step(1.0 / 15.0, &[(4776.0, 2635.0)], 900.0);
        let nav = w.nav.as_ref().unwrap();
        for n in w.npcs.iter().filter(|n| n.lod == Lod::Active) {
            assert!(nav.walkable(n.pos.x, n.pos.z), "{} stands in a blocked cell at ({:.0},{:.0}) after {} s", n.info.name, n.pos.x, n.pos.z, tick / 15);
            let h = nav.height(n.pos.x, n.pos.z).unwrap();
            assert!((n.pos.y - h).abs() < 0.01 && n.pos.y > 200.0);
            seen_active.insert(n.id);
            if n.pos.speed > 0 {
                moved += 1;
            }
        }
    }
    assert!(!seen_active.is_empty(), "somebody lives within 900 m of Sanguine");
    assert!(moved > 100, "and they walk about ({moved} moving samples)");
}

#[test]
fn snap_open_leaves_a_sealed_room_and_basins_are_still_big_enough() {
    // a closed room of 9 x 9 cells: standing inside it is free ground, but there is no way out
    let mut blocked = Vec::new();
    for i in 300..311 {
        blocked.extend([(i, 300, F_WALL), (i, 310, F_WALL), (300, i, F_WALL), (310, i, F_WALL)]);
    }
    let nav = Nav::from_tiles(vec![NavTile::flat(0, 0, 230.0, &blocked)]);
    let inside = (2.0 * 305.0 + 1.0, 2.0 * 305.0 + 1.0);
    assert_eq!(nav.snap(inside, 8), Some(inside), "plain snap is happy inside the room");
    let out = nav.snap_open(inside, 40, 1500).expect("open ground nearby");
    assert!(out.0 < 600.0 || out.0 >= 622.0 || out.1 < 600.0 || out.1 >= 622.0, "moved out of the room (walls span 600..622 m): {out:?}");
    assert!(nav.find_path(out, (100.0, 100.0), 30000).is_some());
    // open ground stays where it is
    assert_eq!(nav.snap_open((100.0, 100.0), 40, 1500), Some((100.0, 100.0)));
    // nothing but a tiny walled pocket in reach: no answer
    assert!(nav.snap_open(inside, 3, 1500).is_none());
}

#[test]
fn closed_regions_are_rejected_without_a_search() {
    // a closed room, and open ground around it: no route in or out, answered at once
    let mut blocked = Vec::new();
    for i in 300..311 {
        blocked.extend([(i, 300, F_WALL), (i, 310, F_WALL), (300, i, F_WALL), (310, i, F_WALL)]);
    }
    let nav = Nav::from_tiles(vec![NavTile::flat(0, 0, 230.0, &blocked)]);
    let inside = (2.0 * 305.0 + 1.0, 2.0 * 305.0 + 1.0);
    let t = std::time::Instant::now();
    assert!(nav.find_path(inside, (100.0, 100.0), 1_000_000).is_none(), "out of a closed room");
    assert!(nav.find_path((100.0, 100.0), inside, 1_000_000).is_none(), "into a closed room");
    assert!(t.elapsed() < std::time::Duration::from_millis(50), "no search was needed: {:?}", t.elapsed());
    // inside the room itself everything is reachable
    assert!(nav.find_path((603.0, 603.0), (617.0, 617.0), 10_000).is_some());
    // open ground to open ground still works
    assert!(nav.find_path((100.0, 100.0), (900.0, 900.0), 100_000).is_some());
}
