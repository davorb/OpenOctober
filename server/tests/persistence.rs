//! NPCs survive a restart: identity, place, goal, needs and position come back from the SQLite file; a save made for another place list is ignored.
use server::npcs::*;
use server::store::Store;

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
    Store::open(&path).unwrap().save(a.places.len(), 0, &saved).unwrap();

    // a different seed makes a different population; restoring overwrites it with the saved one
    let mut b = Npcs::build(places(6), &[(0, (0.0, 0.0))], 25, 99);
    assert_ne!(a.npcs[0].info.name, b.npcs[0].info.name);
    let rows = Store::open(&path).unwrap().load(b.places.len(), 0).unwrap();
    assert_eq!(b.restore(&rows), 25);
    for (x, y) in a.npcs.iter().zip(&b.npcs) {
        assert_eq!((x.id, &x.info.name, &x.info.race, &x.info.gear, &x.skills, x.home, x.at), (y.id, &y.info.name, &y.info.race, &y.info.gear, &y.skills, y.home, y.at));
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
    store.save(a.places.len(), 0, &a.states()).unwrap();
    assert!(store.load(7, 0).unwrap().is_empty(), "place list changed: nothing is restored");
    let mut small = Npcs::build(places(6), &[(0, (0.0, 0.0))], 4, 3);
    assert_eq!(small.restore(&store.load(6, 0).unwrap()), 4);
    let mut big = Npcs::build(places(6), &[(0, (0.0, 0.0))], 14, 3);
    assert_eq!(big.restore(&store.load(6, 0).unwrap()), 10, "the four new npcs start fresh");
}

#[test]
fn shelves_purses_and_cargo_survive_a_restart() {
    use server::economy::*;
    let items = || Items::parse(r#"[{"id":"r_Apple","cost":2,"group":"food","stack":true},{"id":"r_Iron","cost":10,"group":"material","stack":true}]"#).unwrap();
    let spots = [VendorSpot { name: "Ann".into(), kind: VendorKind::Merchant, x: 10.0, z: 5.0 }, VendorSpot { name: "Bo".into(), kind: VendorKind::Market, x: 2010.0, z: 0.0 }];
    let make = |seed| Npcs::build_with_economy(places(6), &[(0, (0.0, 0.0))], items(), &spots, 6, seed);
    let path = tmp("df_npc_persist_econ.db");
    let mut a = make(3);
    {
        let mut eco = a.economy_mut();
        eco.vendors[1].stock.gold = 123.5;
        eco.vendors[1].stock.items.insert("r_Apple".into(), 77);
    }
    let walker = a.npcs.iter().position(|n| n.role != Role::Vendor).unwrap();
    a.npcs[walker].stock = Stock { gold: 42.25, items: [("r_Iron".to_string(), 9)].into() };
    let saved = a.states();
    let mut store = Store::open(&path).unwrap();
    store.save(a.places.len(), a.vendor_count(), &saved).unwrap();

    let mut b = make(3);
    let rows = store.load(b.places.len(), b.vendor_count()).unwrap();
    assert_eq!(b.restore(&rows), 8, "two vendors and six npcs");
    assert_eq!(b.economy().vendors[1].stock.gold, 123.5);
    assert_eq!(b.economy().vendors[1].stock.count("r_Apple"), 77);
    assert_eq!(b.npcs[walker].stock.gold, 42.25);
    assert_eq!(b.npcs[walker].stock.count("r_Iron"), 9);
    assert_eq!(b.npcs[0].role, Role::Vendor);

    // vendors come first among the ids: a different number of vendors means a different layout, and the save is not used
    assert!(store.load(b.places.len(), b.vendor_count() + 1).unwrap().is_empty());
    // a row whose role does not fit is skipped (here: pretend the first npc after the vendors was a vendor when saved)
    let mut bad = saved.clone();
    bad[walker].role = 2;
    let mut c = make(3);
    assert_eq!(c.restore(&bad), 7);
}
