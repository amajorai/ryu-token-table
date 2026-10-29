//! Authoritative local state for cosmetic six-max no-limit Texas Hold'em.
//!
//! The crate owns the SQLite state machine and exposes a state-baked Axum router.
//! It has no dependency on Core: Core's generic ext-proxy can mount the router at
//! `/api/token-table`, while the binary in `main.rs` supplies the loopback/auth shell.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{Display, Formatter};
use std::sync::{Arc, Mutex};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use rand::{rngs::OsRng, seq::SliceRandom, RngCore};
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
    deck: Vec<Card>,
    pending: BTreeSet<String>,
    street_contributions: BTreeMap<String, u64>,
    /// Players who already acted this street and now face only an incomplete
    /// (short all-in) raise: they may call or fold, but action is not reopened
    /// for them until a full bet or raise.
    #[serde(default)]
    call_only: BTreeSet<String>,
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

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewerCredential {
    pub player_id: String,
    pub viewer_token: String,
}

#[derive(Deserialize)]
pub struct ViewerRequest<T> {
    #[serde(rename = "viewerToken")]
    pub viewer_token: String,
    #[serde(flatten)]
    pub request: T,
}

#[derive(Deserialize)]
pub struct JoinViewerRequest {
    #[serde(default)]
    #[serde(rename = "viewerToken")]
    pub viewer_token: Option<String>,
    pub display_name: String,
    #[serde(default)]
    pub stack: Option<u64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JoinedTable {
    pub snapshot: TableSnapshot,
    pub player_id: String,
    pub viewer_token: String,
}

#[derive(Deserialize)]
pub struct ViewRequest {
    #[serde(rename = "viewerToken")]
    pub viewer_token: String,
}

#[derive(Serialize)]
pub struct TableView {
    #[serde(flatten)]
    pub snapshot: TableSnapshot,
    #[serde(rename = "viewerUpgradeRequired")]
    pub viewer_upgrade_required: bool,
}

/// Only explicit showdown contenders, or the verified owner, receive hole cards.
fn visible_snapshot(mut snapshot: TableSnapshot, viewer: Option<&str>) -> TableSnapshot {
    let revealed = snapshot.phase == TablePhase::Showdown
        || (snapshot.phase == TablePhase::Complete
            && snapshot
                .winners
                .iter()
                .any(|winner| !winner.hand_rank.is_empty()));
    for player in &mut snapshot.players {
        if viewer != Some(player.player_id.as_str())
            && !(revealed && player.in_hand && !player.folded)
        {
            player.hole_cards.clear();
        }
    }
    snapshot
}

#[derive(Debug)]
pub enum GameError {
    Unauthorized,
    Invalid(String),
    NotFound(String),
    Conflict(String),
    StaleAction { expected: u64, actual: u64 },
    Storage(String),
}

impl Display for GameError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unauthorized => write!(f, "viewer authorization required"),
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

/// `PRAGMA user_version` from which `viewer_credentials.viewer_token` holds the
/// SHA-256 digest of each token instead of the bearer itself. Both are 64 hex
/// characters, so the version (not the value's shape) says which a row holds.
const VIEWER_TOKEN_DIGEST_VERSION: i64 = 1;

/// The stored form of a viewer token: hex SHA-256. The token is 32 random bytes,
/// so a fast hash is enough; the database alone no longer grants a seat.
fn viewer_token_digest(token: &str) -> String {
    use sha2::Digest as _;
    hex::encode(sha2::Sha256::digest(token.as_bytes()))
}

/// Replace every raw viewer token an older version stored with its digest, once,
/// in one transaction with the version bump.
fn digest_stored_viewer_tokens(connection: &Connection) -> Result<()> {
    let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if version >= VIEWER_TOKEN_DIGEST_VERSION {
        return Ok(());
    }
    let transaction = connection.unchecked_transaction()?;
    let rows = {
        let mut statement = transaction
            .prepare("SELECT table_id, player_id, viewer_token FROM viewer_credentials")?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    for (table_id, player_id, token) in rows {
        transaction.execute(
            "UPDATE viewer_credentials SET viewer_token = ?3 WHERE table_id = ?1 AND player_id = ?2",
            params![table_id, player_id, viewer_token_digest(&token)],
        )?;
    }
    transaction.execute_batch(&format!(
        "PRAGMA user_version = {VIEWER_TOKEN_DIGEST_VERSION}"
    ))?;
    transaction.commit()?;
    Ok(())
}

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
             PRAGMA secure_delete = ON;
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
             );
             CREATE TABLE IF NOT EXISTS viewer_credentials (
                 table_id TEXT NOT NULL REFERENCES tables(table_id),
                 player_id TEXT NOT NULL,
                 viewer_token TEXT NOT NULL UNIQUE,
                 PRIMARY KEY (table_id, player_id)
             );
             CREATE TABLE IF NOT EXISTS table_upgrades (
                 legacy_table_id TEXT PRIMARY KEY REFERENCES tables(table_id),
                 table_id TEXT NOT NULL REFERENCES tables(table_id)
             );",
        )?;
        digest_stored_viewer_tokens(&connection)?;
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
        let state = new_table_state(request)?;
        let snapshot = state.snapshot.clone();
        let connection = self.connection.lock().map_err(lock_error)?;
        insert_state(&connection, &state)?;
        drop(connection);
        self.publish(&snapshot);
        Ok(snapshot)
    }

    fn upgrade_table(&self, table_id: &str) -> Result<TableView> {
        let mut connection = self.connection.lock().map_err(lock_error)?;
        let mapped: Option<String> = connection
            .query_row(
                "SELECT table_id FROM table_upgrades WHERE legacy_table_id = ?1",
                [table_id],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(mapped) = mapped {
            return table_view(
                &connection,
                load_state(&connection, &mapped)?.snapshot,
                None,
            );
        }
        let original = load_state(&connection, table_id)?.snapshot;
        if !table_view(&connection, original.clone(), None)?.viewer_upgrade_required {
            return table_view(&connection, original, None);
        }
        let replacement = new_table_state(CreateTableRequest {
            name: original.name,
            max_seats: Some(original.max_seats),
            small_blind: Some(original.small_blind),
            big_blind: Some(original.big_blind),
            starting_stack: Some(original.starting_stack),
        })?;
        let tx = connection.transaction()?;
        insert_state(&tx, &replacement)?;
        tx.execute(
            "INSERT INTO table_upgrades (legacy_table_id, table_id) VALUES (?1, ?2)",
            params![table_id, replacement.snapshot.table_id],
        )?;
        tx.commit()?;
        let view = table_view(&connection, replacement.snapshot, None)?;
        drop(connection);
        self.publish(&view.snapshot);
        Ok(view)
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

    fn viewer(&self, table_id: &str, token: &str) -> Result<String> {
        if token.len() != 64 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(GameError::Unauthorized);
        }
        let connection = self.connection.lock().map_err(lock_error)?;
        // The column holds the token's digest; the presented bearer is hashed.
        connection.query_row(
            "SELECT player_id FROM viewer_credentials WHERE table_id = ?1 AND viewer_token = ?2",
            params![table_id, viewer_token_digest(token)], |row| row.get(0),
        ).optional()?.ok_or(GameError::Unauthorized)
    }

    fn table_view(&self, snapshot: TableSnapshot, viewer: Option<&str>) -> Result<TableView> {
        let connection = self.connection.lock().map_err(lock_error)?;
        table_view(&connection, snapshot, viewer)
    }

    fn require_player(&self, table_id: &str, token: &str, player_id: &str) -> Result<String> {
        let viewer = self.viewer(table_id, token)?;
        if viewer != player_id {
            return Err(GameError::Unauthorized);
        }
        Ok(viewer)
    }

    fn join_viewer(&self, table_id: &str, request: JoinViewerRequest) -> Result<JoinedTable> {
        if request.display_name.trim().is_empty() || request.stack == Some(0) {
            return Err(GameError::Invalid(
                "display_name and a positive stack are required".into(),
            ));
        }
        let credential = if let Some(token) = request.viewer_token {
            let player_id = self.viewer(table_id, &token)?;
            let snapshot = self.snapshot(table_id)?;
            if snapshot
                .players
                .iter()
                .any(|player| player.player_id == player_id)
            {
                return Ok(JoinedTable {
                    snapshot: visible_snapshot(snapshot, Some(&player_id)),
                    player_id,
                    viewer_token: token,
                });
            }
            ViewerCredential {
                player_id,
                viewer_token: token,
            }
        } else {
            // A caller-supplied player ID can never acquire another player's capability.
            let mut secret = [0u8; 32];
            OsRng.fill_bytes(&mut secret);
            let credential = ViewerCredential {
                player_id: format!("player-{}", Uuid::new_v4()),
                viewer_token: hex::encode(secret),
            };
            let connection = self.connection.lock().map_err(lock_error)?;
            load_state(&connection, table_id)?;
            connection.execute(
                "INSERT INTO viewer_credentials (table_id, player_id, viewer_token) VALUES (?1, ?2, ?3)",
                params![
                    table_id,
                    credential.player_id,
                    viewer_token_digest(&credential.viewer_token)
                ],
            )?;
            credential
        };
        let snapshot = self.join_table(
            table_id,
            JoinTableRequest {
                player_id: credential.player_id.clone(),
                display_name: request.display_name,
                stack: request.stack,
            },
        )?;
        Ok(JoinedTable {
            snapshot: visible_snapshot(snapshot, Some(&credential.player_id)),
            player_id: credential.player_id,
            viewer_token: credential.viewer_token,
        })
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
            snapshot: visible_snapshot(snapshot.clone(), None),
        });
    }
}

