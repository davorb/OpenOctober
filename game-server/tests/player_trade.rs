//! Players and the shops: the worst weapon to start with, owning what you wield, opening a shop, buying and selling, what you can carry, kill drops.
use std::time::Duration;

use game_server::economy::*;
use game_server::npcs::NPC_ID_BASE;
use game_server::proto::*;
use game_server::world::{self, Cmd, Config, JoinReq};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;

const ITEMS: &str = r#"[
 {"id":"r_Apple","cost":2,"group":"food","stack":true,"weight":0.2},{"id":"r_Iron","cost":10,"group":"material","stack":true,"weight":1.0},
 {"id":"r_Rusty Sword","cost":10,"group":"weapon","weight":3.0},{"id":"r_Steel Sword","cost":220,"group":"weapon","weight":4.0},
 {"id":"r_Ship Cannon","cost":50,"group":"weapon","weight":50.0}]"#;
const WEAPONS: &str = r#"{"weapons":{"r_Rusty Sword":{"group":"sword","kind":"melee","reach":2.1,"angle":60,"rank":5},
 "r_Steel Sword":{"group":"sword","kind":"melee","reach":2.1,"angle":60,"rank":35},"r_Plain Axe":{"group":"axe","kind":"melee","reach":2.1,"angle":60,"rank":12},
 "r_Ship Cannon":{"group":"sword","kind":"melee","reach":2.1,"angle":60,"rank":40}},"clipDur":{"singlehand_attack_01":0.6}}"#;

