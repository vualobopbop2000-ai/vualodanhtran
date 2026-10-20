use axum::{
    extract::{ws::{Message, WebSocket, WebSocketUpgrade}, State},
    response::IntoResponse,
    routing::get,
    Router,
};
use futures_util::{SinkExt, StreamExt};
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::HashMap, env, sync::{Arc, Mutex}, time::Duration};
use tokio::{net::TcpListener, sync::mpsc};
use tower_http::services::ServeDir;

const CASTLE_HP: f32 = 5000.0;
const MAP_X: f32 = 60.0;
const MAP_Z: f32 = 40.0;

type ClientId = u64;
type Shared = Arc<Mutex<ServerState>>;

#[derive(Clone)]
struct AppState { shared: Shared }

struct ServerState {
    clients: HashMap<ClientId, Client>,
    rooms: HashMap<String, Room>,
    next_client_id: ClientId,
    next_room_id: u64,
}
struct Client {
    tx: mpsc::UnboundedSender<String>,
    name: String,
    room_id: Option<String>,
    seat: Option<usize>,
}
struct Room {
    id: String,
    name: String,
    password: String,
    players: [Option<ClientId>; 2],
    names: [Option<String>; 2],
    colors: [String; 2],
    game: Option<GameState>,
}

#[derive(Clone, Serialize, Deserialize)]
struct PlayerState {
    name: String,
    color: String,
    gold: f32,
    castle_hp: f32,
    buff: String,
    buff_until: f32,
    potion_ready_at: f32,
}
#[derive(Clone, Serialize, Deserialize)]
struct Unit {
    id: u64,
    owner: usize,
    kind: String,
    x: f32,
    z: f32,
    hp: f32,
    max_hp: f32,
    level: u8,
    command: String,
    dest_x: f32,
    dest_z: f32,
    attack_cd: f32,
    burn_remaining: f32,
    burn_dps: f32,
    burn_tick: f32,
    poison_remaining: f32,
    poison_dps: f32,
    summon_cd: f32,
    charge_cd: f32,
}
#[derive(Clone, Serialize, Deserialize)]
struct GameState {
    players: [PlayerState; 2],
    units: Vec<Unit>,
    elapsed: f32,
    winner: Option<usize>,
    next_unit_id: u64,
}
#[derive(Clone, Serialize)]
struct RoomListing {
    id: String,
    name: String,
    players: usize,
    locked: bool,
}

#[tokio::main]
async fn main() {
    let shared = Arc::new(Mutex::new(ServerState {
        clients: HashMap::new(), rooms: HashMap::new(), next_client_id: 1, next_room_id: 1,
    }));
    let state = AppState { shared: shared.clone() };
    tokio::spawn(tick_loop(shared));

    let app = Router::new()
        .route("/ws", get(ws_handler))
        .fallback_service(ServeDir::new("static").append_index_html_on_directories(true))
        .with_state(state);
    let port = env::var("PORT").ok().and_then(|s| s.parse::<u16>().ok()).unwrap_or(10000);
    let listener = TcpListener::bind(("0.0.0.0", port)).await.expect("bind port");
    println!("Emperor's Great Battle listening on 0.0.0.0:{port}");
    axum::serve(listener, app).await.expect("serve");
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state.shared))
}

async fn handle_socket(socket: WebSocket, shared: Shared) {
    let (mut ws_tx, mut ws_rx) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    let id = {
        let mut s = shared.lock().unwrap();
        let id = s.next_client_id;
        s.next_client_id += 1;
        s.clients.insert(id, Client { tx, name: format!("Guest-{id}"), room_id: None, seat: None });
        id
    };
    let writer = tokio::spawn(async move {
        while let Some(text) = rx.recv().await {
            if ws_tx.send(Message::Text(text.into())).await.is_err() { break; }
        }
    });
    send_json(&shared, id, json!({"type":"connected","clientId":id}));
    send_rooms(&shared);

    while let Some(Ok(msg)) = ws_rx.next().await {
        match msg {
            Message::Text(text) => handle_message(&shared, id, text.as_str()),
            Message::Close(_) => break,
            Message::Ping(_) | Message::Pong(_) | Message::Binary(_) => {},
        }
    }
    disconnect_client(&shared, id);
    writer.abort();
}

