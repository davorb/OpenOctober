//! Prices, trades, restocking and loot (the economy module on its own, no NPCs).
use server::economy::*;

fn items() -> Items {
    Items::parse(
        r#"[{"id":"r_Apple","cost":2,"group":"food","stack":true},{"id":"r_Bread","cost":4,"group":"food","stack":true},
            {"id":"r_Elixir","cost":40,"group":"consumable","stack":true},{"id":"r_Hopta Root","cost":25,"group":"herb","stack":true},
            {"id":"r_Iron","cost":10,"group":"material","stack":true},{"id":"r_Arrow","cost":2,"group":"ammo","stack":true},
            {"id":"r_Talon Axe","cost":300,"group":"weapon"},{"id":"r_Plate Helm","cost":500,"group":"armor"}]"#,
    )
    .unwrap()
}

fn vendor(kind: VendorKind) -> Vendor {
    let mut rng = 12345;
    Vendor::new("Test".into(), kind, (0.0, 0.0), 0, &items(), &mut rng)
}

#[test]
fn catalogue_parses_and_groups() {
    let i = items();
    assert_eq!(i.list.len(), 8);
    assert_eq!(i.get("r_Apple").unwrap().cost, 2.0);
    assert_eq!(i.group(Group::Food).count(), 2);
    assert!(i.get("r_Nothing").is_none());
}

#[test]
fn vendors_stock_only_their_trade_and_start_at_target() {
    let i = items();
    let smith = vendor(VendorKind::Blacksmith);
    assert!(smith.stock.items.keys().all(|k| smith.kind.deals_in(i.get(k).unwrap().group)), "{:?}", smith.stock.items);
    assert!(smith.stock.items.keys().any(|k| i.get(k).unwrap().group == Group::Weapon));
    assert_eq!(smith.stock.items, smith.target);
    let merchant = vendor(VendorKind::Merchant);
    assert!(merchant.stock.items.keys().all(|k| i.get(k).unwrap().group != Group::Weapon));
    assert!(merchant.stock.gold >= 1500.0);
}

#[test]
fn scarcity_makes_prices_follow_supply() {
    assert!((scarcity(10, 10) - 1.0).abs() < 1e-6);
    assert!(scarcity(0, 10) > 1.5 && scarcity(0, 10) <= 2.5);
    assert!(scarcity(50, 10) < 0.6 && scarcity(500, 10) >= 0.5);
    let i = items();
    let mut v = vendor(VendorKind::Merchant);
    let apple = "r_Apple";
    let full = v.target.get(apple).copied().unwrap_or(0);
    v.target.insert(apple.into(), 20);
    v.stock.items.insert(apple.into(), 20);
    let normal = v.ask(&i, apple).unwrap();
    v.stock.items.insert(apple.into(), 2);
    let scarce = v.ask(&i, apple).unwrap();
    assert!(scarce > normal * 1.5, "{scarce} vs {normal}");
    assert!(normal > 2.0 * 1.1 && normal < 2.0 * 1.2, "normal price is the base cost plus the 15% margin: {normal} ({full})");
    v.stock.items.remove(apple);
    assert!(v.ask(&i, apple).is_none(), "nothing to sell");
}

#[test]
fn a_vendor_never_loses_by_trading_in_circles() {
    let i = items();
    let mut v = vendor(VendorKind::Merchant);
    v.target.insert("r_Iron".into(), 10);
    v.stock.items.insert("r_Iron".into(), 10);
    let mut me = Stock { gold: 1000.0, items: Default::default() };
    let bought = v.sell_to(&i, &mut me, "r_Iron", 5).unwrap();
    assert_eq!(bought.units, 5);
    assert_eq!(me.count("r_Iron"), 5);
    let before = me.gold;
    let sold = v.buy_from(&i, &mut me, "r_Iron", 5).unwrap();
    assert_eq!(sold.units, 5);
    assert!(sold.gold < bought.gold * 0.6, "buying back is far cheaper than buying: {} vs {}", sold.gold, bought.gold);
    assert!(me.gold > before && me.gold < 1000.0, "the round trip costs the trader gold");
}

