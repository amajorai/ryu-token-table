import { mkdir } from "node:fs/promises";
import path from "node:path";
import { expect, type FrameLocator, type Page, test } from "@playwright/test";
import {
	actionBodySchema,
	joinTableBodySchema,
	seatPlayerBodySchema,
	wireSnapshotSchema,
} from "../src/schemas.ts";
import "./fixture-types.ts";

const evidenceDirectory = path.resolve(import.meta.dirname, "evidence");

async function saveProductScreenshot(frame: FrameLocator, name: string) {
	await mkdir(evidenceDirectory, { recursive: true });
	await frame.locator(".token-table-app").screenshot({
		animations: "disabled",
		path: path.join(evidenceDirectory, name),
	});
}

function demoFrame(page: Page) {
	return page.frameLocator('iframe[title="Token Table demo preview"]');
}

function hostFrame(page: Page) {
	return page.frameLocator('iframe[title="Token Table ExtensionHost proof"]');
}

test("hand history opens with the current actions and closes from its own control", async ({
	page,
}) => {
	await page.goto("/?mode=demo");
	const app = demoFrame(page);
	const openHistory = app.getByRole("button", { name: "Open hand history" });
	await expect(openHistory).toBeVisible();
	await openHistory.click();

	const history = app.getByRole("dialog", { name: "Hand history" });
	await expect(history).toBeVisible();
	await expect(
		history.getByRole("heading", { name: "Hand history" })
	).toBeVisible();
	await expect(
		history.getByText("raises · 8 tokens", { exact: true })
	).toBeVisible();
	await expect(
		history.getByText("calls · 8 tokens", { exact: true })
	).toBeVisible();
	await expect(
		history.getByText("Simulated tokens only. No quota or monetary value.")
	).toBeVisible();
	await history.getByRole("button", { name: "Close hand history" }).click();
	await expect(history).not.toBeVisible();
	await expect(openHistory).toBeFocused();
	await saveProductScreenshot(app, "token-table-history-closed.png");
});

test("bet sizing preset and slider change the submitted raise and resulting table state", async ({
	page,
}) => {
	await page.goto("/?mode=demo");
	const app = demoFrame(page);
	const presets = app.getByRole("group", { name: "Bet size presets" });
	const thirtyThreePercent = presets.getByRole("button", {
		name: "33%",
		exact: true,
	});
	await thirtyThreePercent.click();
	await expect(thirtyThreePercent).toHaveAttribute("aria-pressed", "true");
	await expect(app.locator(".poker-sizing-dock__amount")).toHaveText(
		"14 tokens"
	);

	const slider = app.getByRole("slider", { name: "Bet amount" });
	await slider.focus();
	await slider.press("ArrowRight");
	await expect(slider).toHaveAttribute("aria-valuenow", "15");
	await expect(app.locator(".poker-sizing-dock__amount")).toHaveText(
		"15 tokens"
	);
	await app
		.getByRole("button", { name: "Raise, 15 tokens", exact: true })
		.click();

	await expect(app.locator(".poker-pot")).toContainText("57 tokens");
	await expect(app.locator(".poker-seat--south")).toContainText("117 tokens");
	await expect(app.locator(".poker-action-note")).toContainText("You raise 15");
	await expect(app.locator(".poker-history-list")).toHaveCount(0);
	await saveProductScreenshot(app, "token-table-raised-15.png");
});

test("fold completes the hand and dealing again advances the hand with a fresh board", async ({
	page,
}) => {
	await page.goto("/?mode=demo");
	const app = demoFrame(page);
	await app.getByRole("button", { name: "Fold", exact: true }).click();
	await expect(app.locator(".poker-action-note")).toContainText(
		"You folded — hand complete"
	);
	await expect(
		app.getByRole("button", { name: /Deal next hand/ })
	).toBeVisible();

	await app.getByRole("button", { name: /Deal next hand/ }).click();
	await expect(app.locator(".poker-hand-number")).toHaveText("Hand #422");
	await expect(app.locator(".poker-pot")).toContainText("42 tokens");
	await expect(
		app.getByRole("group", { name: "Community cards" })
	).toContainText("A♠");
	await expect(app.getByRole("button", { name: /Deal next hand/ })).toHaveCount(
		0
	);
	await saveProductScreenshot(app, "token-table-next-hand-422.png");
});

