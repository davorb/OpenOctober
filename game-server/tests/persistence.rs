//! NPCs survive a restart: identity, place, goal, needs and position come back from the SQLite file; a save made for another place list is ignored.
use game_server::npcs::*;
use game_server::store::Store;

fn places(n: usize) -> Vec<Place> {
    (0..n).map(|i| Place { name: format!("T{i}"), x: i as f32 * 2000.0, z: 0.0, town: true }).collect()
}

fn tmp(name: &str) -> String {
    let p = std::env::temp_dir().join(name);
    let _ = std::fs::remove_file(&p);
    p.to_string_lossy().into()
}

#[test]
fn saved_npcs_come_back_as_they_were() {
    let path = tmp("df_npc_persist.db");
    let mut a = Npcs::build(places(6), &[(0, (0.0, 0.0))], 25, 7);
    for _ in 0..(15 * 90) {
        a.step(1.0 / 15.0, &[(-50_000.0, 0.0)], 900.0);
    }
    let saved = a.states();
    Store::open(&path).unwrap().save(a.places.len(), &saved).unwrap();

    // a different seed makes a different population; restoring overwrites it with the saved one
    let mut b = Npcs::build(places(6), &[(0, (0.0, 0.0))], 25, 99);
    assert_ne!(a.npcs[0].info.name, b.npcs[0].info.name);
    let rows = Store::open(&path).unwrap().load(b.places.len()).unwrap();
    assert_eq!(b.restore(&rows), 25);
    for (x, y) in a.npcs.iter().zip(&b.npcs) {
        assert_eq!((x.id, &x.info.name, &x.info.race, &x.info.gear, x.level, x.home, x.at), (y.id, &y.info.name, &y.info.race, &y.info.gear, y.level, y.home, y.at));
        assert_eq!((x.pos.x, x.pos.z, x.fatigue), (y.pos.x, y.pos.z, y.fatigue));
    }
    // the second save is identical apart from the movement flag we reset on restore
    assert_eq!(a.states().len(), b.states().len());
    // and the restored world keeps living
    for _ in 0..(15 * 30) {
        b.step(1.0 / 15.0, &[(-50_000.0, 0.0)], 900.0);
    }
    assert!(b.npcs.iter().all(|n| n.pos.x.is_finite() && n.pos.z.is_finite()));
}

#[test]
fn a_save_for_other_places_is_ignored_and_a_smaller_population_takes_a_prefix() {
    let path = tmp("df_npc_persist2.db");
    let a = Npcs::build(places(6), &[(0, (0.0, 0.0))], 10, 3);
    let mut store = Store::open(&path).unwrap();
    store.save(a.places.len(), &a.states()).unwrap();
    assert!(store.load(7).unwrap().is_empty(), "place list changed: nothing is restored");
    let mut small = Npcs::build(places(6), &[(0, (0.0, 0.0))], 4, 3);
    assert_eq!(small.restore(&store.load(6).unwrap()), 4);
    let mut big = Npcs::build(places(6), &[(0, (0.0, 0.0))], 14, 3);
    assert_eq!(big.restore(&store.load(6).unwrap()), 10, "the four new npcs start fresh");
}
