import { describe, expect, it } from "bun:test";
import {
	applyDemoAction,
	applyDemoJoin,
	applyDemoLeave,
	DEFAULT_SNAPSHOT,
	getActionAmount,
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

	it("refuses unavailable actions without mutating or replacing the snapshot", () => {
		const waiting = applyDemoLeave(DEFAULT_SNAPSHOT);
		for (const action of [
			"fold",
			"check",
			"call",
			"bet",
			"raise",
			"all-in",
		] as const) {
			expect(applyDemoAction(waiting, action, 25)).toBe(waiting);
		}

		const notYourTurn = { ...DEFAULT_SNAPSHOT, activeSeat: 2 };
		expect(applyDemoAction(notYourTurn, "call", 8)).toBe(notYourTurn);
	});

	it("clamps action amounts to a positive value and the remaining stack", () => {
		expect(getActionAmount(DEFAULT_SNAPSHOT, "raise", -10)).toBe(1);
		expect(getActionAmount(DEFAULT_SNAPSHOT, "raise", 9999)).toBe(132);
		expect(getActionAmount(DEFAULT_SNAPSHOT, "call", 1)).toBe(8);
		expect(getActionAmount(DEFAULT_SNAPSHOT, "all-in", 1)).toBe(132);
		expect(getActionAmount(DEFAULT_SNAPSHOT, "check", 100)).toBe(0);
		expect(getActionAmount(DEFAULT_SNAPSHOT, "fold", 100)).toBe(0);
	});

	it("applies check, fold, raise, and all-in with their distinct state transitions", () => {
		const checkable = { ...DEFAULT_SNAPSHOT, toCall: 0 };
		const checked = applyDemoAction(checkable, "check", 50);
		expect(checked.pot).toBe(checkable.pot);
		expect(checked.toCall).toBe(0);
		expect(checked.lastAction).toBe("You check");

		const folded = applyDemoAction(DEFAULT_SNAPSHOT, "fold", 50);
		expect(folded.phase).toBe("showdown");
		expect(folded.pot).toBe(DEFAULT_SNAPSHOT.pot);
		expect(folded.lastAction).toBe("You folded — hand complete");

		const raised = applyDemoAction(DEFAULT_SNAPSHOT, "raise", 20);
		expect(raised.pot).toBe(62);
		expect(raised.toCall).toBe(0);
		expect(raised.players.find((player) => player.id === "you")?.stack).toBe(
			112
		);

		const allIn = applyDemoAction(DEFAULT_SNAPSHOT, "all-in", 1);
		expect(allIn.pot).toBe(174);
		expect(allIn.players.find((player) => player.id === "you")?.stack).toBe(0);
		expect(allIn.lastAction).toBe("You go all-in");
		expect(allIn.history.at(-1)?.action).toBe("goes all-in");
	});

	it("retains only the newest five prior history rows when recording an action", () => {
		const history = Array.from({ length: 8 }, (_, index) => ({
			action: `action-${index}`,
			actor: `player-${index}`,
		}));
		const next = applyDemoAction({ ...DEFAULT_SNAPSHOT, history }, "call", 8);
		expect(next.history).toHaveLength(6);
		expect(next.history[0]?.action).toBe("action-3");
		expect(next.history.at(-1)?.actor).toBe("You");
	});
});