fn handle_message(shared: &Shared, client_id: ClientId, text: &str) {
    let Ok(v) = serde_json::from_str::<Value>(text) else { return; };
    let Some(kind) = v.get("type").and_then(Value::as_str) else { return; };
    match kind {
        "set_name" => {
            let name = clean_name(v.get("name").and_then(Value::as_str).unwrap_or(""));
            if name.chars().count() < 2 { send_json(shared, client_id, json!({"type":"error","message":"Hãy nhập tên từ 2 đến 20 ký tự."})); return; }
            let mut s = shared.lock().unwrap();
            if let Some(c) = s.clients.get_mut(&client_id) { c.name = name.clone(); }
            send_json_locked(&s, client_id, &json!({"type":"name_saved","name":name}));
        }
        "list_rooms" => send_rooms(shared),
        "create_room" => create_room(shared, client_id, &v),
        "join_room" => join_room(shared, client_id, &v),
        "leave_room" => leave_room(shared, client_id),
        "place_unit" => place_unit(shared, client_id, &v),
        "unit_command" => command_units(shared, client_id, &v),
        "upgrade_unit" => upgrade_unit(shared, client_id, &v),
        "roll_potion" => roll_potion(shared, client_id),
        "ping" => send_json(shared, client_id, json!({"type":"pong"})),
        _ => send_json(shared, client_id, json!({"type":"error","message":"Lệnh không hợp lệ."})),
    }
}

fn clean_name(input: &str) -> String {
    input.chars().filter(|c| !c.is_control()).take(20).collect::<String>().trim().to_string()
}
fn clean_room_name(input: &str) -> String {
    input.chars().filter(|c| !c.is_control()).take(32).collect::<String>().trim().to_string()
}
fn send_json(shared: &Shared, id: ClientId, value: Value) {
    let s = shared.lock().unwrap();
    send_json_locked(&s, id, &value);
}
fn send_json_locked(s: &ServerState, id: ClientId, value: &Value) {
    if let Some(c) = s.clients.get(&id) { let _ = c.tx.send(value.to_string()); }
}
fn send_room_locked(s: &ServerState, room_id: &str, value: &Value) {
    if let Some(room) = s.rooms.get(room_id) {
        for id in room.players.iter().flatten() { send_json_locked(s, *id, value); }
    }
}
fn send_rooms(shared: &Shared) {
    let s = shared.lock().unwrap();
    let rooms: Vec<RoomListing> = s.rooms.values().filter(|r| r.players.iter().any(Option::is_some) && r.game.as_ref().map_or(true, |g| g.winner.is_none())).map(|r| RoomListing {
        id: r.id.clone(), name: r.name.clone(), players: r.players.iter().filter(|p| p.is_some()).count(), locked: !r.password.is_empty(),
    }).collect();
    let msg = json!({"type":"rooms","rooms":rooms});
    for c in s.clients.values() { let _ = c.tx.send(msg.to_string()); }
}

fn create_room(shared: &Shared, client_id: ClientId, v: &Value) {
    let room_name = clean_room_name(v.get("roomName").and_then(Value::as_str).unwrap_or(""));
    let password = v.get("password").and_then(Value::as_str).unwrap_or("").chars().take(48).collect::<String>();
    if room_name.len() < 2 {
        send_json(shared, client_id, json!({"type":"error","message":"Tên phòng phải có ít nhất 2 ký tự."})); return;
    }
    let mut s = shared.lock().unwrap();
    let Some(client) = s.clients.get(&client_id) else { return; };
    if client.room_id.is_some() {
        send_json_locked(&s, client_id, &json!({"type":"error","message":"Bạn đang ở trong một phòng."})); return;
    }
    if s.rooms.values().any(|r| r.name.to_lowercase() == room_name.to_lowercase()) {
        send_json_locked(&s, client_id, &json!({"type":"room_name_taken","message":"Tên phòng bị trùng, đặt tên khác."})); return;
    }
    let username = client.name.clone();
    let n = s.next_room_id; s.next_room_id += 1;
    let id = format!("battle-{n}");
    let mut rng = rand::thread_rng();
    let colors = if rng.gen_bool(0.5) { ["blue".to_string(), "red".to_string()] } else { ["red".to_string(), "blue".to_string()] };
    let mut room = Room {
        id: id.clone(), name: room_name.clone(), password, players: [Some(client_id), None],
        names: [Some(username.clone()), None], colors, game: None,
    };
    room.game = None;
    s.rooms.insert(id.clone(), room);
    if let Some(c) = s.clients.get_mut(&client_id) { c.room_id = Some(id.clone()); c.seat = Some(0); }
    send_json_locked(&s, client_id, &json!({"type":"joined","roomId":id,"roomName":room_name,"seat":0,"waiting":true,"players":[username,null]}));
    send_room_locked(&s, &id, &json!({"type":"room_status","roomName":room_name,"players":[username,null],"waiting":true}));
    drop(s);
    send_rooms(shared);
}

