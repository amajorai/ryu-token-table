import { afterEach, expect, test } from "bun:test";
import { appRequestPayloadSchema } from "@ryu/app-host/rpc";
import { createTokenTableClient } from "./bridge.ts";
import type { AppRequestInput, RyuTokenTableConnection } from "./ryu.d.ts";
import type { WireTableSnapshot } from "./types.ts";

const previous = Object.getOwnPropertyDescriptor(globalThis, "window");
afterEach(() => {
	if (previous) {
		Object.defineProperty(globalThis, "window", previous);
	} else {
		Reflect.deleteProperty(globalThis, "window");
	}
});

test("host identity comes from join capability, survives reload, and never enters shared publications", async () => {
	const token = "f".repeat(64);
	const storage = new Map<string, string>();
	const requests: AppRequestInput[] = [];
	const publications: Parameters<RyuTokenTableConnection["publish"]>[1][] = [];
	const snapshot: WireTableSnapshot = {
		table_id: "table-proof",
		name: "Table",
		max_seats: 6,
		small_blind: 5,
		big_blind: 10,
		starting_stack: 1000,
		phase: "preflop",
		hand_number: 1,
		hand_id: "hand-proof",
		action_seq: 1,
		button_seat: 0,
		small_blind_seat: 0,
		big_blind_seat: 1,
		current_turn: "server-viewer",
		current_bet: 10,
		min_raise: 10,
		pot: 15,
		settled_pot: 0,
		board: [],
		last_action: null,
		players: [
			{
				player_id: "server-viewer",
				display_name: "You",
				seat: 0,
				stack: 990,
				committed: 10,
				street_committed: 10,
				in_hand: true,
				folded: false,
				hole_cards: [
					{ rank: 14, suit: "h" },
					{ rank: 13, suit: "h" },
				],
			},
			{
				player_id: "opponent",
				display_name: "Other",
				seat: 1,
				stack: 995,
				committed: 5,
				street_committed: 5,
				in_hand: true,
				folded: false,
				hole_cards: [],
			},
		],
	};
	Object.defineProperty(globalThis, "window", {
		configurable: true,
		value: {
			sessionStorage: {
				getItem: (key: string) => storage.get(key) ?? null,
				setItem: (key: string, value: string) => storage.set(key, value),
			},
			ryu: {
				context: { tableId: "table-proof", playerId: "opponent" },
				app: {
					request: async (input: AppRequestInput) => {
						appRequestPayloadSchema.parse(input);
						requests.push(input);
						if (input.path.endsWith("/join")) {
							return {
								snapshot,
								playerId: "server-viewer",
								viewerToken: token,
							};
						}
						if (input.method === "GET") {
							return { ...snapshot, players: [] };
						}
						return snapshot;
					},
				},
				tokenTable: {
					connect: async () => ({
						access: "write",
						memberId: "member",
						roomId: "room",
						presence: [],
						close: async () => undefined,
						publish: async (
							_name: string,
							data: Parameters<RyuTokenTableConnection["publish"]>[1]
						) => {
							publications.push(data);
						},
					}),
				},
			},
		},
	});
	const client = createTokenTableClient();
	const joined = await client.request({ type: "join", buyIn: 1000 });
	expect(joined.ok).toBe(true);
	expect(joined.snapshot?.players[0]?.cards).toEqual(["A♥", "K♥"]);
	expect(joined.snapshot?.players[1]?.cards).toBeNull();
	expect(
		requests.find((request) => request.path.endsWith("/join"))?.body
	).toEqual({ display_name: "You", stack: 1000 });
	expect(JSON.stringify(publications)).not.toContain(token);
	expect(JSON.stringify(publications)).not.toContain('"rank":14');
	for (const action of [
		"call",
		"check",
		"fold",
		"all-in",
	] satisfies import("./types.ts").PokerAction[]) {
		const result = await client.request({ type: "action", action });
		expect(result.ok).toBe(true);
		const actionRequest = requests.at(-1);
		expect(actionRequest?.path.endsWith("/action")).toBe(true);
		expect(Object.hasOwn(actionRequest?.body ?? {}, "amount")).toBe(false);
	}
	const raised = await client.request({
		type: "action",
		action: "raise",
		amount: 40,
	});
	expect(raised.ok).toBe(true);
	expect(requests.at(-1)?.body).toMatchObject({ action: "raise", amount: 50 });
	const resumed = createTokenTableClient();
	const reloaded = await resumed.request({ type: "snapshot" });
	expect(reloaded.snapshot?.players[0]?.cards).toEqual(["A♥", "K♥"]);
	expect(
		requests.some(
			(request) =>
				request.path.endsWith("/view") &&
				request.method === "POST" &&
				JSON.stringify(request.body).includes(token)
		)
	).toBe(true);
});

