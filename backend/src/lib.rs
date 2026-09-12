//! Authoritative local state for cosmetic six-max no-limit Texas Hold'em.
//!
//! The crate owns the SQLite state machine and exposes a state-baked Axum router.
//! It has no dependency on Core: Core's generic ext-proxy can mount the router at
//! `/api/token-table`, while the binary in `main.rs` supplies the loopback/auth shell.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{Display, Formatter};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;
use uuid::Uuid;

pub const MAX_SEATS: u8 = 6;
pub const DEFAULT_STACK: u64 = 1_000;
pub const DEFAULT_SMALL_BLIND: u64 = 5;
pub const DEFAULT_BIG_BLIND: u64 = 10;
const EVENT_TOPIC: &str = "token_table.snapshot";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct Card {
    pub rank: u8,
    pub suit: char,
}

impl Card {
    fn new(rank: u8, suit: char) -> Self {
        Self { rank, suit }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum TablePhase {
    Waiting,
    Preflop,
    Flop,
    Turn,
    River,
    Showdown,
    Complete,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlayerSnapshot {
    pub player_id: String,
    pub display_name: String,
    pub seat: Option<u8>,
    pub stack: u64,
    pub committed: u64,
    pub street_committed: u64,
    pub in_hand: bool,
    pub folded: bool,
    pub hole_cards: Vec<Card>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Winner {
    pub player_id: String,
    pub payout: u64,
    pub hand_rank: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActionRecord {
    pub action_id: String,
    pub player_id: String,
    pub action: PlayerAction,
    pub amount: Option<u64>,
    pub action_seq: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TableSnapshot {
    pub table_id: String,
    pub name: String,
    pub max_seats: u8,
    pub small_blind: u64,
    pub big_blind: u64,
    pub starting_stack: u64,
    pub phase: TablePhase,
    pub hand_number: u64,
    pub hand_id: Option<String>,
    pub action_seq: u64,
    pub button_seat: Option<u8>,
    pub small_blind_seat: Option<u8>,
    pub big_blind_seat: Option<u8>,
    pub current_turn: Option<String>,
    pub current_bet: u64,
    pub min_raise: u64,
    pub pot: u64,
    pub settled_pot: u64,
    pub board: Vec<Card>,
    pub players: Vec<PlayerSnapshot>,
    pub last_action: Option<ActionRecord>,
    pub winners: Vec<Winner>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RealtimeEvent {
    pub topic: String,
    pub table_id: String,
    pub sequence: u64,
    pub snapshot: TableSnapshot,
}

pub trait PublishHook: Send + Sync {
    fn publish(&self, event: RealtimeEvent);
}

#[derive(Clone)]
pub struct BroadcastPublisher {
    sender: broadcast::Sender<RealtimeEvent>,
}

impl Default for BroadcastPublisher {
    fn default() -> Self {
        Self::new(128)
    }
}

impl BroadcastPublisher {
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self { sender }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<RealtimeEvent> {
        self.sender.subscribe()
    }
}

impl PublishHook for BroadcastPublisher {
    fn publish(&self, event: RealtimeEvent) {
        let _ = self.sender.send(event);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TableState {
    snapshot: TableSnapshot,
    server_seed: u64,
    deck: Vec<Card>,
    pending: BTreeSet<String>,
    street_contributions: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateTableRequest {
    pub name: String,
    #[serde(default)]
    pub max_seats: Option<u8>,
    #[serde(default)]
    pub small_blind: Option<u64>,
    #[serde(default)]
    pub big_blind: Option<u64>,
    #[serde(default)]
    pub starting_stack: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct JoinTableRequest {
    pub player_id: String,
    pub display_name: String,
    #[serde(default)]
    pub stack: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SeatPlayerRequest {
    pub player_id: String,
    pub seat: u8,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LeaveTableRequest {
    pub player_id: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlayerAction {
    Fold,
    Check,
    Call,
    Bet,
    Raise,
    #[serde(alias = "all-in")]
    AllIn,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ActionRequest {
    pub action_id: String,
    pub player_id: String,
    pub expected_action_seq: u64,
    pub action: PlayerAction,
    /// For bet/raise, this is the total street commitment after the action.
    #[serde(default)]
    pub amount: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActionResult {
    pub action_id: String,
    pub accepted: bool,
    pub replayed: bool,
    pub snapshot: TableSnapshot,
}

#[derive(Debug)]
pub enum GameError {
    Invalid(String),
    NotFound(String),
    Conflict(String),
    StaleAction { expected: u64, actual: u64 },
    Storage(String),
}

impl Display for GameError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(message) => write!(f, "invalid request: {message}"),
            Self::NotFound(message) => write!(f, "not found: {message}"),
            Self::Conflict(message) => write!(f, "conflict: {message}"),
            Self::StaleAction { expected, actual } => {
                write!(
                    f,
                    "stale action: expected sequence {expected}, current sequence {actual}"
                )
            }
            Self::Storage(message) => write!(f, "storage error: {message}"),
        }
    }
}

impl std::error::Error for GameError {}

impl From<rusqlite::Error> for GameError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Storage(error.to_string())
    }
}

type Result<T> = std::result::Result<T, GameError>;

#[derive(Clone)]
pub struct TableStore {
    connection: Arc<Mutex<Connection>>,
    publisher: Arc<dyn PublishHook>,
    broadcast: Option<Arc<BroadcastPublisher>>,
}

impl TableStore {
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self> {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| GameError::Storage(error.to_string()))?;
        }
        let connection = Connection::open(path).map_err(GameError::from)?;
        Self::from_connection(connection)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory().map_err(GameError::from)?)
    }

    fn from_connection(connection: Connection) -> Result<Self> {
        connection.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS tables (
                 table_id TEXT PRIMARY KEY,
                 snapshot_json TEXT NOT NULL,
                 updated_at TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS action_idempotency (
                 table_id TEXT NOT NULL,
                 action_id TEXT NOT NULL,
                 response_json TEXT NOT NULL,
                 created_at TEXT NOT NULL,
                 PRIMARY KEY (table_id, action_id)
             );",
        )?;
        let publisher = Arc::new(BroadcastPublisher::default());
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
            publisher: publisher.clone(),
            broadcast: Some(publisher),
        })
    }

    pub fn with_publisher(mut self, publisher: Arc<dyn PublishHook>) -> Self {
        self.publisher = publisher;
        self.broadcast = None;
        self
    }

    pub fn subscribe(&self) -> Option<broadcast::Receiver<RealtimeEvent>> {
        self.broadcast
            .as_ref()
            .map(|publisher| publisher.subscribe())
    }

    pub fn create_table(&self, request: CreateTableRequest) -> Result<TableSnapshot> {
        let name = request.name.trim();
        if name.is_empty() {
            return Err(GameError::Invalid("name must not be blank".into()));
        }
        let max_seats = request.max_seats.unwrap_or(MAX_SEATS);
        let small_blind = request.small_blind.unwrap_or(DEFAULT_SMALL_BLIND);
        let big_blind = request.big_blind.unwrap_or(DEFAULT_BIG_BLIND);
        let starting_stack = request.starting_stack.unwrap_or(DEFAULT_STACK);
        if !(2..=MAX_SEATS).contains(&max_seats) {
            return Err(GameError::Invalid(
                "max_seats must be between 2 and 6".into(),
            ));
        }
        if small_blind == 0 || big_blind <= small_blind || starting_stack < big_blind {
            return Err(GameError::Invalid(
                "blinds must be positive and starting_stack must cover the big blind".into(),
            ));
        }
        let table_id = format!("table_{}", Uuid::new_v4().simple());
        let snapshot = TableSnapshot {
            table_id: table_id.clone(),
            name: name.to_string(),
            max_seats,
            small_blind,
            big_blind,
            starting_stack,
            phase: TablePhase::Waiting,
            hand_number: 0,
            hand_id: None,
            action_seq: 0,
            button_seat: None,
            small_blind_seat: None,
            big_blind_seat: None,
            current_turn: None,
            current_bet: 0,
            min_raise: big_blind,
            pot: 0,
            settled_pot: 0,
            board: Vec::new(),
            players: Vec::new(),
            last_action: None,
            winners: Vec::new(),
        };
        let state = TableState {
            snapshot: snapshot.clone(),
            server_seed: seed_for(&table_id),
            deck: Vec::new(),
            pending: BTreeSet::new(),
            street_contributions: BTreeMap::new(),
        };
        let connection = self.connection.lock().map_err(lock_error)?;
        insert_state(&connection, &state)?;
        drop(connection);
        self.publish(&snapshot);
        Ok(snapshot)
    }

    pub fn list_tables(&self) -> Result<Vec<TableSnapshot>> {
        let connection = self.connection.lock().map_err(lock_error)?;
        let mut statement = connection
            .prepare("SELECT snapshot_json FROM tables ORDER BY updated_at DESC, table_id ASC")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        let mut tables = Vec::new();
        for row in rows {
            let json = row?;
            tables.push(parse_state(&json)?.snapshot);
        }
        Ok(tables)
    }

    pub fn snapshot(&self, table_id: &str) -> Result<TableSnapshot> {
        let connection = self.connection.lock().map_err(lock_error)?;
        Ok(load_state(&connection, table_id)?.snapshot)
    }

    pub fn join_table(&self, table_id: &str, request: JoinTableRequest) -> Result<TableSnapshot> {
        let player_id = request.player_id.trim();
        let display_name = request.display_name.trim();
        if player_id.is_empty() || display_name.is_empty() {
            return Err(GameError::Invalid(
                "player_id and display_name are required".into(),
            ));
        }
        let connection = self.connection.lock().map_err(lock_error)?;
        let mut state = load_state(&connection, table_id)?;
        if state
            .snapshot
            .players
            .iter()
            .any(|p| p.player_id == player_id)
        {
            return Err(GameError::Conflict("player is already at the table".into()));
        }
        let stack = request.stack.unwrap_or(state.snapshot.starting_stack);
        if stack == 0 {
            return Err(GameError::Invalid("stack must be positive".into()));
        }
        state.snapshot.players.push(PlayerSnapshot {
            player_id: player_id.to_string(),
            display_name: display_name.to_string(),
            seat: None,
            stack,
            committed: 0,
            street_committed: 0,
            in_hand: false,
            folded: false,
            hole_cards: Vec::new(),
        });
        save_state(&connection, &state)?;
        let snapshot = state.snapshot;
        drop(connection);
        self.publish(&snapshot);
        Ok(snapshot)
    }

    pub fn seat_player(&self, table_id: &str, request: SeatPlayerRequest) -> Result<TableSnapshot> {
        let connection = self.connection.lock().map_err(lock_error)?;
        let mut state = load_state(&connection, table_id)?;
        if request.seat >= state.snapshot.max_seats {
            return Err(GameError::Invalid(
                "seat is outside the table capacity".into(),
            ));
        }
        if state.snapshot.phase > TablePhase::Waiting && state.snapshot.phase < TablePhase::Complete
        {
            return Err(GameError::Conflict(
                "seating is locked during a hand".into(),
            ));
        }
        if state
            .snapshot
            .players
            .iter()
            .any(|player| player.seat == Some(request.seat))
        {
            return Err(GameError::Conflict("seat is occupied".into()));
        }
        let player = state
            .snapshot
            .players
            .iter_mut()
            .find(|player| player.player_id == request.player_id)
            .ok_or_else(|| GameError::NotFound("player has not joined this table".into()))?;
        if player.seat.is_some() {
            return Err(GameError::Conflict("player is already seated".into()));
        }
        player.seat = Some(request.seat);
        if state.snapshot.phase == TablePhase::Complete {
            reset_to_waiting(&mut state);
        }
        let seated = seated_player_ids(&state.snapshot);
        if state.snapshot.phase == TablePhase::Waiting && seated.len() >= 2 {
            start_hand_state(&mut state)?;
        }
        save_state(&connection, &state)?;
        let snapshot = state.snapshot;
        drop(connection);
        self.publish(&snapshot);
        Ok(snapshot)
    }

    pub fn start_hand(&self, table_id: &str) -> Result<TableSnapshot> {
        let connection = self.connection.lock().map_err(lock_error)?;
        let mut state = load_state(&connection, table_id)?;
        start_hand_state(&mut state)?;
        save_state(&connection, &state)?;
        let snapshot = state.snapshot;
        drop(connection);
        self.publish(&snapshot);
        Ok(snapshot)
    }

    pub fn leave_table(&self, table_id: &str, request: LeaveTableRequest) -> Result<TableSnapshot> {
        let connection = self.connection.lock().map_err(lock_error)?;
        let mut state = load_state(&connection, table_id)?;
        if state.snapshot.phase > TablePhase::Waiting && state.snapshot.phase < TablePhase::Complete
        {
            return Err(GameError::Conflict(
                "players may leave only between hands".into(),
            ));
        }
        let before = state.snapshot.players.len();
        state
            .snapshot
            .players
            .retain(|player| player.player_id != request.player_id);
        if state.snapshot.players.len() == before {
            return Err(GameError::NotFound("player is not at this table".into()));
        }
        if state.snapshot.phase == TablePhase::Complete {
            reset_to_waiting(&mut state);
        }
        save_state(&connection, &state)?;
        let snapshot = state.snapshot;
        drop(connection);
        self.publish(&snapshot);
        Ok(snapshot)
    }

    pub fn act(&self, table_id: &str, request: ActionRequest) -> Result<ActionResult> {
        let connection = self.connection.lock().map_err(lock_error)?;
        let tx = connection.unchecked_transaction()?;
        if let Some(response_json) = tx
            .query_row(
                "SELECT response_json FROM action_idempotency WHERE table_id = ?1 AND action_id = ?2",
                params![table_id, request.action_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        {
            let mut response: ActionResult = serde_json::from_str(&response_json)
                .map_err(|error| GameError::Storage(error.to_string()))?;
            response.replayed = true;
            return Ok(response);
        }
        let state_json: String = tx
            .query_row(
                "SELECT snapshot_json FROM tables WHERE table_id = ?1",
                params![table_id],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| GameError::NotFound(format!("table {table_id}")))?;
        let mut state = parse_state(&state_json)?;
        if request.expected_action_seq != state.snapshot.action_seq {
            return Err(GameError::StaleAction {
                expected: request.expected_action_seq,
                actual: state.snapshot.action_seq,
            });
        }
        apply_action(&mut state, &request)?;
        state.snapshot.action_seq += 1;
        if let Some(last_action) = state.snapshot.last_action.as_mut() {
            last_action.action_seq = state.snapshot.action_seq;
        }
        let response = ActionResult {
            action_id: request.action_id.clone(),
            accepted: true,
            replayed: false,
            snapshot: state.snapshot.clone(),
        };
        save_state_tx(&tx, &state)?;
        let response_json = serde_json::to_string(&response)
            .map_err(|error| GameError::Storage(error.to_string()))?;
        tx.execute(
            "INSERT INTO action_idempotency (table_id, action_id, response_json, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![table_id, request.action_id, response_json, now_string()],
        )?;
        tx.commit()?;
        drop(connection);
        self.publish(&response.snapshot);
        Ok(response)
    }

    fn publish(&self, snapshot: &TableSnapshot) {
        self.publisher.publish(RealtimeEvent {
            topic: EVENT_TOPIC.to_string(),
            table_id: snapshot.table_id.clone(),
            sequence: snapshot.action_seq,
            snapshot: snapshot.clone(),
        });
    }
}

fn lock_error<T>(_: std::sync::PoisonError<T>) -> GameError {
    GameError::Storage("store lock poisoned".into())
}

fn now_string() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn seed_for(table_id: &str) -> u64 {
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or_default();
    table_id.bytes().fold(time, |seed, byte| {
        seed.rotate_left(7) ^ u64::from(byte).wrapping_mul(0x9e37_79b9)
    })
}

fn insert_state(connection: &Connection, state: &TableState) -> Result<()> {
    let json =
        serde_json::to_string(state).map_err(|error| GameError::Storage(error.to_string()))?;
    connection.execute(
        "INSERT INTO tables (table_id, snapshot_json, updated_at) VALUES (?1, ?2, ?3)",
        params![state.snapshot.table_id, json, now_string()],
    )?;
    Ok(())
}

fn save_state(connection: &Connection, state: &TableState) -> Result<()> {
    let json =
        serde_json::to_string(state).map_err(|error| GameError::Storage(error.to_string()))?;
    connection.execute(
        "UPDATE tables SET snapshot_json = ?2, updated_at = ?3 WHERE table_id = ?1",
        params![state.snapshot.table_id, json, now_string()],
    )?;
    Ok(())
}

fn save_state_tx(tx: &Transaction<'_>, state: &TableState) -> Result<()> {
    let json =
        serde_json::to_string(state).map_err(|error| GameError::Storage(error.to_string()))?;
    tx.execute(
        "UPDATE tables SET snapshot_json = ?2, updated_at = ?3 WHERE table_id = ?1",
        params![state.snapshot.table_id, json, now_string()],
    )?;
    Ok(())
}

fn load_state(connection: &Connection, table_id: &str) -> Result<TableState> {
    let json = connection
        .query_row(
            "SELECT snapshot_json FROM tables WHERE table_id = ?1",
            params![table_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .ok_or_else(|| GameError::NotFound(format!("table {table_id}")))?;
    parse_state(&json)
}

fn parse_state(json: &str) -> Result<TableState> {
    serde_json::from_str(json).map_err(|error| GameError::Storage(error.to_string()))
}

fn reset_to_waiting(state: &mut TableState) {
    state.snapshot.phase = TablePhase::Waiting;
    state.snapshot.hand_id = None;
    state.snapshot.current_turn = None;
    state.snapshot.current_bet = 0;
    state.snapshot.min_raise = state.snapshot.big_blind;
    state.snapshot.pot = 0;
    state.snapshot.settled_pot = 0;
    state.snapshot.board.clear();
    state.snapshot.winners.clear();
    state.snapshot.last_action = None;
    state.pending.clear();
    state.deck.clear();
    state.street_contributions.clear();
    for player in &mut state.snapshot.players {
        player.seat = player.seat.filter(|_| true);
        player.committed = 0;
        player.street_committed = 0;
        player.in_hand = false;
        player.folded = false;
        player.hole_cards.clear();
    }
}

fn seated_player_ids(snapshot: &TableSnapshot) -> Vec<String> {
    let mut seated: Vec<_> = snapshot
        .players
        .iter()
        .filter_map(|player| player.seat.map(|seat| (seat, player.player_id.clone())))
        .collect();
    seated.sort_by_key(|(seat, _)| *seat);
    seated.into_iter().map(|(_, player_id)| player_id).collect()
}

fn start_hand_state(state: &mut TableState) -> Result<()> {
    if !matches!(
        state.snapshot.phase,
        TablePhase::Waiting | TablePhase::Complete
    ) {
        return Err(GameError::Conflict("a hand is already in progress".into()));
    }
    let seated: Vec<(u8, String)> = state
        .snapshot
        .players
        .iter()
        .filter_map(|player| player.seat.map(|seat| (seat, player.player_id.clone())))
        .collect();
    if seated.len() < 2 {
        return Err(GameError::Conflict(
            "at least two seated players are required".into(),
        ));
    }
    state.snapshot.hand_number += 1;
    state.snapshot.hand_id = Some(format!(
        "{}-hand-{}",
        state.snapshot.table_id, state.snapshot.hand_number
    ));
    state.snapshot.phase = TablePhase::Preflop;
    state.snapshot.action_seq = 0;
    state.snapshot.current_bet = state.snapshot.big_blind;
    state.snapshot.min_raise = state.snapshot.big_blind;
    state.snapshot.pot = 0;
    state.snapshot.settled_pot = 0;
    state.snapshot.board.clear();
    state.snapshot.winners.clear();
    state.snapshot.last_action = None;
    state.pending.clear();
    state.street_contributions.clear();
    state.deck = deterministic_deck(state.server_seed ^ state.snapshot.hand_number);

    let mut seats: Vec<u8> = seated.iter().map(|(seat, _)| *seat).collect();
    seats.sort_unstable();
    let button = match state.snapshot.button_seat {
        Some(previous) => next_occupied_seat(previous, &seats),
        None => seats[0],
    };
    let small_blind = if seats.len() == 2 {
        button
    } else {
        next_occupied_seat(button, &seats)
    };
    let big_blind = next_occupied_seat(small_blind, &seats);
    state.snapshot.button_seat = Some(button);
    state.snapshot.small_blind_seat = Some(small_blind);
    state.snapshot.big_blind_seat = Some(big_blind);

    for player in &mut state.snapshot.players {
        let in_hand = player.seat.is_some();
        player.in_hand = in_hand;
        player.folded = false;
        player.committed = 0;
        player.street_committed = 0;
        player.hole_cards.clear();
        if in_hand {
            state.pending.insert(player.player_id.clone());
            state
                .street_contributions
                .insert(player.player_id.clone(), 0);
        }
    }

    // Deal from the first seat left of the button, round by round.
    let deal_order = rotate_seats(&seats, next_occupied_seat(button, &seats));
    for _ in 0..2 {
        for seat in &deal_order {
            let card = state
                .deck
                .pop()
                .ok_or_else(|| GameError::Storage("deterministic deck exhausted".into()))?;
            if let Some(player) = state
                .snapshot
                .players
                .iter_mut()
                .find(|player| player.seat == Some(*seat))
            {
                player.hole_cards.push(card);
            }
        }
    }
    post_blind(state, small_blind, state.snapshot.small_blind);
    post_blind(state, big_blind, state.snapshot.big_blind);
    state.snapshot.current_turn = Some(if seats.len() == 2 {
        player_at_seat(&state.snapshot, button)
    } else {
        player_at_seat(&state.snapshot, next_occupied_seat(big_blind, &seats))
    });
    skip_unable_players(state);
    Ok(())
}

fn post_blind(state: &mut TableState, seat: u8, blind: u64) {
    if let Some(player) = state
        .snapshot
        .players
        .iter_mut()
        .find(|player| player.seat == Some(seat))
    {
        let paid = blind.min(player.stack);
        player.stack -= paid;
        player.committed = paid;
        player.street_committed = paid;
        state.snapshot.pot += paid;
        state
            .street_contributions
            .insert(player.player_id.clone(), paid);
    }
}

fn apply_action(state: &mut TableState, request: &ActionRequest) -> Result<()> {
    if !matches!(
        state.snapshot.phase,
        TablePhase::Preflop | TablePhase::Flop | TablePhase::Turn | TablePhase::River
    ) {
        return Err(GameError::Conflict("no betting action is available".into()));
    }
    if state.snapshot.current_turn.as_deref() != Some(request.player_id.as_str()) {
        return Err(GameError::Conflict("it is not this player's turn".into()));
    }
    let index = state
        .snapshot
        .players
        .iter()
        .position(|player| player.player_id == request.player_id)
        .ok_or_else(|| GameError::NotFound("player is not at this table".into()))?;
    let street_committed = state.snapshot.players[index].street_committed;
    let stack = state.snapshot.players[index].stack;
    let current_bet = state.snapshot.current_bet;
    let action_amount = request.amount;

    match request.action {
        PlayerAction::Fold => {
            state.snapshot.players[index].folded = true;
            state.pending.remove(&request.player_id);
        }
        PlayerAction::Check => {
            if street_committed != current_bet {
                return Err(GameError::Invalid("cannot check while facing a bet".into()));
            }
            state.pending.remove(&request.player_id);
        }
        PlayerAction::Call => {
            if current_bet <= street_committed {
                return Err(GameError::Invalid("nothing to call; use check".into()));
            }
            let target = current_bet.min(street_committed + stack);
            commit_to(state, index, target)?;
            state.pending.remove(&request.player_id);
        }
        PlayerAction::Bet => {
            if current_bet != 0 {
                return Err(GameError::Invalid(
                    "use raise when a bet already exists".into(),
                ));
            }
            let target =
                action_amount.ok_or_else(|| GameError::Invalid("bet amount is required".into()))?;
            if target == 0 || target > street_committed + stack {
                return Err(GameError::Invalid(
                    "bet must be positive and within the player's stack".into(),
                ));
            }
            commit_to(state, index, target)?;
            state.snapshot.current_bet = target;
            state.snapshot.min_raise = target;
            reset_pending_after_raise(state, &request.player_id);
        }
        PlayerAction::Raise => {
            if current_bet == 0 {
                return Err(GameError::Invalid("use bet when no bet exists".into()));
            }
            let target = action_amount
                .ok_or_else(|| GameError::Invalid("raise amount is required".into()))?;
            if target <= current_bet || target > street_committed + stack {
                return Err(GameError::Invalid(
                    "raise must increase the current bet and fit the stack".into(),
                ));
            }
            if target - current_bet < state.snapshot.min_raise && target < street_committed + stack
            {
                return Err(GameError::Invalid(
                    "raise is smaller than the minimum raise".into(),
                ));
            }
            let raise_size = target - current_bet;
            commit_to(state, index, target)?;
            state.snapshot.current_bet = target;
            if raise_size >= state.snapshot.min_raise {
                state.snapshot.min_raise = raise_size;
                reset_pending_after_raise(state, &request.player_id);
            } else {
                state.pending.remove(&request.player_id);
            }
        }
        PlayerAction::AllIn => {
            if stack == 0 {
                return Err(GameError::Invalid("player is already all-in".into()));
            }
            let target = street_committed + stack;
            commit_to(state, index, target)?;
            if target > current_bet {
                let raise_size = target - current_bet;
                if raise_size >= state.snapshot.min_raise {
                    state.snapshot.current_bet = target;
                    state.snapshot.min_raise = raise_size;
                    reset_pending_after_raise(state, &request.player_id);
                } else {
                    state.pending.remove(&request.player_id);
                }
            } else {
                state.pending.remove(&request.player_id);
            }
        }
    }

    state.snapshot.last_action = Some(ActionRecord {
        action_id: request.action_id.clone(),
        player_id: request.player_id.clone(),
        action: request.action,
        amount: action_amount,
        action_seq: state.snapshot.action_seq + 1,
    });
    resolve_after_action(state)
}

fn commit_to(state: &mut TableState, index: usize, target: u64) -> Result<()> {
    let player = &mut state.snapshot.players[index];
    if target < player.street_committed || target > player.street_committed + player.stack {
        return Err(GameError::Invalid(
            "commitment is outside the player's available chips".into(),
        ));
    }
    let delta = target - player.street_committed;
    player.stack -= delta;
    player.street_committed = target;
    player.committed += delta;
    state.snapshot.pot += delta;
    state
        .street_contributions
        .insert(player.player_id.clone(), target);
    Ok(())
}

fn reset_pending_after_raise(state: &mut TableState, actor: &str) {
    state.pending = state
        .snapshot
        .players
        .iter()
        .filter(|player| {
            player.in_hand && !player.folded && player.stack > 0 && player.player_id != actor
        })
        .map(|player| player.player_id.clone())
        .collect();
}

fn resolve_after_action(state: &mut TableState) -> Result<()> {
    let active_count = state
        .snapshot
        .players
        .iter()
        .filter(|player| player.in_hand && !player.folded)
        .count();
    if active_count == 1 {
        let winner = state
            .snapshot
            .players
            .iter()
            .find(|player| player.in_hand && !player.folded)
            .map(|player| player.player_id.clone())
            .unwrap_or_default();
        settle_winner(state, vec![winner], Vec::new());
        return Ok(());
    }
    let all_equal = state
        .snapshot
        .players
        .iter()
        .filter(|player| player.in_hand && !player.folded)
        .all(|player| player.street_committed == state.snapshot.current_bet);
    skip_unable_players(state);
    let can_act = state
        .snapshot
        .players
        .iter()
        .any(|player| player.in_hand && !player.folded && player.stack > 0);
    if state.pending.is_empty() && (all_equal || !can_act) {
        advance_street(state)?;
    } else if state.pending.is_empty() {
        reset_pending_for_current_bet(state);
        skip_unable_players(state);
    } else {
        choose_next_turn(state);
    }
    Ok(())
}

fn advance_street(state: &mut TableState) -> Result<()> {
    for player in &mut state.snapshot.players {
        player.street_committed = 0;
    }
    state.street_contributions.clear();
    state.snapshot.current_bet = 0;
    state.snapshot.min_raise = state.snapshot.big_blind;
    state.pending.clear();
    match state.snapshot.phase {
        TablePhase::Preflop => {
            state.snapshot.phase = TablePhase::Flop;
            deal_board(state, 3)?;
        }
        TablePhase::Flop => {
            state.snapshot.phase = TablePhase::Turn;
            deal_board(state, 1)?;
        }
        TablePhase::Turn => {
            state.snapshot.phase = TablePhase::River;
            deal_board(state, 1)?;
        }
        TablePhase::River => {
            state.snapshot.phase = TablePhase::Showdown;
            settle_showdown(state)?;
            return Ok(());
        }
        _ => return Ok(()),
    }
    reset_pending_for_current_bet(state);
    skip_unable_players(state);
    if state.pending.is_empty() {
        advance_street(state)?;
    } else {
        choose_next_turn(state);
    }
    Ok(())
}

fn deal_board(state: &mut TableState, count: usize) -> Result<()> {
    for _ in 0..count {
        let card = state
            .deck
            .pop()
            .ok_or_else(|| GameError::Storage("deterministic deck exhausted".into()))?;
        state.snapshot.board.push(card);
    }
    Ok(())
}

fn settle_showdown(state: &mut TableState) -> Result<()> {
    let contenders: Vec<_> = state
        .snapshot
        .players
        .iter()
        .filter(|player| player.in_hand && !player.folded)
        .map(|player| {
            (
                player.player_id.clone(),
                player.seat.unwrap_or(u8::MAX),
                evaluate_best(
                    &player
                        .hole_cards
                        .iter()
                        .chain(state.snapshot.board.iter())
                        .cloned()
                        .collect::<Vec<_>>(),
                ),
            )
        })
        .collect();
    let Some(best) = contenders.iter().map(|(_, _, rank)| rank).max() else {
        return Err(GameError::Conflict("showdown has no contenders".into()));
    };
    let winners: Vec<_> = contenders
        .iter()
        .filter(|(_, _, rank)| rank == best)
        .map(|(player_id, _, _)| player_id.clone())
        .collect();
    settle_winner(
        state,
        winners,
        contenders
            .into_iter()
            .map(|(id, _, rank)| (id, rank))
            .collect(),
    );
    Ok(())
}

fn settle_winner(state: &mut TableState, winner_ids: Vec<String>, ranks: Vec<(String, HandRank)>) {
    let pot = state.snapshot.pot;
    let share = pot / winner_ids.len() as u64;
    let remainder = pot % winner_ids.len() as u64;
    let mut ordered = winner_ids;
    ordered.sort_by_key(|id| seat_of(&state.snapshot, id));
    for (position, player_id) in ordered.iter().enumerate() {
        let payout = share + u64::from(position < remainder as usize);
        if let Some(player) = state
            .snapshot
            .players
            .iter_mut()
            .find(|p| &p.player_id == player_id)
        {
            player.stack += payout;
        }
        let hand_rank = ranks
            .iter()
            .find(|(id, _)| id == player_id)
            .map(|(_, rank)| rank.as_vec())
            .unwrap_or_default();
        state.snapshot.winners.push(Winner {
            player_id: player_id.clone(),
            payout,
            hand_rank,
        });
    }
    state.snapshot.settled_pot = pot;
    state.snapshot.pot = 0;
    for player in &mut state.snapshot.players {
        player.committed = 0;
        player.street_committed = 0;
    }
    state.snapshot.phase = TablePhase::Complete;
    state.snapshot.current_turn = None;
    state.pending.clear();
}

fn skip_unable_players(state: &mut TableState) {
    state.pending.retain(|player_id| {
        state.snapshot.players.iter().any(|player| {
            &player.player_id == player_id && player.in_hand && !player.folded && player.stack > 0
        })
    });
    if state.pending.is_empty() {
        state.snapshot.current_turn = None;
    } else if state
        .snapshot
        .current_turn
        .as_ref()
        .is_none_or(|player_id| !state.pending.contains(player_id))
    {
        choose_next_turn(state);
    }
}

fn reset_pending_for_current_bet(state: &mut TableState) {
    state.pending = state
        .snapshot
        .players
        .iter()
        .filter(|player| player.in_hand && !player.folded && player.stack > 0)
        .map(|player| player.player_id.clone())
        .collect();
}

fn choose_next_turn(state: &mut TableState) {
    let Some(current) = state.snapshot.current_turn.as_deref() else {
        state.snapshot.current_turn = state.pending.iter().next().cloned();
        return;
    };
    let current_seat = seat_of(&state.snapshot, current);
    let mut seats: Vec<_> = state
        .snapshot
        .players
        .iter()
        .filter_map(|player| player.seat)
        .collect();
    seats.sort_unstable();
    for seat in rotate_seats(&seats, next_occupied_seat(current_seat, &seats)) {
        if let Some(player) =
            state.snapshot.players.iter().find(|player| {
                player.seat == Some(seat) && state.pending.contains(&player.player_id)
            })
        {
            state.snapshot.current_turn = Some(player.player_id.clone());
            return;
        }
    }
    state.snapshot.current_turn = state.pending.iter().next().cloned();
}

fn player_at_seat(snapshot: &TableSnapshot, seat: u8) -> String {
    snapshot
        .players
        .iter()
        .find(|player| player.seat == Some(seat))
        .map(|player| player.player_id.clone())
        .unwrap_or_default()
}

fn seat_of(snapshot: &TableSnapshot, player_id: &str) -> u8 {
    snapshot
        .players
        .iter()
        .find(|player| player.player_id == player_id)
        .and_then(|player| player.seat)
        .unwrap_or(u8::MAX)
}

fn next_occupied_seat(current: u8, seats: &[u8]) -> u8 {
    seats
        .iter()
        .copied()
        .find(|seat| *seat > current)
        .unwrap_or_else(|| seats[0])
}

fn rotate_seats(seats: &[u8], start: u8) -> Vec<u8> {
    let mut result: Vec<_> = seats
        .iter()
        .copied()
        .filter(|seat| *seat >= start)
        .collect();
    result.extend(seats.iter().copied().filter(|seat| *seat < start));
    result
}

fn deterministic_deck(seed: u64) -> Vec<Card> {
    let suits = ['c', 'd', 'h', 's'];
    let mut deck: Vec<_> = suits
        .iter()
        .flat_map(|suit| (2..=14).map(move |rank| Card::new(rank, *suit)))
        .collect();
    let mut state = seed;
    for index in (1..deck.len()).rev() {
        state = splitmix64(state);
        let swap = (state as usize) % (index + 1);
        deck.swap(index, swap);
    }
    deck
}

fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = value;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandRank {
    pub category: u8,
    pub kickers: Vec<u8>,
}

impl HandRank {
    fn as_vec(&self) -> Vec<u8> {
        std::iter::once(self.category)
            .chain(self.kickers.iter().copied())
            .collect()
    }
}

impl Ord for HandRank {
    fn cmp(&self, other: &Self) -> Ordering {
        self.category
            .cmp(&other.category)
            .then_with(|| self.kickers.cmp(&other.kickers))
    }
}

impl PartialOrd for HandRank {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

pub fn evaluate_best(cards: &[Card]) -> HandRank {
    assert!(cards.len() >= 5, "at least five cards are required");
    let mut best: Option<HandRank> = None;
    let mut chosen = Vec::with_capacity(5);
    combinations(cards, 0, &mut chosen, &mut |five| {
        let rank = evaluate_five(five);
        if best.as_ref().is_none_or(|current| rank > *current) {
            best = Some(rank);
        }
    });
    best.expect("five-card combinations exist")
}

fn combinations<F: FnMut(&[Card])>(
    cards: &[Card],
    start: usize,
    chosen: &mut Vec<Card>,
    callback: &mut F,
) {
    if chosen.len() == 5 {
        callback(chosen);
        return;
    }
    let needed = 5 - chosen.len();
    for index in start..=cards.len() - needed {
        chosen.push(cards[index].clone());
        combinations(cards, index + 1, chosen, callback);
        chosen.pop();
    }
}

fn evaluate_five(cards: &[Card]) -> HandRank {
    let flush = cards.iter().all(|card| card.suit == cards[0].suit);
    let mut ranks: Vec<_> = cards.iter().map(|card| card.rank).collect();
    ranks.sort_unstable_by(|a, b| b.cmp(a));
    let straight_high = straight_high(&ranks);
    let mut counts: BTreeMap<u8, usize> = BTreeMap::new();
    for rank in &ranks {
        *counts.entry(*rank).or_default() += 1;
    }
    let mut groups: Vec<(usize, u8)> = counts
        .into_iter()
        .map(|(rank, count)| (count, rank))
        .collect();
    groups.sort_unstable_by(|a, b| b.cmp(a));
    if flush && straight_high > 0 {
        return HandRank {
            category: 8,
            kickers: vec![straight_high],
        };
    }
    if groups[0].0 == 4 {
        return HandRank {
            category: 7,
            kickers: vec![groups[0].1, groups[1].1],
        };
    }
    if groups[0].0 == 3 && groups[1].0 == 2 {
        return HandRank {
            category: 6,
            kickers: vec![groups[0].1, groups[1].1],
        };
    }
    if flush {
        return HandRank {
            category: 5,
            kickers: ranks,
        };
    }
    if straight_high > 0 {
        return HandRank {
            category: 4,
            kickers: vec![straight_high],
        };
    }
    if groups[0].0 == 3 {
        let mut kickers: Vec<_> = groups.iter().skip(1).map(|(_, rank)| *rank).collect();
        kickers.sort_unstable_by(|a, b| b.cmp(a));
        return HandRank {
            category: 3,
            kickers: std::iter::once(groups[0].1).chain(kickers).collect(),
        };
    }
    if groups[0].0 == 2 && groups[1].0 == 2 {
        let pair_high = groups[0].1.max(groups[1].1);
        let pair_low = groups[0].1.min(groups[1].1);
        return HandRank {
            category: 2,
            kickers: vec![pair_high, pair_low, groups[2].1],
        };
    }
    if groups[0].0 == 2 {
        let mut kickers: Vec<_> = groups.iter().skip(1).map(|(_, rank)| *rank).collect();
        kickers.sort_unstable_by(|a, b| b.cmp(a));
        return HandRank {
            category: 1,
            kickers: std::iter::once(groups[0].1).chain(kickers).collect(),
        };
    }
    HandRank {
        category: 0,
        kickers: ranks,
    }
}

fn straight_high(ranks: &[u8]) -> u8 {
    let mut unique = ranks.to_vec();
    unique.sort_unstable();
    unique.dedup();
    if unique == [2, 3, 4, 5, 14] {
        return 5;
    }
    if unique.len() == 5 && unique.windows(2).all(|window| window[1] == window[0] + 1) {
        return *unique.last().unwrap_or(&0);
    }
    0
}

pub fn routes(store: TableStore) -> Router<()> {
    Router::new()
        .route("/tables", get(list_tables).post(create_table))
        .route("/tables/:table_id", get(get_table))
        .route("/tables/:table_id/join", post(join_table))
        .route("/tables/:table_id/seat", post(seat_player))
        .route("/tables/:table_id/leave", post(leave_table))
        .route("/tables/:table_id/start", post(start_hand))
        .route("/tables/:table_id/action", post(action))
        .route("/tables/:table_id/events", get(events))
        .with_state(store)
}

async fn list_tables(State(store): State<TableStore>) -> ApiResult<Json<Vec<TableSnapshot>>> {
    Ok(Json(store.list_tables()?))
}

async fn create_table(
    State(store): State<TableStore>,
    Json(request): Json<CreateTableRequest>,
) -> ApiResult<(StatusCode, Json<TableSnapshot>)> {
    Ok((StatusCode::CREATED, Json(store.create_table(request)?)))
}

async fn get_table(
    State(store): State<TableStore>,
    Path(table_id): Path<String>,
) -> ApiResult<Json<TableSnapshot>> {
    Ok(Json(store.snapshot(&table_id)?))
}

async fn join_table(
    State(store): State<TableStore>,
    Path(table_id): Path<String>,
    Json(request): Json<JoinTableRequest>,
) -> ApiResult<Json<TableSnapshot>> {
    Ok(Json(store.join_table(&table_id, request)?))
}

async fn seat_player(
    State(store): State<TableStore>,
    Path(table_id): Path<String>,
    Json(request): Json<SeatPlayerRequest>,
) -> ApiResult<Json<TableSnapshot>> {
    Ok(Json(store.seat_player(&table_id, request)?))
}

async fn leave_table(
    State(store): State<TableStore>,
    Path(table_id): Path<String>,
    Json(request): Json<LeaveTableRequest>,
) -> ApiResult<Json<TableSnapshot>> {
    Ok(Json(store.leave_table(&table_id, request)?))
}

async fn start_hand(
    State(store): State<TableStore>,
    Path(table_id): Path<String>,
) -> ApiResult<Json<TableSnapshot>> {
    Ok(Json(store.start_hand(&table_id)?))
}

async fn action(
    State(store): State<TableStore>,
    Path(table_id): Path<String>,
    Json(request): Json<ActionRequest>,
) -> ApiResult<Json<ActionResult>> {
    Ok(Json(store.act(&table_id, request)?))
}

async fn events(
    State(store): State<TableStore>,
    Path(table_id): Path<String>,
) -> ApiResult<
    Sse<impl tokio_stream::Stream<Item = std::result::Result<Event, std::convert::Infallible>>>,
> {
    let receiver = store
        .subscribe()
        .ok_or_else(|| GameError::Conflict("realtime publishing is not configured".into()))?;
    let stream = BroadcastStream::new(receiver).filter_map(move |result| {
        let table_id = table_id.clone();
        match result {
            Ok(event) if event.table_id == table_id => Some(Ok(Event::default()
                .event(event.topic.clone())
                .json_data(event)
                .unwrap_or_default())),
            _ => None,
        }
    });
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

type ApiResult<T> = std::result::Result<T, GameError>;

impl IntoResponse for GameError {
    fn into_response(self) -> axum::response::Response {
        let status = match self {
            Self::Invalid(_) => StatusCode::BAD_REQUEST,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::StaleAction { .. } => StatusCode::CONFLICT,
            Self::Storage(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (
            status,
            Json(serde_json::json!({ "error": self.to_string() })),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(rank: u8, suit: char) -> Card {
        Card::new(rank, suit)
    }

    fn create_store() -> (TableStore, String) {
        let store = TableStore::open_in_memory().expect("in-memory store");
        let table = store
            .create_table(CreateTableRequest {
                name: "Proof table".into(),
                max_seats: Some(6),
                small_blind: Some(5),
                big_blind: Some(10),
                starting_stack: Some(100),
            })
            .expect("table");
        (store, table.table_id)
    }

    fn seat_two(store: &TableStore, table_id: &str) {
        for (player_id, seat) in [("alice", 0), ("bob", 1)] {
            store
                .join_table(
                    table_id,
                    JoinTableRequest {
                        player_id: player_id.into(),
                        display_name: player_id.into(),
                        stack: Some(100),
                    },
                )
                .expect("join");
            store
                .seat_player(
                    table_id,
                    SeatPlayerRequest {
                        player_id: player_id.into(),
                        seat,
                    },
                )
                .expect("seat");
        }
    }

    #[test]
    fn ranking_orders_straight_flush_full_house_and_wheel() {
        let straight_flush = evaluate_best(&[
            card(10, 'h'),
            card(11, 'h'),
            card(12, 'h'),
            card(13, 'h'),
            card(14, 'h'),
            card(2, 'c'),
            card(3, 'd'),
        ]);
        let full_house = evaluate_best(&[
            card(14, 'c'),
            card(14, 'd'),
            card(14, 'h'),
            card(2, 'c'),
            card(2, 'd'),
            card(9, 's'),
            card(8, 's'),
        ]);
        let wheel = evaluate_best(&[
            card(14, 'c'),
            card(2, 'd'),
            card(3, 'h'),
            card(4, 's'),
            card(5, 'c'),
            card(9, 'd'),
            card(10, 'h'),
        ]);
        let six_high = evaluate_best(&[
            card(2, 'c'),
            card(3, 'd'),
            card(4, 'h'),
            card(5, 's'),
            card(6, 'c'),
            card(9, 'd'),
            card(10, 'h'),
        ]);
        assert!(straight_flush > full_house);
        assert_eq!(wheel.as_vec(), vec![4, 5]);
        assert!(six_high > wheel);
    }

    #[test]
    fn betting_rejects_invalid_check_and_preserves_simulated_tokens() {
        let (store, table_id) = create_store();
        seat_two(&store, &table_id);
        let before: u64 = store
            .snapshot(&table_id)
            .unwrap()
            .players
            .iter()
            .map(|player| player.stack + player.committed)
            .sum();
        let snapshot = store.snapshot(&table_id).unwrap();
        let actor = snapshot.current_turn.clone().unwrap();
        let error = store
            .act(
                &table_id,
                ActionRequest {
                    action_id: "bad-check".into(),
                    player_id: actor,
                    expected_action_seq: snapshot.action_seq,
                    action: PlayerAction::Check,
                    amount: None,
                },
            )
            .unwrap_err();
        assert!(error.to_string().contains("cannot check"));
        let after: u64 = store
            .snapshot(&table_id)
            .unwrap()
            .players
            .iter()
            .map(|player| player.stack + player.committed)
            .sum();
        assert_eq!(before, after);
    }

    #[test]
    fn deterministic_progression_has_blinds_and_board_streets() {
        let (store, table_id) = create_store();
        seat_two(&store, &table_id);
        let snapshot = store.snapshot(&table_id).unwrap();
        assert_eq!(snapshot.phase, TablePhase::Preflop);
        assert_eq!(snapshot.pot, 15);
        assert_eq!(snapshot.board.len(), 0);
        let actor = snapshot.current_turn.clone().unwrap();
        let result = store
            .act(
                &table_id,
                ActionRequest {
                    action_id: "fold-preflop".into(),
                    player_id: actor,
                    expected_action_seq: 0,
                    action: PlayerAction::Fold,
                    amount: None,
                },
            )
            .unwrap();
        assert_eq!(result.snapshot.phase, TablePhase::Complete);
        assert_eq!(result.snapshot.board.len(), 0);
        assert_eq!(result.snapshot.settled_pot, 15);
        assert_eq!(result.snapshot.winners.len(), 1);
    }

    #[test]
    fn deterministic_deck_repeats_exactly_for_a_seed() {
        let first = deterministic_deck(42);
        let second = deterministic_deck(42);
        assert_eq!(first, second);
        assert_eq!(first.len(), 52);
        let unique: BTreeSet<_> = first.iter().cloned().collect();
        assert_eq!(unique.len(), 52);
    }

    #[test]
    fn checking_through_all_streets_reaches_showdown_and_keeps_chips_conserved() {
        let (store, table_id) = create_store();
        seat_two(&store, &table_id);
        let before: u64 = store
            .snapshot(&table_id)
            .unwrap()
            .players
            .iter()
            .map(|player| player.stack + player.committed)
            .sum();
        let mut next_action = 0;
        let mut first_preflop = true;
        loop {
            let snapshot = store.snapshot(&table_id).unwrap();
            if snapshot.phase == TablePhase::Complete {
                assert_eq!(snapshot.board.len(), 5);
                assert_eq!(snapshot.winners.len(), 1);
                break;
            }
            let player_id = snapshot.current_turn.clone().expect("a player must act");
            let action = if first_preflop {
                first_preflop = false;
                PlayerAction::Call
            } else {
                PlayerAction::Check
            };
            store
                .act(
                    &table_id,
                    ActionRequest {
                        action_id: format!("street-{next_action}"),
                        player_id,
                        expected_action_seq: snapshot.action_seq,
                        action,
                        amount: None,
                    },
                )
                .expect("legal street action");
            next_action += 1;
        }
        let after: u64 = store
            .snapshot(&table_id)
            .unwrap()
            .players
            .iter()
            .map(|player| player.stack + player.committed)
            .sum();
        assert_eq!(before, after);
    }

    #[test]
    fn duplicate_action_replays_and_stale_sequence_is_rejected() {
        let (store, table_id) = create_store();
        seat_two(&store, &table_id);
        let snapshot = store.snapshot(&table_id).unwrap();
        let player_id = snapshot.current_turn.clone().unwrap();
        let request = ActionRequest {
            action_id: "idempotent-fold".into(),
            player_id,
            expected_action_seq: snapshot.action_seq,
            action: PlayerAction::Fold,
            amount: None,
        };
        let first = store.act(&table_id, request.clone()).unwrap();
        let replay = store.act(&table_id, request).unwrap();
        assert!(!first.replayed);
        assert!(replay.replayed);
        assert_eq!(first.snapshot, replay.snapshot);
        let stale = store
            .act(
                &table_id,
                ActionRequest {
                    action_id: "stale-action".into(),
                    player_id: "alice".into(),
                    expected_action_seq: 0,
                    action: PlayerAction::Fold,
                    amount: None,
                },
            )
            .unwrap_err();
        assert!(matches!(stale, GameError::StaleAction { actual: 1, .. }));
    }

    #[test]
    fn snapshot_serialization_round_trips_without_losing_authoritative_fields() {
        let (store, table_id) = create_store();
        seat_two(&store, &table_id);
        let snapshot = store.snapshot(&table_id).unwrap();
        let json = serde_json::to_string(&snapshot).unwrap();
        let decoded: TableSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, snapshot);
        assert!(json.contains("\"action_seq\""));
        assert!(json.contains("\"hole_cards\""));
        assert!(json.contains("\"simulated") == false);
    }
}

