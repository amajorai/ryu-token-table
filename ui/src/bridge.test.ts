import { afterEach, expect, test } from "bun:test";
import { createTokenTableClient } from "./bridge.ts";

const previous = Object.getOwnPropertyDescriptor(globalThis, "window");
afterEach(() => {
	if (previous) {
		Object.defineProperty(globalThis, "window", previous);
	} else {
		Reflect.deleteProperty(globalThis, "window");
	}
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