fn join_room(shared: &Shared, client_id: ClientId, v: &Value) {
    let room_id = v.get("roomId").and_then(Value::as_str).unwrap_or("").to_string();
    let password = v.get("password").and_then(Value::as_str).unwrap_or("");
    let mut s = shared.lock().unwrap();
    let Some(client) = s.clients.get(&client_id) else { return; };
    if client.room_id.is_some() { send_json_locked(&s, client_id, &json!({"type":"error","message":"Bạn đang ở trong một phòng."})); return; }
    let username = client.name.clone();
    let Some(room) = s.rooms.get(&room_id) else { send_json_locked(&s, client_id, &json!({"type":"error","message":"Phòng không còn tồn tại."})); return; };
    if room.game.as_ref().and_then(|g| g.winner).is_some() { send_json_locked(&s, client_id, &json!({"type":"error","message":"Trận đấu ở phòng này đã kết thúc."})); return; }
    if room.password != password { send_json_locked(&s, client_id, &json!({"type":"error","message":"Sai mật khẩu phòng."})); return; }
    let Some(seat) = room.players.iter().position(Option::is_none) else { send_json_locked(&s, client_id, &json!({"type":"error","message":"Phòng đã đủ 2 người."})); return; };
    let room_name = room.name.clone();
    let r_id = room.id.clone();
    let (players_json, game_started, game_state) = {
        let room = s.rooms.get_mut(&room_id).unwrap();
        room.players[seat] = Some(client_id);
        room.names[seat] = Some(username.clone());
        let names = room.names.clone();
        if room.players.iter().all(Option::is_some) {
            room.game = Some(new_game(names, room.colors.clone()));
        }
        let playing = room.game.is_some();
        let game = room.game.clone();
        (json!([room.names[0], room.names[1]]), playing, game)
    };
    if let Some(c) = s.clients.get_mut(&client_id) { c.room_id = Some(r_id.clone()); c.seat = Some(seat); }
    send_json_locked(&s, client_id, &json!({"type":"joined","roomId":r_id,"roomName":room_name,"seat":seat,"waiting":!game_started,"players":players_json,"game":game_state}));
    send_room_locked(&s, &r_id, &json!({"type":"room_status","roomName":room_name,"players":players_json,"waiting":!game_started}));
    if game_started { send_room_locked(&s, &r_id, &json!({"type":"match_found","players":players_json})); if let Some(g) = game_state { send_room_locked(&s, &r_id, &json!({"type":"game_update","game":g})); } }
    drop(s);
    send_rooms(shared);
}

fn leave_room(shared: &Shared, client_id: ClientId) {
    let mut s = shared.lock().unwrap();
    let Some(room_id) = s.clients.get(&client_id).and_then(|c| c.room_id.clone()) else {
        send_json_locked(&s, client_id, &json!({"type":"left_room"}));
        return;
    };
    let ended = s.rooms.get(&room_id).and_then(|r| r.game.as_ref()).and_then(|g| g.winner).is_some();
    if ended {
        // Once a finished match is left, close the ephemeral room and send any remaining guest back to the lobby.
        if let Some(room) = s.rooms.remove(&room_id) {
            for other_id in room.players.iter().flatten().copied().filter(|id| *id != client_id) {
                if let Some(other) = s.clients.get_mut(&other_id) { other.room_id = None; other.seat = None; }
                send_json_locked(&s, other_id, &json!({"type":"room_closed","message":"Phòng đã đóng sau khi trận kết thúc."}));
            }
        }
    } else {
        remove_from_room_locked(&mut s, client_id, &room_id, true);
    }
    if let Some(c) = s.clients.get_mut(&client_id) { c.room_id = None; c.seat = None; }
    send_json_locked(&s, client_id, &json!({"type":"left_room"}));
    drop(s);
    send_rooms(shared);
}

fn remove_from_room_locked(s: &mut ServerState, client_id: ClientId, room_id: &str, announce: bool) {
    let mut delete = false;
    let mut recipients: Vec<ClientId> = Vec::new();
    let mut final_game: Option<GameState> = None;
    let mut departed_seat: Option<usize> = None;
    if let Some(room) = s.rooms.get_mut(room_id) {
        if let Some(seat) = room.players.iter().position(|p| *p == Some(client_id)) {
            departed_seat = Some(seat);
            room.players[seat] = None;
            room.names[seat] = None;
            recipients = room.players.iter().flatten().copied().collect();
            if announce {
                if let Some(game) = room.game.as_mut() {
                    if game.winner.is_none() {
                        game.winner = Some(1 - seat);
                    }
                    final_game = Some(game.clone());
                }
            }
        }
        delete = room.players.iter().all(Option::is_none);
    }
    if let Some(seat) = departed_seat {
        let msg = json!({"type":"opponent_left","seat":seat});
        for id in &recipients { send_json_locked(s, *id, &msg); }
    }
    if let Some(game) = final_game {
        let msg = json!({"type":"game_update","game":game});
        for id in &recipients { send_json_locked(s, *id, &msg); }
    }
    if delete { s.rooms.remove(room_id); }
}

