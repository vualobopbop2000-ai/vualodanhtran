use axum::{
    extract::{ws::{Message, WebSocket, WebSocketUpgrade}, Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use futures_util::{SinkExt, StreamExt};
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{collections::HashMap, net::SocketAddr, sync::Arc, time::Duration};
use tokio::{sync::{mpsc, Mutex, RwLock}, time};
use tower_http::{cors::CorsLayer, services::ServeDir};
use tracing::{error, info};
use uuid::Uuid;

const WORLD_MAX: f32 = 1000.0;
const RIVER_MIN: f32 = 475.0;
const RIVER_MAX: f32 = 525.0;
const MAX_LEVEL: u8 = 5;

type RoomMap = Arc<RwLock<HashMap<String, Arc<Mutex<Room>>>>>;

#[derive(Clone)]
struct AppState { rooms: RoomMap }

#[derive(Clone)]
struct Room {
    id: String,
    name: String,
    password: Option<String>,
    players: Vec<Player>,
    game: GameState,
}

#[derive(Clone)]
struct Player {
    id: String,
    name: String,
    token: String,
    color: String,
    side: usize,
    gold: f32,
    command: String,
    connected: bool,
    tx: Option<mpsc::UnboundedSender<String>>,
    buff_until: f32,
    buff_kind: Option<String>,
    buff_name: Option<String>,
    buff_cooldown_until: f32,
}

#[derive(Clone, Serialize)]
struct Unit {
    id: u64,
    owner: usize,
    kind: String,
    x: f32,
    y: f32,
    hp: f32,
    max_hp: f32,
    attack: f32,
    defense: f32,
    speed: f32,
    attack_speed: f32,
    range: f32,
    level: u8,
    #[serde(skip_serializing)]
    attack_timer: f32,
    #[serde(skip_serializing)]
    summon_timer: f32,
    #[serde(skip_serializing)]
    dots: Vec<DotEffect>,
}

#[derive(Clone)]
struct DotEffect { remaining: f32, damage_per_second: f32 }

#[derive(Clone, Serialize)]
struct Building {
    id: u64,
    owner: usize,
    kind: String,
    x: f32,
    y: f32,
    hp: f32,
    max_hp: f32,
    level: u8,
}

#[derive(Clone, Serialize)]
struct CapturePoint { id: usize, x: f32, y: f32, owner: Option<usize> }

#[derive(Clone)]
struct GameState {
    started: bool,
    time: f32,
    winner: Option<usize>,
    units: Vec<Unit>,
    buildings: Vec<Building>,
    points: Vec<CapturePoint>,
    next_id: u64,
}

#[derive(Serialize)]
struct PublicPlayer {
    id: String,
    name: String,
    color: String,
    side: usize,
    gold: u32,
    command: String,
    connected: bool,
    buff: Option<String>,
    buff_cooldown: f32,
}

#[derive(Serialize)]
struct Snapshot {
    room_id: String,
    room_name: String,
    started: bool,
    time: f32,
    winner: Option<usize>,
    players: Vec<PublicPlayer>,
    units: Vec<Unit>,
    buildings: Vec<Building>,
    points: Vec<CapturePoint>,
    you_id: String,
}

#[derive(Serialize)]
struct RoomListItem {
    id: String,
    name: String,
    players: usize,
    has_password: bool,
    host: String,
}

#[derive(Deserialize)]
struct CreateRoomRequest { name: String, #[serde(default)] password: String, host_name: String }
#[derive(Deserialize)]
struct CheckRoomNameRequest { name: String }
#[derive(Deserialize)]
struct JoinRoomRequest { name: String, #[serde(default)] password: String }
#[derive(Deserialize)]
struct LeaveRoomRequest { player_id: String, token: String }
#[derive(Deserialize)]
struct WsQuery { player_id: String, token: String }

#[derive(Serialize)]
struct EnterResponse { room_id: String, player_id: String, token: String, color: String, side: usize }

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMessage {
    Deploy { unit: String, x: f32, y: f32, #[serde(default)] level: u8 },
    BuildMine { x: f32, y: f32, #[serde(default)] level: u8 },
    Command { mode: String },
    RollBuff,
    UpgradeMine,
    Ping,
}

#[derive(Clone, Copy)]
struct UnitConfig {
    cost: u32, hp: f32, attack: f32, defense: f32, speed: f32,
    attack_speed: f32, range: f32,
}

fn config(kind: &str) -> Option<UnitConfig> {
    let c = match kind {
        "vanguard" => UnitConfig { cost: 5, hp: 54.0, attack: 4.0, defense: 4.0, speed: 3.0, attack_speed: 5.0, range: 13.0 },
        "archer" => UnitConfig { cost: 7, hp: 34.0, attack: 8.0, defense: 2.0, speed: 2.0, attack_speed: 7.0, range: 105.0 },
        "shield" => UnitConfig { cost: 7, hp: 112.0, attack: 2.0, defense: 12.0, speed: 2.0, attack_speed: 3.0, range: 14.0 },
        "mage" => UnitConfig { cost: 11, hp: 42.0, attack: 7.0, defense: 2.0, speed: 2.0, attack_speed: 5.0, range: 66.0 },
        "cavalry" => UnitConfig { cost: 10, hp: 70.0, attack: 8.0, defense: 5.0, speed: 5.0, attack_speed: 5.0, range: 16.0 },
        "elephant" => UnitConfig { cost: 12, hp: 98.0, attack: 9.0, defense: 7.0, speed: 5.0, attack_speed: 6.0, range: 23.0 },
        "catapult" => UnitConfig { cost: 14, hp: 60.0, attack: 15.0, defense: 5.0, speed: 1.0, attack_speed: 2.0, range: 145.0 },
        "general" => UnitConfig { cost: 15, hp: 205.0, attack: 2.0, defense: 6.0, speed: 0.0, attack_speed: 2.0, range: 25.0 },
        "royal" => UnitConfig { cost: 0, hp: 69.0, attack: 5.5, defense: 5.5, speed: 4.5, attack_speed: 6.5, range: 13.0 },
        _ => return None,
    };
    Some(c)
}

fn new_game() -> GameState {
    GameState {
        started: false, time: 0.0, winner: None, units: Vec::new(), next_id: 3,
        buildings: vec![
            Building { id: 1, owner: 0, kind: "castle".into(), x: 500.0, y: 105.0, hp: 2400.0, max_hp: 2400.0, level: 1 },
            Building { id: 2, owner: 1, kind: "castle".into(), x: 500.0, y: 895.0, hp: 2400.0, max_hp: 2400.0, level: 1 },
        ],
        points: vec![
            CapturePoint { id: 1, x: 355.0, y: 500.0, owner: None },
            CapturePoint { id: 2, x: 645.0, y: 500.0, owner: None },
        ],
    }
}

fn clean_name(input: &str, min: usize, max: usize) -> Result<String, String> {
    let name = input.trim();
    if name.chars().count() < min || name.chars().count() > max {
        return Err(format!("Tên phải dài từ {min} đến {max} ký tự."));
    }
    if name.chars().any(|c| c.is_control()) {
        return Err("Tên chứa ký tự không hợp lệ.".into());
    }
    Ok(name.to_string())
}

fn snapshot(room: &Room, you_id: &str) -> Snapshot {
    Snapshot {
        room_id: room.id.clone(), room_name: room.name.clone(), started: room.game.started,
        time: room.game.time, winner: room.game.winner,
        players: room.players.iter().map(|p| PublicPlayer {
            id: p.id.clone(), name: p.name.clone(), color: p.color.clone(), side: p.side,
            gold: p.gold.floor().max(0.0) as u32, command: p.command.clone(), connected: p.connected,
            buff: if room.game.time < p.buff_until { p.buff_name.clone() } else { None },
            buff_cooldown: (p.buff_cooldown_until - room.game.time).max(0.0),
        }).collect(),
        units: room.game.units.clone(), buildings: room.game.buildings.clone(),
        points: room.game.points.clone(), you_id: you_id.to_string(),
    }
}

fn error_response(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

async fn health() -> &'static str { "ok" }

async fn list_rooms(State(state): State<AppState>) -> Json<Vec<RoomListItem>> {
    let map = state.rooms.read().await;
    let mut rooms = Vec::new();
    for room_arc in map.values() {
        let room = room_arc.lock().await;
        if room.players.len() < 2 {
            rooms.push(RoomListItem {
                id: room.id.clone(), name: room.name.clone(), players: room.players.len(),
                has_password: room.password.is_some(),
                host: room.players.first().map(|p| p.name.clone()).unwrap_or_else(|| "Khách".into()),
            });
        }
    }
    rooms.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Json(rooms)
}

async fn check_room_name(State(state): State<AppState>, Json(req): Json<CheckRoomNameRequest>) -> Response {
    let room_name = match clean_name(&req.name, 3, 28) { Ok(v) => v, Err(e) => return error_response(StatusCode::BAD_REQUEST, e) };
    let rooms = state.rooms.read().await;
    for room_arc in rooms.values() {
        let room = room_arc.lock().await;
        if room.name.eq_ignore_ascii_case(&room_name) {
            return error_response(StatusCode::CONFLICT, "Tên phòng bị trùng, đặt tên khác.");
        }
    }
    Json(json!({"available": true})).into_response()
}

async fn create_room(State(state): State<AppState>, Json(req): Json<CreateRoomRequest>) -> Response {
    let room_name = match clean_name(&req.name, 3, 28) { Ok(v) => v, Err(e) => return error_response(StatusCode::BAD_REQUEST, e) };
    let host_name = match clean_name(&req.host_name, 2, 18) { Ok(v) => v, Err(e) => return error_response(StatusCode::BAD_REQUEST, e) };
    if req.password.chars().count() > 40 { return error_response(StatusCode::BAD_REQUEST, "Mật khẩu phòng tối đa 40 ký tự."); }
    let mut rooms = state.rooms.write().await;
    if rooms.values().any(|r| r.try_lock().map(|g| g.name.eq_ignore_ascii_case(&room_name)).unwrap_or(false)) {
        return error_response(StatusCode::CONFLICT, "Tên phòng bị trùng, đặt tên khác.");
    }
    // If a room happens to be busy during the duplicate scan, check it without holding the map's write lock twice.
    for existing in rooms.values() {
        let r = existing.lock().await;
        if r.name.eq_ignore_ascii_case(&room_name) { return error_response(StatusCode::CONFLICT, "Tên phòng bị trùng, đặt tên khác."); }
    }
    let mut rng = rand::thread_rng();
    let first_color = if rng.gen_bool(0.5) { "blue" } else { "red" };
    let player_id = Uuid::new_v4().to_string();
    let token = Uuid::new_v4().to_string();
    let room_id = Uuid::new_v4().simple().to_string()[..10].to_string();
    let host = Player {
        id: player_id.clone(), name: host_name, token: token.clone(), color: first_color.into(), side: 0,
        gold: 20.0, command: "attack".into(), connected: false, tx: None,
        buff_until: 0.0, buff_kind: None, buff_name: None, buff_cooldown_until: 0.0,
    };
    let room = Arc::new(Mutex::new(Room {
        id: room_id.clone(), name: room_name, password: if req.password.trim().is_empty() { None } else { Some(req.password) },
        players: vec![host], game: new_game(),
    }));
    rooms.insert(room_id.clone(), room.clone());
    drop(rooms);
    spawn_room_loop(room);
    Json(EnterResponse { room_id, player_id, token, color: first_color.into(), side: 0 }).into_response()
}

async fn leave_room(Path(room_id): Path<String>, State(state): State<AppState>, Json(req): Json<LeaveRoomRequest>) -> Response {
    let room_arc = {
        let rooms = state.rooms.read().await;
        match rooms.get(&room_id) { Some(r) => r.clone(), None => return error_response(StatusCode::NOT_FOUND, "Phòng không tồn tại.") }
    };
    let mut room = room_arc.lock().await;
    let index = match room.players.iter().position(|p| p.id == req.player_id && p.token == req.token) {
        Some(i) => i, None => return error_response(StatusCode::UNAUTHORIZED, "Phiên khách không hợp lệ.")
    };
    room.players.remove(index);
    if room.players.is_empty() {
        drop(room);
        let mut rooms = state.rooms.write().await;
        rooms.remove(&room_id);
        return Json(json!({"ok": true})).into_response();
    }
    // A room that loses a player returns to a clean waiting state, preventing a disconnected guest
    // from occupying the second seat or leaving an abandoned army in the next match.
    reset_waiting_room(&mut room);
    drop(room);
    Json(json!({"ok": true})).into_response()
}

async fn join_room(Path(room_id): Path<String>, State(state): State<AppState>, Json(req): Json<JoinRoomRequest>) -> Response {
    let name = match clean_name(&req.name, 2, 18) { Ok(v) => v, Err(e) => return error_response(StatusCode::BAD_REQUEST, e) };
    let room_arc = {
        let rooms = state.rooms.read().await;
        match rooms.get(&room_id) { Some(r) => r.clone(), None => return error_response(StatusCode::NOT_FOUND, "Phòng không tồn tại hoặc đã đóng.") }
    };
    let mut room = room_arc.lock().await;
    if room.players.len() >= 2 { return error_response(StatusCode::CONFLICT, "Phòng đã đủ 2 người chơi."); }
    if room.password.as_deref().unwrap_or("") != req.password { return error_response(StatusCode::UNAUTHORIZED, "Mật khẩu phòng không đúng."); }
    let side = room.players.len();
    let color = if room.players.first().map(|p| p.color.as_str()) == Some("blue") { "red" } else { "blue" };
    let player_id = Uuid::new_v4().to_string();
    let token = Uuid::new_v4().to_string();
    room.players.push(Player {
        id: player_id.clone(), name, token: token.clone(), color: color.into(), side, gold: 20.0,
        command: "attack".into(), connected: false, tx: None,
        buff_until: 0.0, buff_kind: None, buff_name: None, buff_cooldown_until: 0.0,
    });
    room.game.started = true;
    Json(EnterResponse { room_id, player_id, token, color: color.into(), side }).into_response()
}

fn spawn_room_loop(room_arc: Arc<Mutex<Room>>) {
    tokio::spawn(async move {
        let mut interval = time::interval(Duration::from_millis(100));
        loop {
            interval.tick().await;
            let mut room = room_arc.lock().await;
            if room.players.is_empty() { break; }
            if room.players.len() == 2 && room.game.started && room.game.winner.is_none() {
                tick_game(&mut room, 0.1);
            }
            for player in &room.players {
                if let Some(tx) = &player.tx {
                    let state = snapshot(&room, &player.id);
                    if let Ok(encoded) = serde_json::to_string(&json!({"type":"state", "state":state})) {
                        let _ = tx.send(encoded);
                    }
                }
            }
        }
    });
}

fn is_own_half(side: usize, y: f32, kind: &str) -> bool {
    if !(15.0..=WORLD_MAX - 15.0).contains(&y) { return false; }
    if kind == "castle" { return true; }
    if side == 0 { y < RIVER_MIN - 3.0 } else { y > RIVER_MAX + 3.0 }
}

async fn apply_message(room_arc: &Arc<Mutex<Room>>, player_id: &str, msg: ClientMessage) {
    let mut room = room_arc.lock().await;
    let player_index = match room.players.iter().position(|p| p.id == player_id) { Some(i) => i, None => return };
    let owner = room.players[player_index].side;
    match msg {
        ClientMessage::Deploy { unit, x, y, level } => {
            if room.game.winner.is_some() || room.players.len() < 2 { return; }
            let level = level.clamp(1, MAX_LEVEL);
            if !is_own_half(owner, y, &unit) { return; }
            let cfg = match config(&unit) { Some(c) if unit != "royal" => c, _ => return };
            let deployment_cost = (cfg.cost as f32 * (1.0 + 0.18 * (level.saturating_sub(1) as f32))).ceil();
            if room.players[player_index].gold < deployment_cost { return; }
            room.players[player_index].gold -= deployment_cost;
            let multiplier = 1.0 + 0.13 * (level.saturating_sub(1) as f32);
            let id = room.game.next_id; room.game.next_id += 1;
            room.game.units.push(Unit {
                id, owner, kind: unit, x: x.clamp(15.0, 985.0), y: y.clamp(15.0, 985.0),
                hp: cfg.hp * (1.0 + 0.18 * (level.saturating_sub(1) as f32)),
                max_hp: cfg.hp * (1.0 + 0.18 * (level.saturating_sub(1) as f32)),
                attack: cfg.attack * multiplier, defense: cfg.defense * multiplier,
                speed: cfg.speed * multiplier, attack_speed: cfg.attack_speed * multiplier,
                range: cfg.range * (1.0 + 0.04 * (level.saturating_sub(1) as f32)), level,
                attack_timer: 0.2, summon_timer: 3.0, dots: Vec::new(),
            });
        }
        ClientMessage::BuildMine { x, y, level: _ } => {
            if room.game.winner.is_some() || room.players.len() < 2 || !is_own_half(owner, y, "mine") { return; }
            if room.game.buildings.iter().filter(|b| b.owner == owner && b.kind == "mine").count() >= 5 { return; }
            let level = 1_u8;
            let id = room.game.next_id; room.game.next_id += 1;
            let max_hp = 125.0 + (level.saturating_sub(1) as f32) * 25.0;
            room.game.buildings.push(Building { id, owner, kind: "mine".into(), x: x.clamp(20.0, 980.0), y: y.clamp(20.0, 980.0), hp: max_hp, max_hp, level });
        }
        ClientMessage::Command { mode } => {
            if ["attack", "retreat", "defend", "occupy"].contains(&mode.as_str()) { room.players[player_index].command = mode; }
        }
        ClientMessage::RollBuff => {
            if room.game.winner.is_some() || room.game.time < room.players[player_index].buff_cooldown_until { return; }
            let options = [("frenzy", "Điên cuồng"), ("agility", "Nhanh nhẹn"), ("speed", "Tốc đánh"), ("heal", "Hồi máu"), ("poison", "Độc dược")];
            let chosen = options[rand::thread_rng().gen_range(0..options.len())];
            let current_time = room.game.time;
            {
                let p = &mut room.players[player_index];
                p.buff_kind = Some(chosen.0.into()); p.buff_name = Some(chosen.1.into());
                p.buff_until = current_time + if chosen.0 == "heal" { 2.5 } else { 9.0 };
                p.buff_cooldown_until = current_time + 15.0;
            }
            if chosen.0 == "heal" {
                for unit in room.game.units.iter_mut().filter(|u| u.owner == owner) { unit.hp = (unit.hp + unit.max_hp * 0.25).min(unit.max_hp); }
            }
        }
        ClientMessage::UpgradeMine => {
            let target = room.game.buildings.iter().enumerate()
                .filter(|(_, b)| b.owner == owner && b.kind == "mine" && b.level < MAX_LEVEL)
                .min_by_key(|(_, b)| b.level).map(|(i, b)| (i, b.level));
            if let Some((idx, current_level)) = target {
                let cost = 8.0 * current_level as f32;
                if room.players[player_index].gold >= cost {
                    room.players[player_index].gold -= cost;
                    let b = &mut room.game.buildings[idx];
                    b.level += 1; b.max_hp += 25.0; b.hp = (b.hp + 25.0).min(b.max_hp);
                }
            }
        }
        ClientMessage::Ping => {}
    }
}

async fn websocket_handler(
    ws: WebSocketUpgrade, Path(room_id): Path<String>, Query(query): Query<WsQuery>, State(state): State<AppState>,
) -> Response {
    let room_arc = {
        let rooms = state.rooms.read().await;
        match rooms.get(&room_id) { Some(r) => r.clone(), None => return error_response(StatusCode::NOT_FOUND, "Phòng không tồn tại.") }
    };
    {
        let room = room_arc.lock().await;
        if !room.players.iter().any(|p| p.id == query.player_id && p.token == query.token) {
            return error_response(StatusCode::UNAUTHORIZED, "Phiên khách không hợp lệ, hãy vào phòng lại.");
        }
    }
    let rooms = state.rooms.clone();
    ws.on_upgrade(move |socket| socket_session(socket, room_arc, rooms, room_id, query.player_id))
}

async fn socket_session(socket: WebSocket, room_arc: Arc<Mutex<Room>>, rooms: RoomMap, room_id: String, player_id: String) {
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    {
        let mut room = room_arc.lock().await;
        if let Some(p) = room.players.iter_mut().find(|p| p.id == player_id) { p.connected = true; p.tx = Some(tx); }
    }
    let (mut sender, mut receiver) = socket.split();
    loop {
        tokio::select! {
            outgoing = rx.recv() => {
                match outgoing {
                    Some(text) => if sender.send(Message::Text(text)).await.is_err() { break; },
                    None => break,
                }
            }
            incoming = receiver.next() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(msg) = serde_json::from_str::<ClientMessage>(&text) { apply_message(&room_arc, &player_id, msg).await; }
                    }
                    Some(Ok(Message::Ping(data))) => { if sender.send(Message::Pong(data)).await.is_err() { break; } }
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    _ => {}
                }
            }
        }
    }
    let mut room = room_arc.lock().await;
    let old_len = room.players.len();
    room.players.retain(|p| p.id != player_id);
    if room.players.len() < old_len && room.players.len() == 1 {
        reset_waiting_room(&mut room);
    }
    let became_empty = room.players.is_empty();
    drop(room);
    if became_empty {
        let mut map = rooms.write().await;
        map.remove(&room_id);
    }
}

fn reset_waiting_room(room: &mut Room) {
    if let Some(player) = room.players.first_mut() {
        player.side = 0;
        player.gold = 20.0;
        player.command = "attack".into();
        player.buff_until = 0.0;
        player.buff_kind = None;
        player.buff_name = None;
        player.buff_cooldown_until = 0.0;
    }
    room.game = new_game();
    room.game.started = false;
}

#[derive(Clone, Copy)]
enum TargetKind { Unit(u64), Building(u64) }
#[derive(Clone)]
struct Target { kind: TargetKind, x: f32, y: f32, hp: f32 }
#[derive(Clone)]
struct AttackIntent { attacker: u64, target: TargetKind, damage: f32, aoe: f32, dash: bool, poison: bool }

fn dist(ax: f32, ay: f32, bx: f32, by: f32) -> f32 { ((ax-bx).powi(2) + (ay-by).powi(2)).sqrt() }
fn damage_after_armor(attack: f32, defense: f32) -> f32 { (attack - defense * 0.35).max(attack * 0.25).max(0.5) }
fn home(owner: usize) -> (f32, f32) { if owner == 0 { (500.0, 125.0) } else { (500.0, 875.0) } }
fn buff_modifier(room: &Room, owner: usize, kind: &str) -> f32 {
    let p = match room.players.get(owner) { Some(p) => p, None => return 0.0 };
    if room.game.time >= p.buff_until { return 0.0; }
    match (p.buff_kind.as_deref(), kind) {
        (Some("frenzy"), _) => 2.0,
        (Some("agility"), "speed") => 3.0,
        (Some("speed"), "attack_speed") => 3.0,
        _ => 0.0,
    }
}

fn tick_game(room: &mut Room, dt: f32) {
    room.game.time += dt;
    // Mines produce one gold per second per level; held points add a small income bonus.
    for owner in 0..2 {
        let mine_income: f32 = room.game.buildings.iter().filter(|b| b.owner == owner && b.kind == "mine").map(|b| b.level as f32).sum();
        let point_income = room.game.points.iter().filter(|p| p.owner == Some(owner)).count() as f32 * 0.75;
        room.players[owner].gold = (room.players[owner].gold + (mine_income + point_income) * dt).min(9999.0);
    }
    // Damage-over-time ticks.
    for unit in &mut room.game.units {
        let mut damage = 0.0;
        for dot in &mut unit.dots { let applied = dot.damage_per_second * dt; damage += applied; dot.remaining -= dt; }
        unit.hp -= damage;
        unit.dots.retain(|d| d.remaining > 0.0);
        unit.attack_timer = (unit.attack_timer - dt).max(0.0);
        unit.summon_timer -= dt;
    }
    room.game.units.retain(|u| u.hp > 0.0);
    room.game.buildings.retain(|b| b.hp > 0.0);
    if room.game.winner.is_none() {
        if !room.game.buildings.iter().any(|b| b.kind == "castle" && b.owner == 0) { room.game.winner = Some(1); }
        if !room.game.buildings.iter().any(|b| b.kind == "castle" && b.owner == 1) { room.game.winner = Some(0); }
    }
    if room.game.winner.is_some() { return; }

    // Control points change hands when only one team is present nearby.
    for point_index in 0..room.game.points.len() {
        let (px, py) = (room.game.points[point_index].x, room.game.points[point_index].y);
        let blue = room.game.units.iter().any(|u| u.owner == 0 && dist(u.x,u.y,px,py) < 37.0);
        let red = room.game.units.iter().any(|u| u.owner == 1 && dist(u.x,u.y,px,py) < 37.0);
        if blue && !red { room.game.points[point_index].owner = Some(0); }
        if red && !blue { room.game.points[point_index].owner = Some(1); }
    }

    let units_snapshot = room.game.units.clone();
    let buildings_snapshot = room.game.buildings.clone();
    let commands: Vec<String> = room.players.iter().map(|p| p.command.clone()).collect();
    let mut moves: Vec<(u64, f32, f32)> = Vec::new();
    let mut attacks: Vec<AttackIntent> = Vec::new();
    let mut summons: Vec<Unit> = Vec::new();

    for unit in &units_snapshot {
        let owner = unit.owner;
        let mode = commands.get(owner).map(String::as_str).unwrap_or("attack");
        let is_general = unit.kind == "general";
        let my_attack = unit.attack + buff_modifier(room, owner, "attack");
        let my_speed = (unit.speed + buff_modifier(room, owner, "speed")).max(0.0);
        let my_attack_speed = (unit.attack_speed + buff_modifier(room, owner, "attack_speed")).max(0.5);
        if is_general {
            let summon_ready = room.game.units.iter().find(|u| u.id == unit.id).map(|u| u.summon_timer <= 0.0).unwrap_or(false);
            if summon_ready {
                    let count = 2 + unit.level.saturating_sub(1) as usize;
                    for i in 0..count {
                        let cfg = config("royal").unwrap();
                        let id = room.game.next_id + summons.len() as u64;
                        let offset = (i as f32 - count as f32 / 2.0) * 2.2;
                        summons.push(Unit { id, owner, kind: "royal".into(), x: (unit.x + offset).clamp(10.0,990.0), y: (unit.y + if owner==0 { 5.0 } else {-5.0}).clamp(10.0,990.0), hp: cfg.hp * (1.0+0.18*(unit.level.saturating_sub(1) as f32)), max_hp: cfg.hp*(1.0+0.18*(unit.level.saturating_sub(1) as f32)), attack: cfg.attack*(1.0+0.13*(unit.level.saturating_sub(1) as f32)), defense: cfg.defense*(1.0+0.13*(unit.level.saturating_sub(1) as f32)), speed: cfg.speed*(1.0+0.13*(unit.level.saturating_sub(1) as f32)), attack_speed: cfg.attack_speed*(1.0+0.13*(unit.level.saturating_sub(1) as f32)), range: cfg.range, level: unit.level, attack_timer:0.25, summon_timer:3.0, dots:Vec::new() });
                    }
                    if let Some(current_mut) = room.game.units.iter_mut().find(|u| u.id == unit.id) { current_mut.summon_timer = 3.0; }
            }
        }

        let mut targets: Vec<Target> = Vec::new();
        for enemy in units_snapshot.iter().filter(|u| u.owner != owner && u.hp > 0.0) {
            targets.push(Target { kind: TargetKind::Unit(enemy.id), x: enemy.x, y: enemy.y, hp: enemy.hp });
        }
        for b in buildings_snapshot.iter().filter(|b| b.owner != owner && b.hp > 0.0) {
            targets.push(Target { kind: TargetKind::Building(b.id), x: b.x, y: b.y, hp: b.hp });
        }
        if targets.is_empty() { continue; }

        let target = if mode == "occupy" {
            let enemy_near = targets.iter().filter(|t| matches!(t.kind, TargetKind::Unit(_))).min_by(|a,b| dist(unit.x,unit.y,a.x,a.y).total_cmp(&dist(unit.x,unit.y,b.x,b.y)));
            if let Some(t) = enemy_near.filter(|t| dist(unit.x,unit.y,t.x,t.y) < 75.0) { Some((*t).clone()) }
            else {
                room.game.points.iter().filter(|p| p.owner != Some(owner)).min_by(|a,b| dist(unit.x,unit.y,a.x,a.y).total_cmp(&dist(unit.x,unit.y,b.x,b.y))).map(|p| Target { kind: TargetKind::Building(u64::MAX - p.id as u64), x:p.x, y:p.y, hp:1.0 })
            }
        } else if mode == "defend" || mode == "retreat" {
            let nearest = targets.iter().filter(|t| matches!(t.kind, TargetKind::Unit(_))).min_by(|a,b| dist(unit.x,unit.y,a.x,a.y).total_cmp(&dist(unit.x,unit.y,b.x,b.y)));
            if let Some(t) = nearest.filter(|t| dist(unit.x,unit.y,t.x,t.y) < if mode=="retreat" { 55.0 } else { 115.0 }) { Some((*t).clone()) }
            else { None }
        } else {
            targets.iter().min_by(|a,b| {
                let da = dist(unit.x,unit.y,a.x,a.y) + if matches!(a.kind, TargetKind::Building(_)) { 18.0 } else { 0.0 };
                let db = dist(unit.x,unit.y,b.x,b.y) + if matches!(b.kind, TargetKind::Building(_)) { 18.0 } else { 0.0 };
                da.total_cmp(&db)
            }).cloned()
        };

        if mode == "defend" || mode == "retreat" {
            if target.is_none() {
                let (hx,hy)=home(owner);
                let d=dist(unit.x,unit.y,hx,hy);
                if d > 38.0 && my_speed > 0.0 { let step=(my_speed*dt*1.9).min(d); moves.push((unit.id, unit.x+(hx-unit.x)/d*step, unit.y+(hy-unit.y)/d*step)); }
                continue;
            }
        }
        let target = match target { Some(t) => t, None => continue };
        let distance = dist(unit.x, unit.y, target.x, target.y);
        let attack_range = unit.range.max(8.0);
        if distance <= attack_range {
            let current_timer = room.game.units.iter().find(|u| u.id == unit.id).map(|u| u.attack_timer).unwrap_or(1.0);
            if current_timer <= 0.0 && target.hp > 0.0 {
                let poison = room.players.get(owner).map(|p| p.buff_kind.as_deref()==Some("poison") && room.game.time < p.buff_until).unwrap_or(false);
                let aoe = if unit.kind == "mage" { 20.0 } else if unit.kind == "catapult" { 26.0 } else { 0.0 };
                attacks.push(AttackIntent { attacker: unit.id, target: target.kind.clone(), damage: my_attack, aoe, dash: unit.kind == "cavalry", poison });
                if let Some(current) = room.game.units.iter_mut().find(|u| u.id == unit.id) { current.attack_timer = (5.0/my_attack_speed).max(0.45); }
            }
        } else if my_speed > 0.0 && mode != "occupy" || (mode=="occupy" && distance > 18.0) {
            let d=distance.max(0.01); let step=(my_speed*dt*1.8).min((d-attack_range.min(d)).max(0.0));
            if step>0.0 { moves.push((unit.id, unit.x+(target.x-unit.x)/d*step, unit.y+(target.y-unit.y)/d*step)); }
        }
    }

    for (id,x,y) in moves {
        if let Some(u)=room.game.units.iter_mut().find(|u|u.id==id) { u.x=x.clamp(10.0,990.0); u.y=y.clamp(10.0,990.0); }
    }
    for attack in attacks {
        let attacker_owner = room.game.units.iter().find(|u|u.id==attack.attacker).map(|u|u.owner).unwrap_or(0);
        let origin = room.game.units.iter().find(|u|u.id==attack.attacker).map(|u|(u.x,u.y));
        let (tx,ty) = match &attack.target {
            TargetKind::Unit(id) => room.game.units.iter().find(|u|u.id==*id).map(|u|(u.x,u.y)).unwrap_or((0.0,0.0)),
            TargetKind::Building(id) if *id < u64::MAX-10 => room.game.buildings.iter().find(|b|b.id==*id).map(|b|(b.x,b.y)).unwrap_or((0.0,0.0)),
            TargetKind::Building(id) => { let pid=(u64::MAX-*id) as usize; room.game.points.iter().find(|p|p.id==pid).map(|p|(p.x,p.y)).unwrap_or((0.0,0.0)) }
        };
        if attack.dash {
            if let Some((ox,oy)) = origin {
                let d = dist(ox,oy,tx,ty).max(0.01);
                let travel = 20.0_f32.min(d);
                if let Some(attacker) = room.game.units.iter_mut().find(|u|u.id==attack.attacker) {
                    attacker.x = (ox + (tx-ox)/d*travel).clamp(10.0,990.0);
                    attacker.y = (oy + (ty-oy)/d*travel).clamp(10.0,990.0);
                }
            }
        }
        if let TargetKind::Building(id) = attack.target {
            if id >= u64::MAX-10 { continue; } // capture objectives are not damageable
        }
        let targets: Vec<(bool,u64)> = match attack.target {
            TargetKind::Unit(id) => {
                let mut out=vec![(true,id)];
                if attack.aoe>0.0 { for u in room.game.units.iter().filter(|u|u.owner!=attacker_owner && u.id!=id && dist(u.x,u.y,tx,ty)<=attack.aoe) { out.push((true,u.id)); } }
                if attack.dash { if let Some((ox,oy))=origin { for u in room.game.units.iter().filter(|u|u.owner!=attacker_owner && u.id!=id && point_line_distance(u.x,u.y,ox,oy,tx,ty)<7.0) { out.push((true,u.id)); } } }
                out
            }
            TargetKind::Building(id) => {
                let mut out=vec![(false,id)];
                if attack.aoe>0.0 { for b in room.game.buildings.iter().filter(|b|b.owner!=attacker_owner && b.id!=id && dist(b.x,b.y,tx,ty)<=attack.aoe) { out.push((false,b.id)); } }
                out
            }
        };
        let attacker_is_mage = units_snapshot.iter().any(|u| u.id == attack.attacker && u.kind == "mage");
        let defense_bonuses = [buff_modifier(room, 0, "defense"), buff_modifier(room, 1, "defense")];
        for (is_unit,id) in targets {
            if is_unit {
                if let Some(target)=room.game.units.iter_mut().find(|u|u.id==id && u.owner!=attacker_owner) {
                    let defense = target.defense + defense_bonuses.get(target.owner).copied().unwrap_or(0.0);
                    let damage=damage_after_armor(attack.damage,defense); target.hp-=damage;
                    if attack.dash { target.hp-=attack.damage*0.25; }
                    if attack.aoe>0.0 && attack.damage>0.0 && attacker_is_mage {
                        target.dots.push(DotEffect{remaining:4.0,damage_per_second:attack.damage});
                    }
                    if attack.poison { target.dots.push(DotEffect{remaining:2.0,damage_per_second:attack.damage*0.125}); }
                }
            } else if let Some(target)=room.game.buildings.iter_mut().find(|b|b.id==id && b.owner!=attacker_owner) {
                let damage=damage_after_armor(attack.damage, if target.kind=="castle" {8.0}else{1.0}); target.hp-=damage;
            }
        }
    }
    if !summons.is_empty() {
        room.game.next_id += summons.len() as u64;
        room.game.units.extend(summons);
    }
    // Cull dead objects now so destroyed castles end the match without a long visual delay.
    room.game.units.retain(|u|u.hp>0.0);
    room.game.buildings.retain(|b|b.hp>0.0);
    if !room.game.buildings.iter().any(|b|b.kind=="castle"&&b.owner==0) { room.game.winner=Some(1); }
    if !room.game.buildings.iter().any(|b|b.kind=="castle"&&b.owner==1) { room.game.winner=Some(0); }
}

fn point_line_distance(px:f32,py:f32,x1:f32,y1:f32,x2:f32,y2:f32)->f32 {
    let dx=x2-x1; let dy=y2-y1; let denom=dx*dx+dy*dy;
    if denom<0.001 { return dist(px,py,x1,y1); }
    let t=(((px-x1)*dx+(py-y1)*dy)/denom).clamp(0.0,1.0);
    dist(px,py,x1+t*dx,y1+t*dy)
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into())).init();
    let state=AppState{rooms:Arc::new(RwLock::new(HashMap::new()))};
    let app=Router::new()
        .route("/healthz",get(health))
        .route("/api/rooms",get(list_rooms).post(create_room))
        .route("/api/rooms/check-name",post(check_room_name))
        .route("/api/rooms/:room_id/join",post(join_room))
        .route("/api/rooms/:room_id/leave",post(leave_room))
        .route("/ws/:room_id",get(websocket_handler))
        .fallback_service(ServeDir::new("public").append_index_html_on_directories(true))
        .layer(CorsLayer::permissive())
        .with_state(state);
    let port:u16=std::env::var("PORT").ok().and_then(|s|s.parse().ok()).unwrap_or(10000);
    let addr=SocketAddr::from(([0,0,0,0],port));
    info!("Emperor's Great Battle listening on {addr}");
    let listener=tokio::net::TcpListener::bind(addr).await.expect("failed to bind listener");
    if let Err(e)=axum::serve(listener,app).await { error!("server error: {e}"); }
}