#[test]
fn trades_respect_gold_stock_and_trade() {
    let i = items();
    let mut smith = vendor(VendorKind::Blacksmith);
    let mut me = Stock { gold: 5.0, items: Default::default() };
    // too poor
    let axe = smith.stock.items.keys().find(|k| i.get(k).unwrap().group == Group::Weapon).cloned().unwrap();
    assert_eq!(smith.sell_to(&i, &mut me, &axe, 1), Err(Refusal::NothingDone));
    // not his trade
    me.add("r_Apple", 3);
    assert_eq!(smith.buy_from(&i, &mut me, "r_Apple", 3), Err(Refusal::NotInTrade));
    assert_eq!(smith.sell_to(&i, &mut me, "r_Apple", 1), Err(Refusal::NotInTrade));
    assert_eq!(smith.sell_to(&i, &mut me, "r_Ghost", 1), Err(Refusal::UnknownItem));
    // a vendor out of gold cannot buy
    let mut broke = vendor(VendorKind::Merchant);
    broke.stock.gold = 0.5;
    assert_eq!(broke.buy_from(&i, &mut me, "r_Apple", 3), Err(Refusal::NothingDone));
    // selling more than you have only sells what you have
    let mut merchant = vendor(VendorKind::Merchant);
    let d = merchant.buy_from(&i, &mut me, "r_Apple", 10).unwrap();
    assert_eq!(d.units, 3);
    assert_eq!(me.count("r_Apple"), 0);
    // gold is conserved in every trade
    let mut me2 = Stock { gold: 100.0, items: Default::default() };
    let t0 = merchant.stock.gold + me2.gold;
    let _ = merchant.sell_to(&i, &mut me2, "r_Elixir", 2);
    assert!((merchant.stock.gold + me2.gold - t0).abs() < 1e-3);
}

#[test]
fn big_sales_push_the_price_down_as_they_go() {
    let i = items();
    let mut v = vendor(VendorKind::Merchant);
    v.target.insert("r_Apple".into(), 10);
    v.stock.items.insert("r_Apple".into(), 10);
    let first = v.bid(&i, "r_Apple").unwrap();
    let mut me = Stock::default();
    me.add("r_Apple", 30);
    let d = v.buy_from(&i, &mut me, "r_Apple", 30).unwrap();
    assert!(d.units == 30);
    assert!(d.gold < first * 30.0 * 0.8, "the last apples fetched much less than the first: avg {} vs first {first}", d.gold / 30.0);
    assert!(v.bid(&i, "r_Apple").unwrap() < first * 0.6);
}

#[test]
fn restocking_refills_shelves_and_clears_gluts() {
    let mut v = vendor(VendorKind::Merchant);
    let some = v.target.keys().next().cloned().unwrap();
    let t = v.target[&some];
    v.stock.items.remove(&some);
    v.stock.gold = 10.0;
    for _ in 0..30 {
        v.restock();
    }
    assert!(v.stock.count(&some) >= t - 1, "refilled: {} of {t}", v.stock.count(&some));
    assert!(v.stock.gold > v.base_gold * 0.9);
    v.stock.add(&some, 100);
    v.stock.add("r_Unlisted", 9);
    for _ in 0..60 {
        v.restock();
    }
    assert!(v.stock.count(&some) <= t + 1, "glut gone");
}

#[test]
fn loot_grows_with_rank_and_is_reproducible() {
    let i = items();
    let (mut a, mut b) = (7u32, 7u32);
    assert_eq!(loot(&i, 2, &mut a), loot(&i, 2, &mut b));
    let avg = |rank: u8| {
        let mut rng = 99u32;
        (0..400).map(|_| loot(&i, rank, &mut rng).0).sum::<f32>() / 400.0
    };
    assert!(avg(4) > avg(0) * 8.0, "{} vs {}", avg(4), avg(0));
    let mut rng = 5u32;
    let mut any_item = false;
    for _ in 0..200 {
        let (g, drops) = loot(&i, 0, &mut rng);
        assert!(g >= 2.0 && drops.len() <= 2);
        for (id, q) in drops {
            any_item = true;
            assert!(i.get(&id).unwrap().cost <= 15.0, "a rat does not drop a plate helm: {id}");
            assert!(q >= 1);
        }
    }
    assert!(any_item);
}