fn disconnect_client(shared: &Shared, id: ClientId) {
    let mut s = shared.lock().unwrap();
    let rid = s.clients.get(&id).and_then(|c| c.room_id.clone());
    if let Some(rid) = rid { remove_from_room_locked(&mut s, id, &rid, true); }
    s.clients.remove(&id);
    drop(s);
    send_rooms(shared);
}

fn new_game(names: [Option<String>; 2], colors: [String; 2]) -> GameState {
    let p0 = PlayerState { name: names[0].clone().unwrap_or_else(|| "Guest Blue".into()), color: colors[0].clone(), gold: 25.0, castle_hp: CASTLE_HP, buff: "none".into(), buff_until: 0.0, potion_ready_at: 0.0 };
    let p1 = PlayerState { name: names[1].clone().unwrap_or_else(|| "Guest Red".into()), color: colors[1].clone(), gold: 25.0, castle_hp: CASTLE_HP, buff: "none".into(), buff_until: 0.0, potion_ready_at: 0.0 };
    GameState { players: [p0, p1], units: Vec::new(), elapsed: 0.0, winner: None, next_unit_id: 1 }
}

fn unit_price(kind: &str) -> f32 {
    match kind { "gold_mine" => 0.0, "vanguard" => 5.0, "archer" => 7.0, "shield" => 7.0, "mage" => 11.0, "cavalry" => 10.0, "elephant_lancer" => 12.0, "catapult" => 14.0, "general" => 15.0, "royal" => 0.0, _ => 9999.0 }
}
fn base_stats(kind: &str) -> (f32, f32, f32, f32, f32, f32, f32) {
    // ATK, DEF, SPEED, ATK SPEED, range, base HP, splash radius
    match kind {
        "gold_mine" => (0.0, 0.0, 0.0, 0.0, 0.0, 300.0, 0.0),
        "vanguard" => (4.0, 4.0, 3.0, 5.0, 1.35, 60.0, 0.0),
        "archer" => (8.0, 2.0, 2.0, 7.0, 13.0, 36.0, 0.0),
        "shield" => (2.0, 12.0, 2.0, 3.0, 1.5, 112.0, 0.0),
        "mage" => (7.0, 2.0, 2.0, 5.0, 10.5, 44.0, 2.8),
        "cavalry" => (8.0, 5.0, 5.0, 5.0, 1.6, 75.0, 0.0),
        "elephant_lancer" => (9.0, 7.0, 5.0, 6.0, 3.6, 125.0, 0.0),
        "catapult" => (15.0, 5.0, 1.0, 2.0, 18.0, 92.0, 4.0),
        "general" => (2.0, 6.0, 0.0, 2.0, 4.1, 260.0, 0.0),
        "royal" => (5.5, 5.5, 4.5, 6.5, 1.45, 82.0, 0.0),
        _ => (1.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0),
    }
}
fn create_unit(game: &mut GameState, owner: usize, kind: &str, x: f32, z: f32, level: u8) -> Unit {
    let (_, _, _, _, _, hp, _) = base_stats(kind);
    let level = level.clamp(1, 5);
    let max_hp = hp * (1.0 + 0.1 * (level as f32 - 1.0));
    let id = game.next_unit_id; game.next_unit_id += 1;
    Unit { id, owner, kind: kind.into(), x, z, hp: max_hp, max_hp, level, command: "attack".into(), dest_x: x, dest_z: z, attack_cd: 0.15, burn_remaining: 0.0, burn_dps: 0.0, burn_tick: 0.5, poison_remaining: 0.0, poison_dps: 0.0, summon_cd: 3.0, charge_cd: 6.0 }
}

