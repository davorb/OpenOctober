//! The data files the server reads when it runs for real parse, and say what the game relies on them for.
use server::combat::Weapons;
use server::economy::{load_vendors, Group, Items};

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../game/data");

#[test]
fn the_weapon_table_loads_and_has_a_worst_obtainable_weapon() {
    let w = Weapons::load(&format!("{ROOT}/weapons.json")).expect("weapons.json parses (some flags are written as 1, not true)");
    assert!(w.defs.len() > 400);
    let worst = w.worst_melee().expect("a worst weapon");
    let d = w.get(worst).unwrap();
    assert!(d.obtainable() && d.rank.unwrap() > 0.0, "{worst}: {d:?}");
    let rank = d.rank.unwrap();
    assert!(w.defs.values().filter(|d| d.obtainable() && server::combat::is_melee(d.group.as_deref().unwrap_or("")) && d.group.as_deref() != Some("shield")).all(|d| d.rank.unwrap() >= rank), "no obtainable melee weapon is worse");
    // the troll lords' unranked weapons and natural attacks are not for characters
    assert!(!w.get("r_Aernaran").unwrap().obtainable());
    println!("worst obtainable weapon: {worst}");
}

#[test]
fn items_vendors_and_weights_load() {
    let items = Items::load(&format!("{ROOT}/items.json")).unwrap();
    assert!(items.list.len() > 900);
    let with_weight = items.list.iter().filter(|i| i.weight.is_some()).count();
    assert!(with_weight > 900, "the game's own weights: {with_weight}");
    assert_eq!(items.get("r_Talon Axe").unwrap().weight, Some(5.0));
    assert!(items.group(Group::Weapon).any(|i| i.cost == 0.0), "weapons with no price are carried but not sold");
    let spots = load_vendors(&format!("{ROOT}/vendors.json")).unwrap();
    assert!(spots.len() > 200);
}