/// A village at game (100, 200) with a merchant and a blacksmith at its centre (wire z = -200) and, if `mobs` names a difficulty, three such beasts 25 m away.
fn start(tag: &str, own_weapons: bool, mobs: &str) -> mpsc::Sender<Cmd> {
    let dir = std::env::temp_dir();
    let f = |n: &str| dir.join(format!("df_ptrade_{tag}_{n}.json"));
    std::fs::write(f("pois"), r#"[{"name":"Town","type":"Village","x":100,"z":200}]"#).unwrap();
    std::fs::write(f("caps"), r#"[{"name":"Capital","race":"Human","x":100,"y":0,"z":200}]"#).unwrap();
    std::fs::write(f("items"), ITEMS).unwrap();
    std::fs::write(f("weapons"), WEAPONS).unwrap();
    std::fs::write(f("vendors"), r#"[{"name":"Ann","kind":"merchant","x":105,"z":200},{"name":"Smith","kind":"blacksmith","x":95,"z":200}]"#).unwrap();
    std::fs::write(f("spawns"), if mobs.is_empty() { "[]".to_string() } else { format!(r#"[{{"id":1,"x":125,"z":200,"mobs":[{{"id":8,"name":"Beast","qty":3,"difficulty":"{mobs}"}}]}}]"#) }).unwrap();
    let s = |n: &str| Some(f(n).to_string_lossy().to_string());
    let cfg = Config { radius: 900.0, max_players: 4, tick_hz: 60, spawns: s("spawns"), weapons: s("weapons"), pvp: false, npcs: 1, pois: s("pois"), capitals: s("caps"), npc_seed: 2, db: None, nav: None, npc_time_scale: 1.0, own_weapons, body_secs: 45.0, tomb_secs: 3600, loot_secs: 3.0, items: s("items"), vendors: s("vendors") };
    let (cmds, rx) = mpsc::channel(256);
    tokio::spawn(world::run(cfg, rx));
    cmds
}

struct Me {
    id: u16,
    inbox: mpsc::Receiver<Message>,
    seen: Vec<serde_json::Value>,
}

impl Me {
    /// Everything that arrived so far, as JSON.
    async fn pump(&mut self) {
        tokio::time::sleep(Duration::from_millis(250)).await;
        while let Ok(m) = self.inbox.try_recv() {
            if let Message::Text(t) = m {
                self.seen.push(serde_json::from_str(&t).unwrap());
            }
        }
    }
    fn last(&self, t: &str) -> Option<&serde_json::Value> {
        self.seen.iter().rev().find(|v| v["t"] == t)
    }
    fn all(&self, t: &str) -> Vec<&serde_json::Value> {
        self.seen.iter().filter(|v| v["t"] == t).collect()
    }
}

async fn join(cmds: &mpsc::Sender<Cmd>, at: (f32, f32), fly: bool) -> Me {
    let (tx, inbox) = mpsc::channel(32768);
    let (reply, reply_rx) = oneshot::channel();
    cmds.send(Cmd::Join(JoinReq { name: "buyer".into(), race: "hum".into(), gender: "m".into(), gear: String::new(), tx, reply })).await.unwrap();
    let id = reply_rx.await.unwrap().unwrap();
    cmds.send(Cmd::Pos(id, PosUpdate { x: at.0, y: 0.0, z: at.1, flags: if fly { FLAG_FLY } else { 0 }, ..Default::default() })).await.unwrap();
    Me { id, inbox, seen: Vec::new() }
}

const MERCHANT: u16 = NPC_ID_BASE; // vendors come first among the ids, in the order of the file

fn give(gold: f32, items: &[(&str, u32)]) -> Stock {
    Stock { gold, items: items.iter().map(|(k, q)| (k.to_string(), *q)).collect() }
}

#[tokio::test]
async fn a_new_character_starts_with_the_worst_weapon_and_must_own_what_he_wields() {
    let cmds = start("own", true, "");
    let mut me = join(&cmds, (100.0, -200.0), true).await;
    me.pump().await;
    // the worst melee weapon of the game, in his hand and sheathed, he is told, and he carries nothing
    let eq = me.all("equip").into_iter().find(|v| v["id"].as_u64() == Some(me.id as u64)).expect("told about his weapon");
    assert_eq!((eq["w"].as_str(), eq["drawn"].as_bool()), (Some("r_Rusty Sword"), Some(false)));
    let st = me.last("stock").unwrap();
    assert_eq!(st["gold"], 0.0);
    assert!(st["items"].as_object().unwrap().is_empty());
    assert_eq!(st["weight"].as_f64(), Some(3.0), "the weapon in his hand weighs 3");
    assert_eq!(st["limit"].as_f64(), Some(CARRY_LIMIT as f64));
    // he can draw it, but not wield a weapon he does not have
    cmds.send(Cmd::Equip(me.id, "r_Rusty Sword".into(), true)).await.unwrap();
    me.seen.clear();
    cmds.send(Cmd::Equip(me.id, "r_Steel Sword".into(), true)).await.unwrap();
    me.pump().await;
    let refused = me.last("equip").expect("the refusal comes back as his real weapon");
    assert_eq!(refused["w"], "r_Rusty Sword");
    // once it is in his pack he can: the old one goes into the pack, the new one comes out
    cmds.send(Cmd::Give(me.id, give(0.0, &[("r_Steel Sword", 1)]))).await.unwrap();
    cmds.send(Cmd::Equip(me.id, "r_Steel Sword".into(), true)).await.unwrap();
    me.pump().await;
    let st = me.last("stock").unwrap();
    assert_eq!(st["items"]["r_Rusty Sword"], 1);
    assert!(st["items"].get("r_Steel Sword").is_none());
    assert_eq!(st["weight"].as_f64(), Some(7.0), "both swords");
    // sheathing it means nothing changes
    cmds.send(Cmd::Equip(me.id, "".into(), false)).await.unwrap();
    me.pump().await;
    assert_eq!(me.last("stock").unwrap()["items"]["r_Steel Sword"], 1, "bare hands: the sword is back in the pack");
}

#[tokio::test]
async fn he_opens_a_shop_buys_and_sells_at_the_vendors_prices() {
    let cmds = start("shop", false, "");
    let mut me = join(&cmds, (100.0, -200.0), true).await;
    cmds.send(Cmd::Give(me.id, give(300.0, &[("r_Iron", 4)]))).await.unwrap();
    cmds.send(Cmd::Shop(me.id, MERCHANT)).await.unwrap();
    me.pump().await;
    let shop = me.last("shop").expect("the shop view");
    assert_eq!(shop["name"], "Ann the Merchant");
    assert_eq!(shop["open"], true);
    let goods = shop["goods"].as_array().unwrap();
    assert!(!goods.is_empty() && goods.iter().all(|g| g["ask"].as_f64().unwrap() > 0.0 && g["w"].as_f64().unwrap() > 0.0 && g["qty"].as_u64().unwrap() > 0));
    // the merchant would pay for the iron he carries
    let iron_bid = shop["buys"].as_array().unwrap().iter().find(|b| b["item"] == "r_Iron").expect("he wants iron")["bid"].as_f64().unwrap();
    assert!(iron_bid > 0.0);
    // buy two of whatever it sells first
    let (item, ask) = (goods[0]["item"].as_str().unwrap().to_string(), goods[0]["ask"].as_f64().unwrap());
    let gold0 = me.last("stock").unwrap()["gold"].as_f64().unwrap();
    me.seen.clear();
    cmds.send(Cmd::Buy(me.id, MERCHANT, item.clone(), 2)).await.unwrap();
    me.pump().await;
    let t = me.last("trade").unwrap();
    assert_eq!((t["ok"].as_bool(), t["op"].as_str(), t["units"].as_u64()), (Some(true), Some("buy"), Some(2)), "{t}");
    let paid = t["gold"].as_f64().unwrap();
    assert!(paid >= ask * 2.0 * 0.99 && paid < ask * 2.0 * 1.6, "paid {paid} for two at about {ask}");
    let st = me.last("stock").unwrap();
    assert!((st["gold"].as_f64().unwrap() - (gold0 - paid)).abs() < 0.01);
    assert_eq!(st["items"][&item], 2);
    assert!(me.last("shop").is_some(), "a fresh shop view follows");
    // sell them back: he gets much less than he paid
    cmds.send(Cmd::Sell(me.id, MERCHANT, item.clone(), 2)).await.unwrap();
    me.pump().await;
    let t = me.last("trade").unwrap();
    assert_eq!((t["ok"].as_bool(), t["op"].as_str()), (Some(true), Some("sell")));
    assert!(t["gold"].as_f64().unwrap() < paid * 0.7, "the vendor keeps its margin: {t}");
}

#[tokio::test]
async fn trades_are_refused_for_the_right_reasons() {
    let cmds = start("refuse", false, "");
    let mut me = join(&cmds, (100.0, -200.0), true).await;
    let id = me.id;
    me.pump().await;
    let ask = |me: &Me| me.last("trade").map(|t| (t["ok"].as_bool().unwrap(), t["reason"].as_str().unwrap().to_string())).unwrap();
    async fn once(cmds: &mpsc::Sender<Cmd>, me: &mut Me, c: Cmd) {
        me.seen.clear();
        cmds.send(c).await.unwrap();
        me.pump().await;
    }
    // no such vendor, unknown item, not his trade, nothing to sell, too poor, too heavy
    once(&cmds, &mut me, Cmd::Buy(id, NPC_ID_BASE + 400, "r_Apple".into(), 1)).await;
    assert_eq!(ask(&me), (false, "no such vendor".into()));
    once(&cmds, &mut me, Cmd::Buy(id, MERCHANT, "r_Nothing".into(), 1)).await;
    assert_eq!(ask(&me), (false, "unknown item".into()));
    once(&cmds, &mut me, Cmd::Buy(id, MERCHANT, "r_Steel Sword".into(), 1)).await;
    assert_eq!(ask(&me), (false, "not in trade".into()), "a merchant does not sell swords");
    once(&cmds, &mut me, Cmd::Sell(id, MERCHANT, "r_Apple".into(), 1)).await;
    assert_eq!(ask(&me), (false, "nothing to sell".into()));
    once(&cmds, &mut me, Cmd::Buy(id, MERCHANT, "r_Iron".into(), 1)).await;
    let r = ask(&me);
    assert!(r == (false, "too poor".into()) || r == (false, "none left".into()), "{r:?}");
    cmds.send(Cmd::Give(me.id, give(100000.0, &[]))).await.unwrap();
    // the weight limit (60): a pack of 60 iron (weight 1 each) is full, and he is refused one more
    cmds.send(Cmd::Give(me.id, give(0.0, &[("r_Iron", 60)]))).await.unwrap();
    me.pump().await;
    once(&cmds, &mut me, Cmd::Buy(id, MERCHANT, "r_Iron".into(), 5)).await;
    assert_eq!(ask(&me), (false, "too heavy".into()), "he is carrying all he can");
    // too far to trade
    cmds.send(Cmd::Pos(me.id, PosUpdate { x: 100.0, y: 0.0, z: -260.0, flags: FLAG_FLY, ..Default::default() })).await.unwrap();
    once(&cmds, &mut me, Cmd::Shop(id, MERCHANT)).await;
    assert_eq!(ask(&me), (false, "too far".into()));
    assert!(me.last("shop").is_none());
}

#[tokio::test]
async fn a_purchase_is_cut_down_to_what_still_fits_in_the_pack() {
    let cmds = start("carry", true, "");
    let mut me = join(&cmds, (100.0, -200.0), true).await;
    // a ship cannon (weight 50) and the worst weapon in his hand (3): 7 weight units left, which is 35 apples of 0.2
    cmds.send(Cmd::Give(me.id, give(50000.0, &[("r_Ship Cannon", 1)]))).await.unwrap();
    cmds.send(Cmd::Shop(me.id, MERCHANT)).await.unwrap();
    me.pump().await;
    let apples = me.last("shop").unwrap()["goods"].as_array().unwrap().iter().find(|g| g["item"] == "r_Apple").map(|g| g["qty"].as_u64().unwrap()).expect("the merchant sells apples");
    me.seen.clear();
    cmds.send(Cmd::Buy(me.id, MERCHANT, "r_Apple".into(), 100)).await.unwrap();
    me.pump().await;
    let t = me.last("trade").unwrap();
    assert_eq!(t["ok"], true);
    assert_eq!(t["units"].as_u64().unwrap(), apples.min(35), "as many as the shelf holds or the pack carries: {t}");
    let st = me.last("stock").unwrap();
    assert!(st["weight"].as_f64().unwrap() <= CARRY_LIMIT as f64 + 1e-3, "{st}");
    if apples >= 35 {
        cmds.send(Cmd::Buy(me.id, MERCHANT, "r_Apple".into(), 1)).await.unwrap();
        me.pump().await;
        assert_eq!(me.last("trade").unwrap()["reason"], "too heavy", "the pack is full");
    }
    // the cannon goes into the pack of the blacksmith's trade: weapons are sold there, and a smith pays for it (it is in his trade, but not here)
    cmds.send(Cmd::Sell(me.id, MERCHANT, "r_Ship Cannon".into(), 1)).await.unwrap();
    me.pump().await;
    assert_eq!(me.last("trade").unwrap()["reason"], "not in trade", "a merchant does not buy weapons");
}

#[tokio::test]
async fn what_a_player_kills_drops_into_his_pack() {
    let cmds = start("drops", true, "Easy");
    let mut me = join(&cmds, (125.0, -197.0), false).await;
    cmds.send(Cmd::Equip(me.id, "r_Rusty Sword".into(), true)).await.unwrap();
    me.pump().await;
    let mut mob: Option<(f32, f32)> = None;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(40);
    let mut got = None;
    while tokio::time::Instant::now() < deadline && got.is_none() {
        while let Ok(m) = me.inbox.try_recv() {
            match m {
                Message::Binary(b) if b[0] == OP_MOBS && u16::from_le_bytes([b[5], b[6]]) > 0 => {
                    let f = |k: usize| f32::from_le_bytes([b[7 + k], b[8 + k], b[9 + k], b[10 + k]]);
                    mob = Some((f(4), f(12)));
                }
                Message::Text(t) => {
                    let v: serde_json::Value = serde_json::from_str(&t).unwrap();
                    if v["t"] == "drop" {
                        got = Some(v);
                    }
                    me.seen.push(serde_json::from_str(&t).unwrap());
                }
                _ => {}
            }
        }
        if let Some((mx, mz)) = mob {
            // stand just south of the nearest wolf, facing it, and keep swinging
            cmds.send(Cmd::Pos(me.id, PosUpdate { x: mx, y: 0.0, z: mz + 1.8, yaw: 0.0, ..Default::default() })).await.unwrap();
            cmds.send(Cmd::Attack(me.id, "melee".into(), "singlehand_attack_01".into())).await.unwrap();
        }
        tokio::time::sleep(Duration::from_millis(120)).await;
    }
    let drop = got.expect("a wolf died and dropped something");
    assert!(drop["gold"].as_f64().unwrap() >= 2.0);
    me.pump().await;
    let st = me.last("stock").unwrap();
    assert!(st["gold"].as_f64().unwrap() >= drop["gold"].as_f64().unwrap() - 0.01, "it is in his pack: {st}");
}

#[tokio::test]
async fn he_gets_up_again_with_the_worst_weapon_after_losing_everything() {
    let cmds = start("death", true, "Boss");
    let mut me = join(&cmds, (125.0, -200.0), false).await; // right among the bosses
    cmds.send(Cmd::Give(me.id, give(77.0, &[("r_Steel Sword", 1)]))).await.unwrap();
    // wield the steel sword: he owns it now
    cmds.send(Cmd::Equip(me.id, "r_Steel Sword".into(), false)).await.unwrap();
    let (mut died, mut respawned) = (false, false);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(40);
    while tokio::time::Instant::now() < deadline && !respawned {
        me.pump().await;
        for v in me.seen.drain(..).collect::<Vec<_>>() {
            if v["t"] == "died" && v["i"].as_u64() == Some(me.id as u64) {
                died = true;
            }
            if v["t"] == "respawn" && v["id"].as_u64() == Some(me.id as u64) {
                respawned = true;
            }
            me.seen.push(v);
        }
    }
    assert!(died && respawned, "the bosses killed him and he got up again");
    me.pump().await;
    let equips: Vec<&serde_json::Value> = me.all("equip").into_iter().filter(|v| v["id"].as_u64() == Some(me.id as u64)).collect();
    assert_eq!(equips.last().unwrap()["w"], "r_Rusty Sword", "the last thing he is told is the worst weapon");
    // everything he held is on his body now (the steel sword from his hand too); his pack is empty
    let st = me.last("stock").unwrap();
    assert_eq!(st["gold"], 0.0);
    assert!(st["items"].as_object().unwrap().is_empty(), "{st}");
}
