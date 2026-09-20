import {
	applyDemoAction,
	applyDemoJoin,
	applyDemoLeave,
	DEFAULT_SNAPSHOT,
	startDemoHand,
} from "./model.ts";
import type {
	RyuApp,
	RyuTokenTable,
	RyuTokenTableConnection,
	RyuTokenTableEvent,
} from "./ryu.d.ts";
import type {
	TokenTableRequest,
	TokenTableResponse,
	TokenTableSnapshot,
	TokenTableSubscription,
	WireActionRecord,
	WireCard,
	WirePlayerSnapshot,
	WireTableSnapshot,
} from "./types.ts";

export interface TokenTableClient {
	mode: "host" | "demo";
	request(request: TokenTableRequest): Promise<TokenTableResponse>;
	subscribe(
		listener: (snapshot: TokenTableSnapshot) => void
	): TokenTableSubscription;
}

const DEFAULT_TABLE_NAME = "Studio room";
let localActionNumber = 0;

function createHostPlayerId(): string {
	if (
		typeof crypto !== "undefined" &&
		typeof crypto.randomUUID === "function"
	) {
		return `player-${crypto.randomUUID()}`;
	}
	return `player-${Math.random().toString(36).slice(2, 10)}`;
}

function hostSurfaces(): { app: RyuApp; realtime: RyuTokenTable } | null {
	if (typeof window === "undefined") {
		return null;
	}
	const app = window.ryu?.app;
	const realtime = window.ryu?.tokenTable;
	return app && realtime ? { app, realtime } : null;
}

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null;
}

function asWireSnapshot(value: unknown): WireTableSnapshot | null {
	if (!isRecord(value)) {
		return null;
	}
	const nested = isRecord(value.snapshot) ? value.snapshot : value;
	if (typeof nested.table_id !== "string" || !Array.isArray(nested.players)) {
		return null;
	}
	return nested as unknown as WireTableSnapshot;
}

function asWireList(value: unknown): WireTableSnapshot[] {
	if (Array.isArray(value)) {
		return value.flatMap((item) => {
			const snapshot = asWireSnapshot(item);
			return snapshot ? [snapshot] : [];
		});
	}
	if (isRecord(value) && Array.isArray(value.tables)) {
		return asWireList(value.tables);
	}
	return [];
}

function cardLabel(card: WireCard): string {
	const ranks: Record<number, string> = {
		2: "2",
		3: "3",
		4: "4",
		5: "5",
		6: "6",
		7: "7",
		8: "8",
		9: "9",
		10: "10",
		11: "J",
		12: "Q",
		13: "K",
		14: "A",
	};
	const suits: Record<string, string> = { c: "♣", d: "♦", h: "♥", s: "♠" };
	return `${ranks[card.rank] ?? card.rank}${suits[card.suit] ?? card.suit}`;
}

function cardPair(player: WirePlayerSnapshot): [string, string] | null {
	if (player.hole_cards.length < 2) {
		return null;
	}
	const [first, second] = player.hole_cards;
	if (!(first && second)) {
		return null;
	}
	return [cardLabel(first), cardLabel(second)];
}

function playerForSeat(
	snapshot: WireTableSnapshot,
	seat: number
): WirePlayerSnapshot | null {
	return snapshot.players.find((player) => player.seat === seat) ?? null;
}

function actionLabel(
	action: WireActionRecord | null,
	playerId: string
): string {
	if (!action) {
		return "Waiting for the next action";
	}
	const actor = action.player_id === playerId ? "You" : action.player_id;
	return `${actor} ${action.action.replace("_", " ")}${action.amount ? ` ${action.amount}` : ""}`;
}