fn place_unit(shared: &Shared, client_id: ClientId, v: &Value) {
    let kind = v.get("kind").and_then(Value::as_str).unwrap_or("").to_string();
    let x = v.get("x").and_then(Value::as_f64).unwrap_or(0.0) as f32;
    let z = v.get("z").and_then(Value::as_f64).unwrap_or(0.0) as f32;
    let level = v.get("level").and_then(Value::as_u64).unwrap_or(1).clamp(1, 5) as u8;
    if !["gold_mine","vanguard","archer","shield","mage","cavalry","elephant_lancer","catapult","general"].contains(&kind.as_str()) {
        send_json(shared, client_id, json!({"type":"error","message":"Loại quân không hợp lệ."})); return;
    }
    let mut s = shared.lock().unwrap();
    let (room_id, seat) = match s.clients.get(&client_id) {
        Some(c) => match (c.room_id.clone(), c.seat) {
            (Some(r), Some(seat)) => (r, seat),
            _ => { send_json_locked(&s, client_id, &json!({"type":"error","message":"Hãy vào một phòng trước."})); return; }
        },
        None => return,
    };
    let result: Result<GameState, String> = (|| {
        let room = s.rooms.get_mut(&room_id).ok_or_else(|| "Phòng không còn tồn tại.".to_string())?;
        let game = room.game.as_mut().ok_or_else(|| "Đang chờ đối thủ vào phòng.".to_string())?;
        if game.winner.is_some() { return Err("Trận đấu đã kết thúc.".into()); }
        if x.abs() > MAP_X - 6.0 || z.abs() > MAP_Z - 3.0 || (seat == 0 && x > -3.2) || (seat == 1 && x < 3.2) {
            return Err("Chỉ được thả quân trong nửa sân của phe mình.".into());
        }
        if kind == "gold_mine" && game.units.iter().filter(|u| u.owner == seat && u.kind == "gold_mine").count() >= 5 {
            return Err("Mỗi phe chỉ được đặt tối đa 5 mỏ vàng.".into());
        }
        let effective_level = if kind == "gold_mine" { 1 } else { level };
        let price = unit_price(&kind) * (1.0 + (effective_level as f32 - 1.0) * 0.22);
        if game.players[seat].gold + 0.001 < price { return Err(format!("Không đủ vàng. Cần {:.0} vàng.", price)); }
        game.players[seat].gold -= price;
        let unit = create_unit(game, seat, &kind, x, z, effective_level);
        game.units.push(unit);
        Ok(game.clone())
    })();
    match result {
        Ok(snapshot) => send_room_locked(&s, &room_id, &json!({"type":"game_update","game":snapshot})),
        Err(message) => send_json_locked(&s, client_id, &json!({"type":"error","message":message})),
    }
}

fn command_units(shared: &Shared, client_id: ClientId, v: &Value) {
    let command = v.get("command").and_then(Value::as_str).unwrap_or("attack");
    if !["attack","defend","occupy"].contains(&command) { return; }
    let ids: Vec<u64> = v.get("ids").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_u64).collect()).unwrap_or_default();
    let x = v.get("x").and_then(Value::as_f64).unwrap_or(0.0) as f32;
    let z = v.get("z").and_then(Value::as_f64).unwrap_or(0.0) as f32;
    let mut s = shared.lock().unwrap();
    let Some(c) = s.clients.get(&client_id) else { return; };
    let (Some(room_id), Some(seat)) = (c.room_id.clone(), c.seat) else { return; };
    if command == "occupy" && ((seat == 0 && x > -3.2) || (seat == 1 && x < 3.2)) {
        send_json_locked(&s, client_id, &json!({"type":"error","message":"Điểm chiếm đóng phải nằm trong phần sân của bạn."})); return;
    }
    let Some(room) = s.rooms.get_mut(&room_id) else { return; };
    let Some(game) = room.game.as_mut() else { return; };
    for unit in &mut game.units {
        if unit.owner == seat && ids.contains(&unit.id) && unit.kind != "gold_mine" {
            unit.command = command.to_string();
            if command == "occupy" { unit.dest_x = x.clamp(-MAP_X + 4.0, MAP_X - 4.0); unit.dest_z = z.clamp(-MAP_Z + 4.0, MAP_Z - 4.0); }
        }
    }
    let snapshot = game.clone();
    send_room_locked(&s, &room_id, &json!({"type":"game_update","game":snapshot}));
}

fn upgrade_unit(shared: &Shared, client_id: ClientId, v: &Value) {
    let uid = v.get("id").and_then(Value::as_u64).unwrap_or(0);
    let mut s = shared.lock().unwrap();
    let (room_id, seat) = match s.clients.get(&client_id) {
        Some(c) => match (c.room_id.clone(), c.seat) {
            (Some(r), Some(seat)) => (r, seat),
            _ => { send_json_locked(&s, client_id, &json!({"type":"error","message":"Hãy vào một phòng trước."})); return; }
        },
        None => return,
    };
    let result: Result<GameState, String> = (|| {
        let room = s.rooms.get_mut(&room_id).ok_or_else(|| "Phòng không còn tồn tại.".to_string())?;
        let game = room.game.as_mut().ok_or_else(|| "Trận chưa bắt đầu.".to_string())?;
        let index = game.units.iter().position(|u| u.id == uid && u.owner == seat)
            .ok_or_else(|| "Hãy chọn một quân hoặc công trình của phe mình.".to_string())?;
        let old_level = game.units[index].level;
        if old_level >= 5 { return Err("Đã đạt cấp tối đa 5.".into()); }
        let price = (unit_price(&game.units[index].kind).max(3.0) * (old_level as f32 + 1.0) * 0.7).ceil();
        if game.players[seat].gold < price { return Err(format!("Nâng cấp cần {:.0} vàng.", price)); }
        game.players[seat].gold -= price;
        let unit = &mut game.units[index];
        let old_max = unit.max_hp;
        unit.level += 1;
        let (_, _, _, _, _, base_hp, _) = base_stats(&unit.kind);
        unit.max_hp = base_hp * (1.0 + 0.1 * (unit.level as f32 - 1.0));
        unit.hp = (unit.hp + (unit.max_hp - old_max)).min(unit.max_hp);
        Ok(game.clone())
    })();
    match result {
        Ok(snapshot) => send_room_locked(&s, &room_id, &json!({"type":"game_update","game":snapshot})),
        Err(message) => send_json_locked(&s, client_id, &json!({"type":"error","message":message})),
    }
}

