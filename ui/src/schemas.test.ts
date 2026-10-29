import { describe, expect, it } from "bun:test";
import {
	snapshotEnvelopeSchema,
	tablePhaseSchema,
	viewerCredentialSchema,
	wireSnapshotSchema,
} from "./schemas.ts";
import type { WireTableSnapshot } from "./types.ts";

function snapshot(): WireTableSnapshot {
	return {
		action_seq: 3,
		big_blind: 10,
		big_blind_seat: 1,
		board: [{ rank: 14, suit: "h" }],
		button_seat: 0,
		current_bet: 10,
		current_turn: "player-1",
		hand_id: "hand-1",
		hand_number: 1,
		last_action: {
			action: "call",
			action_id: "action-3",
			action_seq: 3,
			amount: null,
			player_id: "player-1",
		},
		max_seats: 6,
		min_raise: 10,
		name: "Studio",
		phase: "preflop",
		players: [
			{
				committed: 10,
				display_name: "You",
				folded: false,
				hole_cards: [
					{ rank: 13, suit: "h" },
					{ rank: 12, suit: "h" },
				],
				in_hand: true,
				player_id: "player-1",
				seat: 0,
				stack: 990,
				street_committed: 10,
			},
		],
		pot: 20,
		settled_pot: 0,
		small_blind: 5,
		small_blind_seat: 0,
		starting_stack: 1000,
		table_id: "table-1",
	};
}
describe("Token Table wire and viewer contracts", () => {
	it("retains snapshot object identities and additive JSON while validating nested cards and winners", () => {
		const input = {
			...snapshot(),
			metadata: { producer: "Rust" },
			winners: [{ player_id: "player-1", payout: 20, hand_rank: [8, 14] }],
		};
		expect(wireSnapshotSchema.parse(input)).toBe(input);
		expect(
			snapshotEnvelopeSchema.parse({
				snapshot: input,
				topic: "token_table.snapshot",
			}).snapshot
		).toBe(input);
		expect(
			wireSnapshotSchema.safeParse({
				...input,
				winners: [{ player_id: "player-1", payout: "20", hand_rank: [8, 14] }],
			}).success
		).toBe(false);
		expect(
			wireSnapshotSchema.safeParse({
				...input,
				board: [{ rank: "14", suit: "h" }],
			}).success
		).toBe(false);
	});
	it("accepts legacy snapshots without winners and the server's waiting phase while requiring every player field", () => {
		const input = snapshot();
		input.phase = "waiting";
		expect(wireSnapshotSchema.parse(input)).toBe(input);
		expect(tablePhaseSchema.parse("waiting")).toBe("waiting");
		expect(
			wireSnapshotSchema.safeParse({
				...input,
				players: [
					{ player_id: "player-1", display_name: "You", seat: 0, stack: 990 },
				],
			}).success
		).toBe(false);
	});
	it("retains the existing 64-character hexadecimal viewer capability check", () => {
		expect(
			viewerCredentialSchema.parse({
				playerId: "player-1",
				viewerToken: "F".repeat(64),
			}).viewerToken
		).toBe("F".repeat(64));
		expect(
			viewerCredentialSchema.safeParse({
				playerId: "player-1",
				viewerToken: "f".repeat(63),
			}).success
		).toBe(false);
		expect(
			viewerCredentialSchema.safeParse({
				playerId: "player-1",
				viewerToken: "g".repeat(64),
			}).success
		).toBe(false);
	});
});