function toUiSnapshot(
	wire: WireTableSnapshot,
	connection: TokenTableSnapshot["connection"],
	playerId: string
): TokenTableSnapshot {
	const phase = wire.phase === "complete" ? "showdown" : wire.phase;
	const own =
		wire.players.find((player) => player.player_id === playerId) ?? null;
	const currentTurn = wire.players.find(
		(player) => player.player_id === wire.current_turn
	);
	const players = DEFAULT_SNAPSHOT.players.map((fallback, index) => {
		const wirePlayer = playerForSeat(wire, index);
		if (!wirePlayer) {
			return {
				...fallback,
				cards: null,
				id: `empty-${index}`,
				name: "Open seat",
				seat: index + 1,
				stack: 0,
				status: "empty" as const,
			};
		}
		return {
			...fallback,
			avatar:
				wirePlayer.display_name.slice(0, 1).toUpperCase() || fallback.avatar,
			bet: wirePlayer.street_committed,
			cards: wirePlayer.player_id === playerId ? cardPair(wirePlayer) : null,
			id: wirePlayer.player_id === playerId ? "you" : wirePlayer.player_id,
			name: wirePlayer.display_name,
			seat: index + 1,
			stack: wirePlayer.stack,
			status: wirePlayer.folded
				? ("folded" as const)
				: wire.current_turn === wirePlayer.player_id
					? ("thinking" as const)
					: ("active" as const),
		};
	});
	const ownSeat =
		own?.seat === null || own?.seat === undefined ? null : own.seat + 1;
	const toCall = own ? Math.max(0, wire.current_bet - own.street_committed) : 0;
	return {
		activeSeat:
			currentTurn?.seat === null || currentTurn?.seat === undefined
				? 1
				: currentTurn.seat + 1,
		board: wire.board.map(cardLabel),
		connection,
		handId: wire.hand_id ?? `${wire.table_id}-${wire.hand_number}`,
		handNumber: wire.hand_number,
		history: wire.last_action
			? [
					{
						action: wire.last_action.action.replace("_", " "),
						actor:
							wire.last_action.player_id === playerId
								? "You"
								: wire.last_action.player_id,
						amount: wire.last_action.amount ?? undefined,
						accent: wire.last_action.player_id === playerId,
					},
				]
			: [],
		lastAction: actionLabel(wire.last_action, playerId),
		phase: phase as TokenTableSnapshot["phase"],
		players,
		pot: wire.pot,
		bigBlind: wire.big_blind,
		smallBlind: wire.small_blind,
		serverTime: "live now",
		toCall,
		you: {
			balance: (own?.stack ?? 0) + (own?.committed ?? 0),
			seat: ownSeat,
			status: ownSeat === null ? "waiting" : "seated",
		},
	};
}

function nextActionId(): string {
	localActionNumber += 1;
	if (
		typeof crypto !== "undefined" &&
		typeof crypto.randomUUID === "function"
	) {
		return crypto.randomUUID();
	}
	return `ui-action-${localActionNumber}`;
}

function createDemoClient(): TokenTableClient {
	let snapshot = structuredClone(DEFAULT_SNAPSHOT);
	const listeners = new Set<(next: TokenTableSnapshot) => void>();
	const publish = (next: TokenTableSnapshot) => {
		snapshot = next;
		for (const listener of listeners) {
			listener(snapshot);
		}
	};
	return {
		mode: "demo",
		request: async (request) => {
			switch (request.type) {
				case "snapshot":
					return { ok: true, snapshot };
				case "action":
					publish(
						applyDemoAction(snapshot, request.action, request.amount ?? 12)
					);
					return { ok: true, snapshot };
				case "join":
					publish(applyDemoJoin(snapshot, request.buyIn));
					return { ok: true, snapshot };
				case "leave":
					publish(applyDemoLeave(snapshot));
					return { ok: true, snapshot };
				case "new-hand":
					publish(startDemoHand(snapshot));
					return { ok: true, snapshot };
			}
		},
		subscribe: (listener) => {
			listeners.add(listener);
			return { dispose: () => listeners.delete(listener) };
		},
	};
}