test("a host with missing capabilities does not execute a simulated game", async () => {
	Object.defineProperty(globalThis, "window", {
		configurable: true,
		value: { ryu: {} },
	});
	const client = createTokenTableClient();
	expect(client.mode).toBe("host");
	for (const request of [
		{ type: "snapshot" },
		{ type: "join", buyIn: 100 },
		{ type: "action", action: "call" },
	] as const) {
		const result = await client.request(request);
		expect(result.ok).toBe(false);
		expect(result.snapshot).toBeUndefined();
		expect(result.error).toContain("unavailable");
	}
});

test("legacy identity upgrade selects a fresh table once and remembers it across reload", async () => {
	const storage = new Map<string, string>();
	const requests: AppRequestInput[] = [];
	const rooms: string[] = [];
	let upgradeDenied = false;
	const legacy: WireTableSnapshot = {
		table_id: "legacy-table",
		name: "Legacy room",
		max_seats: 6,
		small_blind: 5,
		big_blind: 10,
		starting_stack: 1000,
		phase: "preflop",
		hand_number: 1,
		hand_id: "legacy-hand",
		action_seq: 0,
		button_seat: 0,
		small_blind_seat: 0,
		big_blind_seat: 1,
		current_turn: "legacy-a",
		current_bet: 10,
		min_raise: 10,
		pot: 15,
		settled_pot: 0,
		board: [],
		last_action: null,
		viewerUpgradeRequired: true,
		players: [
			{
				player_id: "legacy-a",
				display_name: "Alice",
				seat: 0,
				stack: 995,
				committed: 5,
				street_committed: 5,
				in_hand: true,
				folded: false,
				hole_cards: [],
			},
			{
				player_id: "legacy-b",
				display_name: "Bob",
				seat: 1,
				stack: 990,
				committed: 10,
				street_committed: 10,
				in_hand: true,
				folded: false,
				hole_cards: [],
			},
		],
	};
	const fresh: WireTableSnapshot = {
		...legacy,
		table_id: "fresh-table",
		phase: "waiting",
		hand_number: 0,
		hand_id: null,
		current_turn: null,
		pot: 0,
		players: [],
		viewerUpgradeRequired: false,
	};
	Object.defineProperty(globalThis, "window", {
		configurable: true,
		value: {
			sessionStorage: {
				getItem: (key: string) => storage.get(key) ?? null,
				setItem: (key: string, value: string) => storage.set(key, value),
			},
			ryu: {
				context: { tableId: legacy.table_id, roomId: "legacy-room" },
				app: {
					request: async (input: AppRequestInput) => {
						requests.push(input);
						if (input.path.endsWith("/upgrade") && input.method === "POST") {
							if (upgradeDenied) {
								throw new Error("Play permission is required");
							}
							return fresh;
						}
						return input.path.includes(fresh.table_id) ? fresh : legacy;
					},
				},
				tokenTable: {
					connect: async (input: { roomId: string }) => {
						rooms.push(input.roomId);
						return {
							access: "write",
							memberId: "member",
							roomId: "room",
							presence: [],
							close: async () => undefined,
							publish: async () => undefined,
						};
					},
				},
			},
		},
	});
	const client = createTokenTableClient();
	expect((await client.request({ type: "snapshot" })).snapshot?.handId).toBe(
		"fresh-table-0"
	);
	const reloaded = createTokenTableClient();
	expect((await reloaded.request({ type: "snapshot" })).snapshot?.handId).toBe(
		"fresh-table-0"
	);
	expect(
		requests.filter(
			(request) =>
				request.path.endsWith("/upgrade") && request.method === "POST"
		)
	).toHaveLength(1);
	storage.clear();
	const secondViewer = createTokenTableClient();
	expect(
		(await secondViewer.request({ type: "snapshot" })).snapshot?.handId
	).toBe("fresh-table-0");
	expect(
		requests.filter(
			(request) =>
				request.path.endsWith("/upgrade") && request.method === "POST"
		)
	).toHaveLength(2);
	expect(rooms.every((room) => room === "token-table:fresh-table")).toBe(true);
	expect(
		requests.some((request) => request.path === "/tables/fresh-table")
	).toBe(true);
	expect(requests.some((request) => request.method === "DELETE")).toBe(false);
	storage.clear();
	upgradeDenied = true;
	const readOnly = createTokenTableClient();
	expect((await readOnly.request({ type: "snapshot" })).snapshot?.handId).toBe(
		"legacy-hand"
	);
	expect((await readOnly.request({ type: "join", buyIn: 200 })).ok).toBe(false);
	expect(requests.some((request) => request.path.endsWith("/join"))).toBe(
		false
	);
});
