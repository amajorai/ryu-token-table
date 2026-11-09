import { ExtensionHost } from "@ryu/app-host/ExtensionHost";
import {
	type AppRequestPayload,
	capabilitiesFromGrants,
	type HostServices,
	type RealtimeConnectionInfo,
	type RealtimeConnectionPayload,
	type RealtimeConnectPayload,
	type RealtimePublishPayload,
} from "@ryu/app-host/rpc";
import { htmlCompanionSrcdoc } from "@ryu/app-host/third-party-plugin";
import { useMemo } from "react";
import { createRoot } from "react-dom/client";
import manifest from "../../manifest.json" with { type: "json" };
import appHtml from "../dist/index.html?raw";
import {
	actionBodySchema,
	joinTableBodySchema,
	leaveTableBodySchema,
	seatPlayerBodySchema,
	type TokenTableAppReply,
	tokenTableJsonSchema,
	viewerBodySchema,
	wireSnapshotSchema,
} from "../src/schemas.ts";
import type {
	WireActionRecord,
	WireCard,
	WireTableSnapshot,
} from "../src/types.ts";
import type {
	HostCall,
	RealtimeCall,
	TokenTableFixture,
} from "./fixture-types.ts";
import styles from "./host-proof.module.css";
import "./host-proof.css";

const PLAYER_ID = "host-proof-player";
const TABLE_ID = "host-proof-table";
const VIEWER_TOKEN = "f".repeat(64);
const appCalls: HostCall[] = [];
const realtimeConnections: RealtimeConnectPayload[] = [];
const realtimePublishes: RealtimeCall[] = [];

function createWireCard(rank: number, suit: string): WireCard {
	return { rank, suit };
}

function createSnapshot(): WireTableSnapshot {
	return {
		action_seq: 1,
		big_blind: 10,
		big_blind_seat: 0,
		board: [
			createWireCard(14, "s"),
			createWireCard(7, "d"),
			createWireCard(13, "c"),
			createWireCard(2, "h"),
			createWireCard(9, "c"),
		],
		button_seat: 0,
		current_bet: 10,
		current_turn: null,
		hand_id: "host-proof-hand-7",
		hand_number: 7,
		last_action: null,
		max_seats: 6,
		min_raise: 10,
		name: "Local ExtensionHost fixture",
		phase: "preflop",
		players: [
			{
				committed: 5,
				display_name: "Mara",
				folded: false,
				hole_cards: [],
				in_hand: true,
				player_id: "host-proof-mara",
				seat: 0,
				stack: 195,
				street_committed: 5,
			},
			{
				committed: 10,
				display_name: "Noah",
				folded: false,
				hole_cards: [],
				in_hand: true,
				player_id: "host-proof-noah",
				seat: 2,
				stack: 190,
				street_committed: 10,
			},
		],
		pot: 15,
		settled_pot: 0,
		small_blind: 5,
		small_blind_seat: 1,
		starting_stack: 200,
		table_id: TABLE_ID,
	};
}

const snapshot = createSnapshot();

function recordCall(input: AppRequestPayload): void {
	appCalls.push(structuredClone(input));
}

function cloneSnapshot(): WireTableSnapshot {
	return structuredClone(snapshot);
}

function routeForTable(path: string, suffix: string): boolean {
	return path === `/tables/${encodeURIComponent(TABLE_ID)}/${suffix}`;
}

function finishAction(action: WireActionRecord): WireTableSnapshot {
	snapshot.action_seq += 1;
	snapshot.last_action = action;
	return cloneSnapshot();
}