function createHostClient(surfaces: {
	app: RyuApp;
	realtime: RyuTokenTable;
}): TokenTableClient {
	const context = typeof window === "undefined" ? null : window.ryu?.context;
	const playerId = context?.playerId ?? createHostPlayerId();
	let tableId = context?.tableId ?? null;
	let currentWire: WireTableSnapshot | null = null;
	let currentSnapshot: TokenTableSnapshot | null = null;
	let connection: RyuTokenTableConnection | null = null;
	let connecting: Promise<void> | null = null;
	let closing = false;
	const listeners = new Set<(next: TokenTableSnapshot) => void>();
	const send = (method: "GET" | "POST", path: string, body?: unknown) =>
		surfaces.app.request(
			body === undefined ? { method, path } : { body, method, path }
		);
	const publish = (next: TokenTableSnapshot) => {
		currentSnapshot = next;
		for (const listener of listeners) {
			listener(next);
		}
	};
	const acceptWire = (wire: WireTableSnapshot) => {
		tableId = wire.table_id;
		currentWire = wire;
		const next = toUiSnapshot(wire, "connected", playerId);
		publish(next);
		return next;
	};
	const ensureWireSnapshot = async (): Promise<WireTableSnapshot> => {
		if (currentWire && (!tableId || currentWire.table_id === tableId)) {
			return currentWire;
		}
		if (tableId) {
			const wire = asWireSnapshot(
				await send("GET", `/tables/${encodeURIComponent(tableId)}`)
			);
			if (wire) {
				return wire;
			}
		}
		const tables = asWireList(await send("GET", "/tables"));
		if (tables[0]) {
			return tables[0];
		}
		const created = asWireSnapshot(
			await send("POST", "/tables", {
				big_blind: 10,
				max_seats: 6,
				name: DEFAULT_TABLE_NAME,
				small_blind: 5,
				starting_stack: 1000,
			})
		);
		if (!created) {
			throw new Error("Token Table did not return a table snapshot.");
		}
		return created;
	};
	const connectRealtime = async () => {
		if (connection || closing) {
			return;
		}
		if (connecting) {
			return connecting;
		}
		connecting = (async () => {
			const wire = await ensureWireSnapshot();
			connection = await surfaces.realtime.connect(
				{ roomId: context?.roomId ?? `token-table:${wire.table_id}` },
				{
					onClose: () => {
						if (currentSnapshot) {
							publish({ ...currentSnapshot, connection: "reconnecting" });
						}
					},
					onEvent: (event: RyuTokenTableEvent) => {
						const nextWire = asWireSnapshot(event.data);
						if (nextWire) {
							acceptWire(nextWire);
						}
					},
				}
			);
		})().finally(() => {
			connecting = null;
		});
		return connecting;
	};
	const request = async (
		requestInput: TokenTableRequest
	): Promise<TokenTableResponse> => {
		try {
			if (requestInput.type === "snapshot") {
				const next = acceptWire(await ensureWireSnapshot());
				void connectRealtime().catch(() => undefined);
				return { ok: true, snapshot: next };
			}
			const wire = await ensureWireSnapshot();
			await connectRealtime().catch(() => undefined);
			const encodedTableId = encodeURIComponent(wire.table_id);
			let result: unknown;
			switch (requestInput.type) {
				case "action": {
					const own = wire.players.find(
						(player) => player.player_id === playerId
					);
					const totalCommitment =
						requestInput.action === "bet" || requestInput.action === "raise"
							? (own?.street_committed ?? 0) + (requestInput.amount ?? 0)
							: undefined;
					result = await send("POST", `/tables/${encodedTableId}/action`, {
						action:
							requestInput.action === "all-in" ? "all_in" : requestInput.action,
						action_id: nextActionId(),
						amount: totalCommitment,
						expected_action_seq: wire.action_seq,
						player_id: playerId,
					});
					break;
				}
				case "join": {
					await send("POST", `/tables/${encodedTableId}/join`, {
						display_name: "You",
						player_id: playerId,
						stack: requestInput.buyIn,
					});
					const openSeat =
						[0, 1, 2, 3, 4, 5].find(
							(seat) => !wire.players.some((player) => player.seat === seat)
						) ?? 3;
					result = await send("POST", `/tables/${encodedTableId}/seat`, {
						player_id: playerId,
						seat: openSeat,
					});
					break;
				}
				case "leave":
					result = await send("POST", `/tables/${encodedTableId}/leave`, {
						player_id: playerId,
					});
					break;
				case "new-hand":
					result = await send("POST", `/tables/${encodedTableId}/start`);
					break;
			}
			const nextWire = asWireSnapshot(result);
			if (!nextWire) {
				throw new Error("Token Table returned an invalid snapshot.");
			}
			if (connection) {
				await connection
					.publish("table.snapshot", nextWire)
					.catch(() => undefined);
			}
			return { ok: true, snapshot: acceptWire(nextWire) };
		} catch (error) {
			return {
				error:
					error instanceof Error
						? error.message
						: "Token Table request failed.",
				ok: false,
				snapshot: currentSnapshot ?? undefined,
			};
		}
	};
	return {
		mode: "host",
		request,
		subscribe: (listener) => {
			listeners.add(listener);
			void connectRealtime().catch(() => undefined);
			return {
				dispose: () => {
					listeners.delete(listener);
					if (listeners.size === 0 && connection) {
						closing = true;
						void connection.close();
						connection = null;
					}
				},
			};
		},
	};
}

export function createTokenTableClient(): TokenTableClient {
	const surfaces = hostSurfaces();
	if (surfaces) {
		return createHostClient(surfaces);
	}
	if (typeof window !== "undefined" && window.ryu) {
		return {
			mode: "host",
			request: async () => ({
				ok: false,
				error: "Token Table capabilities are unavailable on this host.",
			}),
			subscribe: () => ({ dispose: () => undefined }),
		};
	}
	return createDemoClient();
}