fn roll_potion(shared: &Shared, client_id: ClientId) {
    let mut s = shared.lock().unwrap();
    let (room_id, seat) = match s.clients.get(&client_id) {
        Some(c) => match (c.room_id.clone(), c.seat) {
            (Some(r), Some(seat)) => (r, seat),
            _ => { send_json_locked(&s, client_id, &json!({"type":"error","message":"Hãy vào một phòng trước."})); return; }
        },
        None => return,
    };
    let result: Result<(GameState, String), String> = (|| {
        let room = s.rooms.get_mut(&room_id).ok_or_else(|| "Phòng không còn tồn tại.".to_string())?;
        let game = room.game.as_mut().ok_or_else(|| "Trận chưa bắt đầu.".to_string())?;
        if game.winner.is_some() { return Err("Trận đấu đã kết thúc.".into()); }
        if game.elapsed + 0.001 < game.players[seat].potion_ready_at {
            return Err(format!("Lotion đang hồi chiêu: {:.0}s.", (game.players[seat].potion_ready_at - game.elapsed).ceil()));
        }
        const POTIONS: [&str; 5] = ["frenzy","agility","haste","heal","poison"];
        let chosen = POTIONS[rand::thread_rng().gen_range(0..POTIONS.len())].to_string();
        let now = game.elapsed;
        game.players[seat].potion_ready_at = now + 15.0;
        match chosen.as_str() {
            "frenzy" | "agility" | "haste" | "poison" => { game.players[seat].buff = chosen.clone(); game.players[seat].buff_until = now + 10.0; }
            "heal" => for u in game.units.iter_mut().filter(|u| u.owner == seat) { u.hp = (u.hp + u.max_hp * 0.25).min(u.max_hp); },
            _ => {}
        }
        Ok((game.clone(), chosen))
    })();
    match result {
        Ok((snapshot, chosen)) => {
            send_room_locked(&s, &room_id, &json!({"type":"potion_rolled","owner":seat,"potion":chosen}));
            send_room_locked(&s, &room_id, &json!({"type":"game_update","game":snapshot}));
        }
        Err(message) => send_json_locked(&s, client_id, &json!({"type":"error","message":message})),
    }
}

fn stats_for(unit: &Unit, game: &GameState) -> (f32, f32, f32, f32, f32, f32) {
    let (mut atk, mut def, mut speed, mut atk_speed, range, _, splash) = base_stats(&unit.kind);
    let level_mul = 1.0 + 0.1 * (unit.level as f32 - 1.0);
    atk *= level_mul; def *= level_mul; speed *= 1.0 + 0.04 * (unit.level as f32 - 1.0); atk_speed *= 1.0 + 0.04 * (unit.level as f32 - 1.0);
    let p = &game.players[unit.owner];
    if p.buff_until > game.elapsed {
        match p.buff.as_str() {
            "frenzy" => { atk += 2.0; def += 2.0; speed += 2.0; atk_speed += 2.0; }
            "agility" => speed += 3.0,
            "haste" => atk_speed += 3.0,
            _ => {}
        }
    }
    (atk, def, speed, atk_speed, range, splash)
}
fn damage_after_defense(atk: f32, defense: f32) -> f32 { (atk - defense * 0.35).max(1.0) }
fn dist(x1: f32, z1: f32, x2: f32, z2: f32) -> f32 { ((x1-x2).powi(2) + (z1-z2).powi(2)).sqrt() }

async fn tick_loop(shared: Shared) {
    let mut interval = tokio::time::interval(Duration::from_millis(200));
    loop {
        interval.tick().await;
        let mut s = shared.lock().unwrap();
        let room_ids: Vec<String> = s.rooms.keys().cloned().collect();
        for room_id in room_ids {
            let mut update = None;
            if let Some(room) = s.rooms.get_mut(&room_id) {
                if let Some(game) = room.game.as_mut() {
                    if game.winner.is_none() {
                        tick_game(game, 0.2);
                        update = Some(game.clone());
                    }
                }
            }
            if let Some(game) = update { send_room_locked(&s, &room_id, &json!({"type":"game_update","game":game})); }
        }
    }
}