async function appRequest(
	input: AppRequestPayload
): Promise<TokenTableAppReply> {
	recordCall(input);
	if (input.method === "GET" && input.path === "/tables") {
		return [cloneSnapshot()];
	}
	if (input.method === "GET" && input.path === `/tables/${TABLE_ID}`) {
		return cloneSnapshot();
	}
	if (input.method === "POST" && routeForTable(input.path, "view")) {
		const body = viewerBodySchema.parse(input.body);
		if (body.viewerToken !== VIEWER_TOKEN) {
			throw new Error("The local viewer capability is invalid.");
		}
		return cloneSnapshot();
	}
	if (input.method === "POST" && routeForTable(input.path, "join")) {
		const body = joinTableBodySchema.parse(input.body);
		const playerId = PLAYER_ID;
		if (!snapshot.players.some((player) => player.player_id === playerId)) {
			snapshot.players.push({
				committed: 0,
				display_name: body.display_name,
				folded: false,
				hole_cards: [],
				in_hand: true,
				player_id: playerId,
				seat: null,
				stack: body.stack,
				street_committed: 0,
			});
		}
		return { snapshot: cloneSnapshot(), playerId, viewerToken: VIEWER_TOKEN };
	}
	if (input.method === "POST" && routeForTable(input.path, "seat")) {
		const body = seatPlayerBodySchema.parse(input.body);
		const player = snapshot.players.find(
			(candidate) => candidate.player_id === body.player_id
		);
		if (!player) {
			throw new Error("Seat fixture received a player that did not join.");
		}
		player.seat = body.seat;
		snapshot.current_turn = player.player_id;
		return cloneSnapshot();
	}
	if (input.method === "POST" && routeForTable(input.path, "action")) {
		const body = actionBodySchema.parse(input.body);
		const playerId = body.player_id;
		const player = snapshot.players.find(
			(candidate) => candidate.player_id === playerId
		);
		if (!player || snapshot.current_turn !== playerId) {
			throw new Error("Action fixture rejected a player out of turn.");
		}
		const action = body.action;
		const amount =
			action === "call"
				? Math.min(snapshot.current_bet - player.street_committed, player.stack)
				: action === "fold"
					? 0
					: Math.max(0, (body.amount ?? 0) - player.street_committed);
		player.committed += amount;
		player.street_committed += amount;
		player.stack -= amount;
		snapshot.pot += amount;
		if (action === "fold") {
			player.folded = true;
			snapshot.phase = "complete";
			snapshot.current_turn = null;
		} else {
			snapshot.current_turn = "host-proof-mara";
		}
		return finishAction({
			action,
			action_id: body.action_id,
			action_seq: snapshot.action_seq,
			amount,
			player_id: playerId,
		});
	}
	if (input.method === "POST" && routeForTable(input.path, "leave")) {
		const body = leaveTableBodySchema.parse(input.body);
		snapshot.players = snapshot.players.filter(
			(player) => player.player_id !== body.player_id
		);
		snapshot.current_turn = "host-proof-mara";
		return cloneSnapshot();
	}
	if (input.method === "POST" && routeForTable(input.path, "start")) {
		snapshot.hand_number += 1;
		snapshot.hand_id = `host-proof-hand-${snapshot.hand_number}`;
		snapshot.phase = "preflop";
		snapshot.board = [];
		snapshot.pot = 15;
		snapshot.current_bet = 10;
		snapshot.current_turn = PLAYER_ID;
		for (const player of snapshot.players) {
			player.folded = false;
			player.in_hand = true;
			player.committed = player.player_id === PLAYER_ID ? 10 : 5;
			player.street_committed = player.player_id === PLAYER_ID ? 10 : 5;
		}
		return cloneSnapshot();
	}
	throw new Error(
		`The local Token Table fixture has no route for ${input.path}.`
	);
}

const services: HostServices = {
	listAgents: async () => [],
	registerRoute: async (claim) => ({ path: claim.path }),
	appRequest: async (input) =>
		tokenTableJsonSchema.parse(await appRequest(input)),
	realtimeConnect: async (
		input: RealtimeConnectPayload
	): Promise<RealtimeConnectionInfo> => {
		realtimeConnections.push(structuredClone(input));
		return {
			access: "write",
			connection_id: "host-proof-connection",
			member_id: PLAYER_ID,
			presence: [],
			room_id: input.room_id,
		};
	},
	realtimePublish: async (input: RealtimePublishPayload) => {
		realtimePublishes.push({
			event: input.event,
			input: {
				connection_id: input.connection_id,
				data: wireSnapshotSchema.parse(input.data),
				event: input.event,
			},
		});
	},
	realtimeSubscribe: async (
		_input: RealtimeConnectionPayload,
		_emit: (delta: string) => void,
		signal: AbortSignal
	) => {
		if (signal.aborted) {
			return;
		}
		await new Promise<void>((resolve) => {
			signal.addEventListener("abort", () => resolve(), { once: true });
		});
	},
	realtimeClose: async (_input: RealtimeConnectionPayload) => undefined,
};

const fixture: TokenTableFixture = {
	appCalls,
	realtimeConnections,
	realtimePublishes,
	getSnapshot: cloneSnapshot,
};
Object.assign(window, { __tokenTableFixture: fixture });

function ProductHost() {
	const nonce = useMemo(() => crypto.randomUUID(), []);
	const isDemo =
		new URLSearchParams(window.location.search).get("mode") === "demo";
	const srcdoc = useMemo(
		() =>
			htmlCompanionSrcdoc(nonce, appHtml, manifest.id, {
				playerId: PLAYER_ID,
				roomId: `token-table:${TABLE_ID}`,
				tableId: TABLE_ID,
			}),
		[nonce]
	);
	const grants = useMemo(
		() => capabilitiesFromGrants(manifest.permission_grants),
		[]
	);

	if (isDemo) {
		return (
			<main className={styles["token-table-proof-frame"]}>
				<iframe
					allow="clipboard-read; clipboard-write"
					className={styles["token-table-demo-preview"]}
					srcDoc={appHtml}
					title="Token Table demo preview"
				/>
			</main>
		);
	}

	return (
		<main className={styles["token-table-proof-frame"]}>
			<ExtensionHost
				granted={grants}
				nonce={nonce}
				services={services}
				srcdoc={srcdoc}
				title="Token Table ExtensionHost proof"
			/>
		</main>
	);
}

const root = document.getElementById("root");
if (root) {
	createRoot(root).render(<ProductHost />);
}