test("leave, buy-in bounds, and joining update the displayed seat and stack", async ({
	page,
}) => {
	await page.goto("/?mode=demo");
	const app = demoFrame(page);
	await app.getByRole("button", { name: "Leave table", exact: true }).click();
	await expect(
		app.getByRole("heading", { name: "Join the table" })
	).toBeVisible();
	await expect(app.getByText("Seat 4 is open", { exact: true })).toBeVisible();

	const decrease = app.getByRole("button", { name: "Decrease buy-in" });
	const increase = app.getByRole("button", { name: "Increase buy-in" });
	await decrease.click();
	await decrease.click();
	await expect(app.getByText("100 tokens", { exact: true })).toBeVisible();
	await expect(decrease).toBeDisabled();
	await increase.click();
	await expect(app.getByText("150 tokens", { exact: true })).toBeVisible();
	await app.getByRole("button", { name: /Buy in & take seat/ }).click();

	await expect(app.locator(".poker-seat--south")).toContainText("150 tokens");
	await expect(
		app.getByRole("heading", { name: "Join the table" })
	).toHaveCount(0);
	await expect(app.locator(".poker-disclaimer")).toContainText(
		"no quota, redemption, or monetary value"
	);
	await saveProductScreenshot(app, "token-table-buy-in-150.png");
});

test("the real ExtensionHost bridge joins, calls, and publishes the updated table snapshot", async ({
	page,
}) => {
	await page.goto("/");
	const app = hostFrame(page);
	await expect(
		app.getByRole("heading", { name: "Join the table" })
	).toBeVisible();
	await expect(app.getByText("Seat 2 is open", { exact: true })).toBeVisible();
	await app.getByRole("button", { name: "Increase buy-in" }).click();
	await expect(app.getByText("250 tokens", { exact: true })).toBeVisible();
	await app.getByRole("button", { name: /Buy in & take seat/ }).click();
	await expect(
		app.getByRole("article", {
			name: "You, 250 tokens remaining, current turn",
			exact: true,
		})
	).toBeVisible();
	await app
		.getByRole("button", { name: "Call, 10 tokens", exact: true })
		.click();

	await expect(app.locator(".poker-pot")).toContainText("25 tokens");
	await expect(app.locator(".poker-action-note")).toContainText("You call 10");
	const fixture = await page.evaluate(() => {
		const hostFixture = window.__tokenTableFixture;
		if (!hostFixture) {
			throw new Error("The ExtensionHost fixture was not installed.");
		}
		return {
			appCalls: hostFixture.appCalls,
			realtimeConnections: hostFixture.realtimeConnections,
			realtimePublishes: hostFixture.realtimePublishes,
			snapshot: hostFixture.getSnapshot(),
		};
	});
	const join = fixture.appCalls.find(
		(call) => call.method === "POST" && call.path.endsWith("/join")
	);
	const seat = fixture.appCalls.find(
		(call) => call.method === "POST" && call.path.endsWith("/seat")
	);
	const action = fixture.appCalls.find(
		(call) => call.method === "POST" && call.path.endsWith("/action")
	);
	expect(joinTableBodySchema.parse(join?.body)).toMatchObject({
		display_name: "You",
		stack: 250,
	});
	expect(seatPlayerBodySchema.parse(seat?.body)).toMatchObject({
		player_id: "host-proof-player",
		seat: 1,
	});
	const actionBody = actionBodySchema.parse(action?.body);
	expect(actionBody).toMatchObject({
		action: "call",
		player_id: "host-proof-player",
	});
	expect(actionBody.expected_action_seq).toBe(1);
	expect(fixture.realtimeConnections).toContainEqual({
		room_id: "token-table:host-proof-table",
	});
	expect(
		fixture.realtimePublishes.some((call) => {
			const parsed = wireSnapshotSchema.safeParse(call.input.data);
			return (
				call.event === "table.snapshot" &&
				parsed.success &&
				parsed.data.pot === 25 &&
				parsed.data.last_action?.action === "call"
			);
		})
	).toBe(true);
	expect(fixture.snapshot.pot).toBe(25);
	await saveProductScreenshot(app, "token-table-extension-host-call.png");
});