fn tick_game(game: &mut GameState, dt: f32) {
    game.elapsed += dt;
    for p in &mut game.players {
        if p.buff_until <= game.elapsed { p.buff = "none".into(); }
    }
    for owner in 0..2 {
        let income: f32 = game.units.iter().filter(|u| u.owner == owner && u.kind == "gold_mine").map(|u| u.level as f32).sum();
        game.players[owner].gold += income * dt;
    }

    let snapshot = game.units.clone();
    let original_len = game.units.len();
    let mut damage_events: Vec<(u64, f32)> = Vec::new();
    let mut burn_events: Vec<(u64, f32)> = Vec::new();
    let mut poison_events: Vec<(u64, f32)> = Vec::new();
    let mut castle_events: Vec<(usize, f32)> = Vec::new();
    let mut new_units: Vec<Unit> = Vec::new();

    for i in 0..original_len {
        if game.units[i].hp <= 0.0 || game.units[i].kind == "gold_mine" { continue; }
        let owner = game.units[i].owner;

        if game.units[i].burn_remaining > 0.0 {
            game.units[i].burn_remaining = (game.units[i].burn_remaining - dt).max(0.0);
            game.units[i].burn_tick -= dt;
            while game.units[i].burn_tick <= 0.0 && game.units[i].burn_remaining > 0.0 {
                damage_events.push((game.units[i].id, game.units[i].burn_dps * 0.5));
                game.units[i].burn_tick += 0.5;
            }
        }
        if game.units[i].poison_remaining > 0.0 {
            damage_events.push((game.units[i].id, game.units[i].poison_dps * dt));
            game.units[i].poison_remaining = (game.units[i].poison_remaining - dt).max(0.0);
        }

        game.units[i].attack_cd = (game.units[i].attack_cd - dt).max(0.0);
        game.units[i].charge_cd = (game.units[i].charge_cd - dt).max(0.0);
        if game.units[i].kind == "general" { game.units[i].summon_cd -= dt; }
        if game.units[i].kind == "general" && game.units[i].summon_cd <= 0.0 {
            game.units[i].summon_cd = 3.0;
            let summoner = game.units[i].clone();
            let count = 2 + summoner.level as usize - 1;
            for k in 0..count {
                let offset = (k as f32 - (count as f32 - 1.0) / 2.0) * 0.9;
                let sx = (summoner.x + offset).clamp(-MAP_X + 3.0, MAP_X - 3.0);
                let sz = (summoner.z + 1.5).clamp(-MAP_Z + 3.0, MAP_Z - 3.0);
                new_units.push(create_unit(game, owner, "royal", sx, sz, summoner.level));
            }
        }

        // Work on a copy so movement/combat can safely queue effects for later application.
        let cur = game.units[i].clone();
        let (atk, _def, speed, atk_speed, range, splash) = stats_for(&cur, game);
        let enemy_target = snapshot.iter().filter(|u| u.owner != owner && u.hp > 0.0)
            .min_by(|a,b| dist(cur.x,cur.z,a.x,a.z).partial_cmp(&dist(cur.x,cur.z,b.x,b.z)).unwrap_or(std::cmp::Ordering::Equal));
        let mut target_pos: Option<(f32,f32,u64)> = None;
        if let Some(t) = enemy_target {
            match cur.command.as_str() {
                "defend" if dist(cur.x,cur.z,t.x,t.z) > 10.0 => {},
                "occupy" if dist(cur.x,cur.z,t.x,t.z) > 11.0 => {},
                _ => target_pos = Some((t.x,t.z,t.id)),
            }
        }
        if target_pos.is_none() && cur.command == "occupy" { target_pos = Some((cur.dest_x,cur.dest_z,u64::MAX)); }
        if target_pos.is_none() && cur.command == "defend" {
            // Pull back toward the friendly keep, then hold the defensive perimeter.
            let home_x = if owner == 0 { -MAP_X + 12.0 } else { MAP_X - 12.0 };
            if dist(cur.x,cur.z,home_x,0.0) > 7.0 { target_pos = Some((home_x,0.0,u64::MAX)); }
        }
        if target_pos.is_none() && cur.command == "attack" && enemy_target.is_none() {
            let castle_x = if owner == 0 { MAP_X - 12.0 } else { -MAP_X + 12.0 };
            target_pos = Some((castle_x,0.0,0));
        }
        if target_pos.is_none() { continue; }
        let (tx,tz,target_id) = target_pos.unwrap();
        let d = dist(cur.x,cur.z,tx,tz);
        let mut move_speed = speed;
        if cur.kind == "cavalry" && game.units[i].charge_cd <= 0.0 && d > 4.0 {
            move_speed *= 4.0;
            game.units[i].charge_cd = 6.0;
            for enemy in snapshot.iter().filter(|u| u.owner != owner && u.hp > 0.0 && point_segment_distance(u.x,u.z,cur.x,cur.z,tx,tz) < 1.7) {
                damage_events.push((enemy.id, damage_after_defense(atk * 1.5, base_stats(&enemy.kind).1)));
                if game.players[owner].buff == "poison" && game.players[owner].buff_until > game.elapsed {
                    poison_events.push((enemy.id, atk));
                }
            }
        }
        let is_destination = target_id == u64::MAX;
        let reach = if target_id == 0 { range + 4.0 } else if is_destination { 0.65 } else { range + 0.35 };
        let in_range = d <= reach;
        if !in_range && move_speed > 0.0 {
            let desired_gap = if is_destination { 0.5 } else { range.max(0.4) };
            let step_base = (move_speed * dt).min((d - desired_gap).max(0.0));
            let mut next_x = cur.x;
            let mut next_z = cur.z;
            if d > 0.001 {
                next_x += (tx-cur.x)/d * step_base;
                next_z += (tz-cur.z)/d * step_base;
            }
            // Wading through the shallow river slows troops briefly.
            if (cur.x * next_x <= 0.0 && (cur.x.abs() < 4.0 || next_x.abs() < 4.0)) || next_x.abs() < 2.0 {
                next_x = cur.x + (next_x-cur.x)*0.55;
                next_z = cur.z + (next_z-cur.z)*0.55;
            }
            game.units[i].x = next_x.clamp(-MAP_X+2.0, MAP_X-2.0);
            game.units[i].z = next_z.clamp(-MAP_Z+2.0, MAP_Z-2.0);
        } else if !is_destination && game.units[i].attack_cd <= 0.0 && (target_id != 0 || d < range + 5.0) {
            game.units[i].attack_cd = (1.6 - atk_speed * 0.18).clamp(0.24, 1.3);
            if target_id == 0 {
                let enemy_owner = 1 - owner;
                castle_events.push((enemy_owner, damage_after_defense(atk, 8.0)));
            } else if let Some(target) = snapshot.iter().find(|u| u.id == target_id) {
                if splash > 0.0 {
                    for enemy in snapshot.iter().filter(|u| u.owner != owner && u.hp > 0.0 && dist(tx,tz,u.x,u.z) <= splash) {
                        damage_events.push((enemy.id, damage_after_defense(atk, base_stats(&enemy.kind).1)));
                        if cur.kind == "mage" { burn_events.push((enemy.id, atk)); }
                        if game.players[owner].buff == "poison" && game.players[owner].buff_until > game.elapsed { poison_events.push((enemy.id, atk)); }
                    }
                } else {
                    damage_events.push((target_id, damage_after_defense(atk, base_stats(&target.kind).1)));
                    if cur.kind == "mage" { burn_events.push((target_id, atk)); }
                    if game.players[owner].buff == "poison" && game.players[owner].buff_until > game.elapsed { poison_events.push((target_id, atk)); }
                }
            }
        }
    }

    for (id, damage) in damage_events {
        if let Some(u) = game.units.iter_mut().find(|u| u.id == id) { u.hp -= damage; }
    }
    for (id, atk) in burn_events { if let Some(u) = game.units.iter_mut().find(|u| u.id == id) { u.burn_remaining = 4.0; u.burn_dps = atk; u.burn_tick = 0.5; } }
    for (id, atk) in poison_events { if let Some(u) = game.units.iter_mut().find(|u| u.id == id) { u.poison_remaining = 4.0; u.poison_dps = atk * 0.25 / 4.0; } }
    for (owner, damage) in castle_events { game.players[owner].castle_hp = (game.players[owner].castle_hp - damage).max(0.0); }
    game.units.extend(new_units);
    game.units.retain(|u| u.hp > 0.0);
    if game.players[0].castle_hp <= 0.0 { game.winner = Some(1); }
    else if game.players[1].castle_hp <= 0.0 { game.winner = Some(0); }
}

fn point_segment_distance(px:f32,pz:f32,ax:f32,az:f32,bx:f32,bz:f32)->f32 {
    let dx=bx-ax; let dz=bz-az; let denom=dx*dx+dz*dz;
    if denom < 0.0001 { return dist(px,pz,ax,az); }
    let t=(((px-ax)*dx+(pz-az)*dz)/denom).clamp(0.0,1.0);
    dist(px,pz,ax+t*dx,az+t*dz)
}
