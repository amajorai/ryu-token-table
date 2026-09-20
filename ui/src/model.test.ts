import { describe, expect, it } from "bun:test";
import {
	applyDemoAction,
	applyDemoJoin,
	applyDemoLeave,
	DEFAULT_SNAPSHOT,
	getActionAvailability,
	startDemoHand,
} from "./model.ts";

describe("token table state model", () => {
	it("enables call and fold when it is your turn with a live to-call", () => {
		const actions = getActionAvailability(DEFAULT_SNAPSHOT);
		expect(actions.yourTurn).toBe(true);
		expect(actions.fold).toBe(true);
		expect(actions.call).toBe(true);
		expect(actions.check).toBe(false);
		expect(actions.bet).toBe(false);
	});

	it("replaces the snapshot after a local action", () => {
		const next = applyDemoAction(DEFAULT_SNAPSHOT, "call", 8);
		expect(next).not.toBe(DEFAULT_SNAPSHOT);
		expect(next.pot).toBe(50);
		expect(next.players.find((player) => player.id === "you")?.stack).toBe(124);
		expect(next.lastAction).toBe("You call 8");
	});

	it("supports join, leave, and next-hand disclosure states", () => {
		const waiting = applyDemoLeave(DEFAULT_SNAPSHOT);
		expect(waiting.you.status).toBe("waiting");
		expect(waiting.you.seat).toBeNull();
		const joined = applyDemoJoin(waiting, 250);
		expect(joined.you.status).toBe("seated");
		expect(joined.players.find((player) => player.id === "you")?.stack).toBe(
			250
		);
		const nextHand = startDemoHand({ ...joined, handNumber: 421 });
		expect(nextHand.handNumber).toBe(422);
		expect(nextHand.handId).toBe("demo-hand-0422");
	});
});