fn new_table_state(request: CreateTableRequest) -> Result<TableState> {
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
        snapshot,
        deck: Vec::new(),
        pending: BTreeSet::new(),
        street_contributions: BTreeMap::new(),
        call_only: BTreeSet::new(),
    };
    Ok(state)
}

fn table_view(
    connection: &Connection,
    snapshot: TableSnapshot,
    viewer: Option<&str>,
) -> Result<TableView> {
    let mut viewer_upgrade_required = false;
    for player in snapshot
        .players
        .iter()
        .filter(|player| player.seat.is_some())
    {
        let claimed: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM viewer_credentials WHERE table_id = ?1 AND player_id = ?2)", params![snapshot.table_id, player.player_id], |row| row.get(0))?;
        if !claimed {
            viewer_upgrade_required = true;
            break;
        }
    }
    Ok(TableView {
        snapshot: visible_snapshot(snapshot, viewer),
        viewer_upgrade_required,
    })
}

fn lock_error<T>(_: std::sync::PoisonError<T>) -> GameError {
    GameError::Storage("store lock poisoned".into())
}

fn now_string() -> String {
    chrono::Utc::now().to_rfc3339()
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
    state.call_only.clear();
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

/// Seated players with chips behind; a busted (0-stack) seat is not dealt in.
fn seated_player_ids(snapshot: &TableSnapshot) -> Vec<String> {
    let mut seated: Vec<_> = snapshot
        .players
        .iter()
        .filter(|player| player.stack > 0)
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
        .filter(|player| player.stack > 0)
        .filter_map(|player| player.seat.map(|seat| (seat, player.player_id.clone())))
        .collect();
    if seated.len() < 2 {
        return Err(GameError::Conflict(
            "at least two seated players with chips are required".into(),
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
    state.call_only.clear();
    state.street_contributions.clear();
    state.deck = shuffled_deck();

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
        let in_hand = player.seat.is_some() && player.stack > 0;
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
            ensure_action_reopened(state, &request.player_id)?;
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
                reopen_for_incomplete_raise(state, &request.player_id);
            }
        }
        PlayerAction::AllIn => {
            if stack == 0 {
                return Err(GameError::Invalid("player is already all-in".into()));
            }
            let target = street_committed + stack;
            if target > current_bet {
                // Going all-in for more than the bet is a raise, which a
                // player facing only an incomplete raise may not make.
                ensure_action_reopened(state, &request.player_id)?;
            }
            commit_to(state, index, target)?;
            if target > current_bet {
                let raise_size = target - current_bet;
                // An incomplete all-in still raises the bet others must call;
                // only a full raise resets the minimum and reopens action.
                state.snapshot.current_bet = target;
                if raise_size >= state.snapshot.min_raise {
                    state.snapshot.min_raise = raise_size;
                    reset_pending_after_raise(state, &request.player_id);
                } else {
                    reopen_for_incomplete_raise(state, &request.player_id);
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
    state.call_only.clear();
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

/// After an incomplete raise, only players who still owe chips must act
/// again. Those who had already acted may call or fold but not re-raise;
/// players who had not yet acted keep every option.
fn reopen_for_incomplete_raise(state: &mut TableState, actor: &str) {
    state.pending.remove(actor);
    let current_bet = state.snapshot.current_bet;
    for player in &state.snapshot.players {
        if player.player_id == actor
            || !player.in_hand
            || player.folded
            || player.stack == 0
            || player.street_committed >= current_bet
        {
            continue;
        }
        if state.pending.insert(player.player_id.clone()) {
            state.call_only.insert(player.player_id.clone());
        }
    }
}

fn ensure_action_reopened(state: &TableState, player_id: &str) -> Result<()> {
    if state.call_only.contains(player_id) {
        return Err(GameError::Invalid(
            "an incomplete all-in does not reopen raising; call or fold".into(),
        ));
    }
    Ok(())
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
    // All-in players cannot match a later bet, so only players with chips
    // behind decide whether the street's betting is closed.
    let all_equal = state
        .snapshot
        .players
        .iter()
        .filter(|player| player.in_hand && !player.folded && player.stack > 0)
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
    }
    // Otherwise the actor has left `pending`, so `skip_unable_players` has
    // already passed the turn to the next pending seat; advancing again here
    // would skip a player whenever three or more are in the hand.
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
    state.call_only.clear();
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
                player.committed,
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
    let Some(top_contender_commitment) =
        contenders.iter().map(|(_, committed, _)| *committed).max()
    else {
        return Err(GameError::Conflict("showdown has no contenders".into()));
    };
    // Split the pot into contribution layers (main pot, then side pots). A
    // player can only win a layer they matched; a layer funded by one
    // contender alone is an uncalled bet and goes back to them.
    let mut levels: Vec<u64> = state
        .snapshot
        .players
        .iter()
        .map(|player| player.committed)
        .filter(|committed| *committed > 0)
        .collect();
    levels.sort_unstable();
    levels.dedup();
    let mut awards: BTreeMap<String, u64> = BTreeMap::new();
    let mut refunds: BTreeMap<String, u64> = BTreeMap::new();
    let mut previous = 0;
    for level in levels {
        let contributors: Vec<&PlayerSnapshot> = state
            .snapshot
            .players
            .iter()
            .filter(|player| player.committed > previous)
            .collect();
        let layer: u64 = contributors
            .iter()
            .map(|player| player.committed.min(level) - previous)
            .sum();
        previous = level;
        if let [only] = contributors.as_slice() {
            if only.in_hand && !only.folded {
                *refunds.entry(only.player_id.clone()).or_default() += layer;
                continue;
            }
        }
        // Chips above every contender's commitment (left by players who
        // folded) go to whoever wins the top contested layer.
        let threshold = level.min(top_contender_commitment);
        let eligible: Vec<_> = contenders
            .iter()
            .filter(|(_, committed, _)| *committed >= threshold)
            .collect();
        let Some(best) = eligible.iter().map(|(_, _, rank)| rank).max() else {
            continue;
        };
        let layer_winners: Vec<String> = eligible
            .iter()
            .filter(|(_, _, rank)| rank == best)
            .map(|(player_id, _, _)| player_id.clone())
            .collect();
        for (player_id, payout) in split_by_seat(&state.snapshot, layer_winners, layer) {
            *awards.entry(player_id).or_default() += payout;
        }
    }
    let ranks: Vec<_> = contenders
        .into_iter()
        .map(|(id, _, rank)| (id, rank))
        .collect();
    complete_settlement(state, awards, &ranks, refunds);
    Ok(())
}

/// Fold-out path: the last player standing is entitled to every layer.
fn settle_winner(state: &mut TableState, winner_ids: Vec<String>, ranks: Vec<(String, HandRank)>) {
    let awards = split_by_seat(&state.snapshot, winner_ids, state.snapshot.pot)
        .into_iter()
        .collect();
    complete_settlement(state, awards, &ranks, BTreeMap::new());
}

/// Divide `amount` evenly, giving odd chips to the earliest seats.
fn split_by_seat(
    snapshot: &TableSnapshot,
    mut winner_ids: Vec<String>,
    amount: u64,
) -> Vec<(String, u64)> {
    if winner_ids.is_empty() {
        return Vec::new();
    }
    let share = amount / winner_ids.len() as u64;
    let remainder = amount % winner_ids.len() as u64;
    winner_ids.sort_by_key(|id| seat_of(snapshot, id));
    winner_ids
        .into_iter()
        .enumerate()
        .map(|(position, player_id)| {
            let payout = share + u64::from((position as u64) < remainder);
            (player_id, payout)
        })
        .collect()
}

fn complete_settlement(
    state: &mut TableState,
    awards: BTreeMap<String, u64>,
    ranks: &[(String, HandRank)],
    refunds: BTreeMap<String, u64>,
) {
    for (player_id, refund) in &refunds {
        if let Some(player) = state
            .snapshot
            .players
            .iter_mut()
            .find(|p| &p.player_id == player_id)
        {
            player.stack += refund;
        }
    }
    let mut ordered: Vec<_> = awards.into_iter().collect();
    ordered.sort_by_key(|(id, _)| seat_of(&state.snapshot, id));
    let mut settled = 0;
    for (player_id, payout) in ordered {
        if let Some(player) = state
            .snapshot
            .players
            .iter_mut()
            .find(|p| p.player_id == player_id)
        {
            player.stack += payout;
        }
        settled += payout;
        let hand_rank = ranks
            .iter()
            .find(|(id, _)| *id == player_id)
            .map(|(_, rank)| rank.as_vec())
            .unwrap_or_default();
        state.snapshot.winners.push(Winner {
            player_id,
            payout,
            hand_rank,
        });
    }
    state.snapshot.settled_pot = settled;
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

fn ordered_deck() -> Vec<Card> {
    let suits = ['c', 'd', 'h', 's'];
    suits
        .iter()
        .flat_map(|suit| (2..=14).map(move |rank| Card::new(rank, *suit)))
        .collect()
}

fn shuffled_deck() -> Vec<Card> {
    let mut deck = ordered_deck();
    deck.shuffle(&mut OsRng);
    deck
}

#[cfg(test)]
fn deterministic_deck(seed: u64) -> Vec<Card> {
    let mut deck = ordered_deck();
    let mut state = seed;
    for index in (1..deck.len()).rev() {
        state = splitmix64(state);
        let swap = (state as usize) % (index + 1);
        deck.swap(index, swap);
    }
    deck
}

#[cfg(test)]
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
        .route("/tables/:table_id/view", post(view_table))
        .route("/tables/:table_id/upgrade", post(upgrade_table))
        .route("/tables/:table_id/join", post(join_table))
        .route("/tables/:table_id/seat", post(seat_player))
        .route("/tables/:table_id/leave", post(leave_table))
        .route("/tables/:table_id/start", post(start_hand))
        .route("/tables/:table_id/action", post(action))
        .route("/tables/:table_id/events", get(events))
        .with_state(store)
}

async fn list_tables(State(store): State<TableStore>) -> ApiResult<Json<Vec<TableView>>> {
    Ok(Json(
        store
            .list_tables()?
            .into_iter()
            .map(|snapshot| store.table_view(snapshot, None))
            .collect::<Result<Vec<_>>>()?,
    ))
}

async fn create_table(
    State(store): State<TableStore>,
    Json(request): Json<CreateTableRequest>,
) -> ApiResult<(StatusCode, Json<TableSnapshot>)> {
    Ok((
        StatusCode::CREATED,
        Json(visible_snapshot(store.create_table(request)?, None)),
    ))
}

async fn get_table(
    State(store): State<TableStore>,
    Path(table_id): Path<String>,
) -> ApiResult<Json<TableView>> {
    Ok(Json(store.table_view(store.snapshot(&table_id)?, None)?))
}

async fn view_table(
    State(store): State<TableStore>,
    Path(table_id): Path<String>,
    Json(request): Json<ViewRequest>,
) -> ApiResult<Json<TableView>> {
    let viewer = store.viewer(&table_id, &request.viewer_token)?;
    Ok(Json(
        store.table_view(store.snapshot(&table_id)?, Some(&viewer))?,
    ))
}

async fn upgrade_table(
    State(store): State<TableStore>,
    Path(table_id): Path<String>,
) -> ApiResult<Json<TableView>> {
    Ok(Json(store.upgrade_table(&table_id)?))
}

async fn join_table(
    State(store): State<TableStore>,
    Path(table_id): Path<String>,
    Json(request): Json<JoinViewerRequest>,
) -> ApiResult<Json<JoinedTable>> {
    Ok(Json(store.join_viewer(&table_id, request)?))
}

async fn seat_player(
    State(store): State<TableStore>,
    Path(table_id): Path<String>,
    Json(input): Json<ViewerRequest<SeatPlayerRequest>>,
) -> ApiResult<Json<TableSnapshot>> {
    let viewer = store.require_player(&table_id, &input.viewer_token, &input.request.player_id)?;
    Ok(Json(visible_snapshot(
        store.seat_player(&table_id, input.request)?,
        Some(&viewer),
    )))
}

async fn leave_table(
    State(store): State<TableStore>,
    Path(table_id): Path<String>,
    Json(input): Json<ViewerRequest<LeaveTableRequest>>,
) -> ApiResult<Json<TableSnapshot>> {
    let viewer = store.require_player(&table_id, &input.viewer_token, &input.request.player_id)?;
    Ok(Json(visible_snapshot(
        store.leave_table(&table_id, input.request)?,
        Some(&viewer),
    )))
}

async fn start_hand(
    State(store): State<TableStore>,
    Path(table_id): Path<String>,
    Json(request): Json<ViewRequest>,
) -> ApiResult<Json<TableSnapshot>> {
    let viewer = store.viewer(&table_id, &request.viewer_token)?;
    Ok(Json(visible_snapshot(
        store.start_hand(&table_id)?,
        Some(&viewer),
    )))
}

async fn action(
    State(store): State<TableStore>,
    Path(table_id): Path<String>,
    Json(input): Json<ViewerRequest<ActionRequest>>,
) -> ApiResult<Json<ActionResult>> {
    let viewer = store.require_player(&table_id, &input.viewer_token, &input.request.player_id)?;
    let mut result = store.act(&table_id, input.request)?;
    result.snapshot = visible_snapshot(result.snapshot, Some(&viewer));
    Ok(Json(result))
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
            Self::Unauthorized => StatusCode::FORBIDDEN,
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

    #[test]
    fn valid_viewer_resume_preserves_player_and_snapshot_without_rejoining() {
        let (store, table_id) = create_store();
        let joined = store
            .join_viewer(
                &table_id,
                JoinViewerRequest {
                    viewer_token: None,
                    display_name: "Alice".into(),
                    stack: Some(100),
                },
            )
            .unwrap();
        let before = store.snapshot(&table_id).unwrap();
        let resumed = store
            .join_viewer(
                &table_id,
                JoinViewerRequest {
                    viewer_token: Some(joined.viewer_token.clone()),
                    display_name: "Changed".into(),
                    stack: Some(500),
                },
            )
            .unwrap();
        assert_eq!(resumed.player_id, joined.player_id);
        assert_eq!(resumed.viewer_token, joined.viewer_token);
        assert_eq!(store.snapshot(&table_id).unwrap(), before);
        assert_eq!(resumed.snapshot.players.len(), 1);
    }

    #[test]
    fn legacy_active_tables_are_public_history_and_fresh_tables_preserve_their_state() {
        let (store, table_id) = create_store();
        seat_two(&store, &table_id);
        let before = store.snapshot(&table_id).unwrap();
        let public = store.table_view(before.clone(), None).unwrap();
        assert!(public.viewer_upgrade_required);
        assert!(public
            .snapshot
            .players
            .iter()
            .all(|player| player.hole_cards.is_empty()));
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let shared = store.clone();
                let table_id = table_id.clone();
                std::thread::spawn(move || shared.upgrade_table(&table_id).unwrap().snapshot)
            })
            .collect();
        let mut fresh_tables: Vec<_> = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect();
        let fresh = fresh_tables.pop().unwrap();
        assert_eq!(fresh.table_id, fresh_tables[0].table_id);
        assert_eq!(store.list_tables().unwrap().len(), 2);
        assert_ne!(fresh.table_id, table_id);
        assert_eq!(fresh.phase, TablePhase::Waiting);
        assert!(
            !store
                .table_view(fresh, None)
                .unwrap()
                .viewer_upgrade_required
        );
        assert_eq!(store.snapshot(&table_id).unwrap(), before);
    }

    #[test]
    fn production_shuffle_uses_fresh_entropy_and_preserves_the_complete_deck() {
        let first = shuffled_deck();
        let second = shuffled_deck();
        assert_eq!(first.len(), 52);
        assert_eq!(first.iter().cloned().collect::<BTreeSet<_>>().len(), 52);
        assert_ne!(first, second);
    }

    #[test]
    fn viewer_capabilities_bind_private_cards_to_server_identity_and_table() {
        let (store, table_id) = create_store();
        let alice = store
            .join_viewer(
                &table_id,
                JoinViewerRequest {
                    viewer_token: None,
                    display_name: "Alice".into(),
                    stack: Some(100),
                },
            )
            .unwrap();
        let bob = store
            .join_viewer(
                &table_id,
                JoinViewerRequest {
                    viewer_token: None,
                    display_name: "Bob".into(),
                    stack: Some(100),
                },
            )
            .unwrap();
        let mut events = store.subscribe().unwrap();
        for (player, seat) in [(&alice.player_id, 0), (&bob.player_id, 1)] {
            store
                .seat_player(
                    &table_id,
                    SeatPlayerRequest {
                        player_id: player.clone(),
                        seat,
                    },
                )
                .unwrap();
        }
        let dealt = store.snapshot(&table_id).unwrap();
        assert!(dealt
            .players
            .iter()
            .all(|player| player.hole_cards.len() == 2));
        assert!(visible_snapshot(dealt.clone(), None)
            .players
            .iter()
            .all(|player| player.hole_cards.is_empty()));
        let viewer = store.viewer(&table_id, &alice.viewer_token).unwrap();
        let private = visible_snapshot(dealt, Some(&viewer));
        assert_eq!(
            private
                .players
                .iter()
                .find(|player| player.player_id == alice.player_id)
                .unwrap()
                .hole_cards
                .len(),
            2
        );
        assert!(private
            .players
            .iter()
            .find(|player| player.player_id == bob.player_id)
            .unwrap()
            .hole_cards
            .is_empty());
        assert!(store
            .require_player(&table_id, &alice.viewer_token, &bob.player_id)
            .is_err());
        let (_, other_table) = create_store();
        assert!(store.viewer(&other_table, &alice.viewer_token).is_err());
        assert!(store.viewer(&table_id, &"f".repeat(64)).is_err());
        let mut count = 0;
        while let Ok(event) = events.try_recv() {
            assert!(event
                .snapshot
                .players
                .iter()
                .all(|player| player.hole_cards.is_empty()));
            count += 1;
        }
        assert!(count >= 2);
    }

    #[test]
    fn showdown_reveals_contenders_but_preserves_folded_and_uncontested_privacy() {
        let (store, table_id) = create_store();
        seat_two(&store, &table_id);
        let mut snapshot = store.snapshot(&table_id).unwrap();
        snapshot.players[0].folded = true;
        snapshot.phase = TablePhase::Complete;
        snapshot.winners = vec![Winner {
            player_id: "bob".into(),
            payout: 15,
            hand_rank: vec![],
        }];
        assert!(visible_snapshot(snapshot.clone(), None)
            .players
            .iter()
            .all(|player| player.hole_cards.is_empty()));
        snapshot.winners[0].hand_rank = vec![1, 14];
        let visible = visible_snapshot(snapshot, None);
        assert!(visible.players[0].hole_cards.is_empty());
        assert_eq!(visible.players[1].hole_cards.len(), 2);
    }

    #[test]
    fn viewer_credentials_survive_database_reopen_and_cannot_claim_legacy_players() {
        let path =
            std::env::temp_dir().join(format!("token-table-viewer-{}.sqlite", Uuid::new_v4()));
        let store = TableStore::open(&path).unwrap();
        let table = store
            .create_table(CreateTableRequest {
                name: "Persistence".into(),
                max_seats: None,
                small_blind: None,
                big_blind: None,
                starting_stack: None,
            })
            .unwrap();
        store
            .join_table(
                &table.table_id,
                JoinTableRequest {
                    player_id: "legacy-alice".into(),
                    display_name: "Alice".into(),
                    stack: None,
                },
            )
            .unwrap();
        let joined = store
            .join_viewer(
                &table.table_id,
                JoinViewerRequest {
                    viewer_token: None,
                    display_name: "Viewer".into(),
                    stack: None,
                },
            )
            .unwrap();
        assert_ne!(joined.player_id, "legacy-alice");
        store
            .seat_player(
                &table.table_id,
                SeatPlayerRequest {
                    player_id: "legacy-alice".into(),
                    seat: 0,
                },
            )
            .unwrap();
        let original = store.snapshot(&table.table_id).unwrap();
        let replacement = store.upgrade_table(&table.table_id).unwrap().snapshot;
        drop(store);
        let reopened = TableStore::open(&path).unwrap();
        assert_eq!(
            reopened
                .viewer(&table.table_id, &joined.viewer_token)
                .unwrap(),
            joined.player_id
        );
        assert_eq!(
            reopened
                .upgrade_table(&table.table_id)
                .unwrap()
                .snapshot
                .table_id,
            replacement.table_id
        );
        assert_eq!(reopened.snapshot(&table.table_id).unwrap(), original);
        drop(reopened);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn viewer_tokens_are_stored_as_digests_and_legacy_rows_migrate_once() {
        let path =
            std::env::temp_dir().join(format!("ryu-token-table-digest-{}.db", Uuid::new_v4()));
        let store = TableStore::open(&path).unwrap();
        let table = store
            .create_table(CreateTableRequest {
                name: "Digest".into(),
                max_seats: None,
                small_blind: None,
                big_blind: None,
                starting_stack: None,
            })
            .unwrap();
        let joined = store
            .join_viewer(
                &table.table_id,
                JoinViewerRequest {
                    viewer_token: None,
                    display_name: "Viewer".into(),
                    stack: None,
                },
            )
            .unwrap();
        drop(store);
        let raw = |path: &std::path::Path| -> Vec<String> {
            let connection = Connection::open(path).unwrap();
            let mut statement = connection
                .prepare("SELECT viewer_token FROM viewer_credentials")
                .unwrap();
            let rows = statement
                .query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            rows
        };
        assert_eq!(raw(&path), vec![viewer_token_digest(&joined.viewer_token)]);

        // Simulate a database an older version wrote: the raw bearer, version 0.
        {
            let connection = Connection::open(&path).unwrap();
            connection
                .execute(
                    "UPDATE viewer_credentials SET viewer_token = ?1",
                    params![joined.viewer_token],
                )
                .unwrap();
            connection.execute_batch("PRAGMA user_version = 0").unwrap();
        }
        let reopened = TableStore::open(&path).unwrap();
        assert_eq!(
            reopened
                .viewer(&table.table_id, &joined.viewer_token)
                .unwrap(),
            joined.player_id
        );
        // The digest itself is not a credential.
        assert!(reopened
            .viewer(&table.table_id, &viewer_token_digest(&joined.viewer_token))
            .is_err());
        drop(reopened);
        // Migrated once: reopening does not hash the digest again.
        let again = TableStore::open(&path).unwrap();
        assert!(again.viewer(&table.table_id, &joined.viewer_token).is_ok());
        drop(again);
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
            .unwrap();
        drop(connection);
        let bytes = std::fs::read(&path).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains(&joined.viewer_token));
        std::fs::remove_file(path).unwrap();
    }

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

    /// Replace the live hand's OsRng deal with fixed hole cards and board.
    /// The production shuffle can legitimately split a checked-down pot, so a
    /// test that asserts one winner must not depend on whichever deck it drew.
    fn rig_live_hand(
        store: &TableStore,
        table_id: &str,
        hole_cards: &[(&str, [Card; 2])],
        board: [Card; 5],
    ) {
        let connection = store.connection.lock().expect("store lock");
        let mut state = load_state(&connection, table_id).expect("live hand");
        let mut dealt = board.to_vec();
        for (player_id, cards) in hole_cards {
            let player = state
                .snapshot
                .players
                .iter_mut()
                .find(|player| player.player_id == *player_id)
                .expect("seated player");
            player.hole_cards = cards.to_vec();
            dealt.extend_from_slice(cards);
        }
        let mut deck: Vec<Card> = ordered_deck()
            .into_iter()
            .filter(|card| !dealt.contains(card))
            .collect();
        // deal_board pops from the end, so the flop card dealt first goes last.
        deck.extend(board.iter().rev().cloned());
        state.deck = deck;
        save_state(&connection, &state).expect("rigged hand");
    }

    #[test]
    fn checking_through_all_streets_reaches_showdown_and_keeps_chips_conserved() {
        let (store, table_id) = create_store();
        seat_two(&store, &table_id);
        rig_live_hand(
            &store,
            &table_id,
            &[
                ("alice", [card(14, 's'), card(14, 'h')]),
                ("bob", [card(7, 'c'), card(2, 'd')]),
            ],
            [
                card(13, 'd'),
                card(9, 'c'),
                card(4, 'h'),
                card(3, 's'),
                card(8, 'd'),
            ],
        );
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
                assert_eq!(
                    snapshot.board,
                    vec![
                        card(13, 'd'),
                        card(9, 'c'),
                        card(4, 'h'),
                        card(3, 's'),
                        card(8, 'd'),
                    ]
                );
                assert_eq!(snapshot.winners.len(), 1);
                assert_eq!(snapshot.winners[0].player_id, "alice");
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

    /// Seat players with explicit stacks (0 = busted) and start a hand
    /// without the two-player auto-start locking out later seats.
    fn rig_table(store: &TableStore, table_id: &str, players: &[(&str, u8, u64)]) {
        for (player_id, _, _) in players {
            store
                .join_table(
                    table_id,
                    JoinTableRequest {
                        player_id: (*player_id).into(),
                        display_name: (*player_id).into(),
                        stack: Some(1),
                    },
                )
                .expect("join");
        }
        let connection = store.connection.lock().expect("store lock");
        let mut state = load_state(&connection, table_id).expect("waiting table");
        for (player_id, seat, stack) in players {
            let player = state
                .snapshot
                .players
                .iter_mut()
                .find(|player| player.player_id == *player_id)
                .expect("joined player");
            player.seat = Some(*seat);
            player.stack = *stack;
        }
        start_hand_state(&mut state).expect("hand starts");
        save_state(&connection, &state).expect("rigged table");
    }

    fn act_now(store: &TableStore, table_id: &str, action: PlayerAction, amount: Option<u64>) {
        let snapshot = store.snapshot(table_id).unwrap();
        let player_id = snapshot.current_turn.clone().expect("a player must act");
        store
            .act(
                table_id,
                ActionRequest {
                    action_id: format!("action-{}", snapshot.action_seq),
                    player_id,
                    expected_action_seq: snapshot.action_seq,
                    action,
                    amount,
                },
            )
            .expect("legal action");
    }

    fn stack_of(snapshot: &TableSnapshot, player_id: &str) -> u64 {
        snapshot
            .players
            .iter()
            .find(|player| player.player_id == player_id)
            .map(|player| player.stack)
            .expect("player")
    }

    #[test]
    fn incomplete_all_in_raises_the_bet_so_the_big_blind_can_call() {
        let (store, table_id) = create_store();
        rig_table(&store, &table_id, &[("alice", 0, 15), ("bob", 1, 100)]);
        let snapshot = store.snapshot(&table_id).unwrap();
        assert_eq!(snapshot.current_turn.as_deref(), Some("alice"));
        // Small blind shoves 15 total: 5 more than the big blind, short of a full raise.
        act_now(&store, &table_id, PlayerAction::AllIn, None);
        let snapshot = store.snapshot(&table_id).unwrap();
        assert_eq!(snapshot.current_bet, 15);
        assert_eq!(snapshot.min_raise, 10);
        assert_eq!(snapshot.current_turn.as_deref(), Some("bob"));
        act_now(&store, &table_id, PlayerAction::Call, None);
        let snapshot = store.snapshot(&table_id).unwrap();
        assert_eq!(snapshot.phase, TablePhase::Flop);
        assert_eq!(snapshot.pot, 30);
        assert_eq!(stack_of(&snapshot, "bob"), 85);
    }

    fn try_act(
        store: &TableStore,
        table_id: &str,
        player_id: &str,
        action: PlayerAction,
        amount: Option<u64>,
    ) -> Result<ActionResult> {
        let snapshot = store.snapshot(table_id).unwrap();
        store.act(
            table_id,
            ActionRequest {
                action_id: format!("action-{}", snapshot.action_seq),
                player_id: player_id.into(),
                expected_action_seq: snapshot.action_seq,
                action,
                amount,
            },
        )
    }

    #[test]
    fn incomplete_all_in_preflop_does_not_reopen_action_for_players_who_called() {
        let (store, table_id) = create_store();
        // Button alice is first to act three-handed; bob (small blind) is short.
        rig_table(
            &store,
            &table_id,
            &[("alice", 0, 100), ("bob", 1, 15), ("carol", 2, 100)],
        );
        let snapshot = store.snapshot(&table_id).unwrap();
        assert_eq!(snapshot.current_turn.as_deref(), Some("alice"));
        act_now(&store, &table_id, PlayerAction::Call, None);
        // Small blind shoves 15 total: 5 over the big blind, short of a full raise.
        act_now(&store, &table_id, PlayerAction::AllIn, None);
        let snapshot = store.snapshot(&table_id).unwrap();
        assert_eq!(snapshot.current_bet, 15);
        assert_eq!(snapshot.current_turn.as_deref(), Some("carol"));
        // The big blind had not acted yet, so a full re-raise stays legal for her;
        // she only calls here.
        act_now(&store, &table_id, PlayerAction::Call, None);
        let snapshot = store.snapshot(&table_id).unwrap();
        assert_eq!(snapshot.phase, TablePhase::Preflop);
        assert_eq!(snapshot.current_turn.as_deref(), Some("alice"));
        // Alice already acted and faces only an incomplete raise: call or fold.
        let raise = try_act(&store, &table_id, "alice", PlayerAction::Raise, Some(30));
        assert!(matches!(raise, Err(GameError::Invalid(_))));
        let shove = try_act(&store, &table_id, "alice", PlayerAction::AllIn, None);
        assert!(matches!(shove, Err(GameError::Invalid(_))));
        act_now(&store, &table_id, PlayerAction::Call, None);
        let snapshot = store.snapshot(&table_id).unwrap();
        assert_eq!(snapshot.phase, TablePhase::Flop);
        assert_eq!(snapshot.pot, 45);
    }

    #[test]
    fn incomplete_all_in_on_a_street_only_asks_unmatched_players_to_call() {
        let (store, table_id) = create_store();
        rig_table(
            &store,
            &table_id,
            &[
                ("alice", 0, 1000),
                ("bob", 1, 1000),
                ("carol", 2, 1000),
                ("dave", 3, 1000),
            ],
        );
        // Limp/check preflop so everyone reaches the flop with 10 committed.
        for _ in 0..3 {
            act_now(&store, &table_id, PlayerAction::Call, None);
        }
        act_now(&store, &table_id, PlayerAction::Check, None);
        let snapshot = store.snapshot(&table_id).unwrap();
        assert_eq!(snapshot.phase, TablePhase::Flop);
        let first = snapshot.current_turn.clone().expect("flop actor");
        let order: Vec<String> = {
            let mut seats: Vec<_> = snapshot
                .players
                .iter()
                .map(|player| (player.seat.unwrap(), player.player_id.clone()))
                .collect();
            seats.sort_unstable();
            let start = seats.iter().position(|(_, id)| *id == first).unwrap();
            (0..seats.len())
                .map(|offset| seats[(start + offset) % seats.len()].1.clone())
                .collect()
        };
        // The third player to act has exactly 150 behind for the short shove.
        {
            let connection = store.connection.lock().expect("store lock");
            let mut state = load_state(&connection, &table_id).expect("live table");
            let short = state
                .snapshot
                .players
                .iter_mut()
                .find(|player| player.player_id == order[2])
                .unwrap();
            short.stack = 150;
            save_state(&connection, &state).expect("rigged stack");
        }
        act_now(&store, &table_id, PlayerAction::Bet, Some(100));
        act_now(&store, &table_id, PlayerAction::Call, None);
        act_now(&store, &table_id, PlayerAction::AllIn, None);
        let snapshot = store.snapshot(&table_id).unwrap();
        assert_eq!(snapshot.current_bet, 150);
        assert_eq!(snapshot.min_raise, 100);
        assert_eq!(snapshot.current_turn.as_deref(), Some(order[3].as_str()));
        act_now(&store, &table_id, PlayerAction::Call, None);
        // The player who just called the shove does not act again; the
        // bettor and the first caller only owe the extra 50.
        let snapshot = store.snapshot(&table_id).unwrap();
        assert_eq!(snapshot.phase, TablePhase::Flop);
        assert_eq!(snapshot.current_turn.as_deref(), Some(order[0].as_str()));
        let raise = try_act(&store, &table_id, &order[0], PlayerAction::Raise, Some(300));
        assert!(matches!(raise, Err(GameError::Invalid(_))));
        act_now(&store, &table_id, PlayerAction::Call, None);
        assert_eq!(
            store.snapshot(&table_id).unwrap().current_turn.as_deref(),
            Some(order[1].as_str())
        );
        act_now(&store, &table_id, PlayerAction::Call, None);
        let snapshot = store.snapshot(&table_id).unwrap();
        assert_eq!(snapshot.phase, TablePhase::Turn);
        assert_eq!(snapshot.pot, 40 + 600);
    }

    #[test]
    fn short_all_in_wins_only_the_main_pot_and_busted_seats_are_not_dealt_in() {
        let (store, table_id) = create_store();
        rig_table(
            &store,
            &table_id,
            &[
                ("alice", 0, 50),
                ("bob", 1, 1000),
                ("carol", 2, 1000),
                ("dave", 3, 0),
            ],
        );
        let snapshot = store.snapshot(&table_id).unwrap();
        let dave = snapshot
            .players
            .iter()
            .find(|player| player.player_id == "dave")
            .unwrap();
        assert!(!dave.in_hand);
        assert!(dave.hole_cards.is_empty());
        assert_eq!(snapshot.current_turn.as_deref(), Some("alice"));
        rig_live_hand(
            &store,
            &table_id,
            &[
                ("alice", [card(14, 's'), card(14, 'h')]),
                ("bob", [card(13, 's'), card(13, 'h')]),
                ("carol", [card(8, 'c'), card(6, 'd')]),
            ],
            [
                card(2, 'c'),
                card(7, 'd'),
                card(9, 'h'),
                card(11, 's'),
                card(3, 'd'),
            ],
        );
        act_now(&store, &table_id, PlayerAction::AllIn, None);
        act_now(&store, &table_id, PlayerAction::Call, None);
        act_now(&store, &table_id, PlayerAction::Call, None);
        assert_eq!(store.snapshot(&table_id).unwrap().phase, TablePhase::Flop);
        act_now(&store, &table_id, PlayerAction::Bet, Some(400));
        act_now(&store, &table_id, PlayerAction::Call, None);
        for _ in 0..4 {
            act_now(&store, &table_id, PlayerAction::Check, None);
        }
        let snapshot = store.snapshot(&table_id).unwrap();
        assert_eq!(snapshot.phase, TablePhase::Complete);
        assert_eq!(stack_of(&snapshot, "alice"), 150);
        assert_eq!(stack_of(&snapshot, "bob"), 1350);
        assert_eq!(stack_of(&snapshot, "carol"), 550);
        assert_eq!(stack_of(&snapshot, "dave"), 0);
        assert_eq!(snapshot.settled_pot, 950);
        let payouts: Vec<_> = snapshot
            .winners
            .iter()
            .map(|winner| (winner.player_id.as_str(), winner.payout))
            .collect();
        assert_eq!(payouts, vec![("alice", 150), ("bob", 800)]);
    }

    #[test]
    fn uncalled_part_of_an_all_in_returns_to_the_bettor() {
        let (store, table_id) = create_store();
        rig_table(&store, &table_id, &[("alice", 0, 100), ("bob", 1, 30)]);
        rig_live_hand(
            &store,
            &table_id,
            &[
                ("alice", [card(13, 's'), card(13, 'h')]),
                ("bob", [card(14, 's'), card(14, 'h')]),
            ],
            [
                card(2, 'c'),
                card(7, 'd'),
                card(9, 'h'),
                card(11, 's'),
                card(3, 'd'),
            ],
        );
        act_now(&store, &table_id, PlayerAction::AllIn, None);
        act_now(&store, &table_id, PlayerAction::Call, None);
        let snapshot = store.snapshot(&table_id).unwrap();
        assert_eq!(snapshot.phase, TablePhase::Complete);
        assert_eq!(stack_of(&snapshot, "alice"), 70);
        assert_eq!(stack_of(&snapshot, "bob"), 60);
        assert_eq!(snapshot.settled_pot, 60);
        assert_eq!(snapshot.winners.len(), 1);
        assert_eq!(snapshot.winners[0].player_id, "bob");
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
