import { describe, expect, test } from "bun:test";
import { handLabel, stakesLabel } from "./table-copy";

describe("Token Table labels", () => {
	test("derive hand and stakes labels from the table snapshot", () => {
		expect(handLabel(0)).toBe("Hand #0");
		expect(handLabel(422)).toBe("Hand #422");
		expect(stakesLabel(5, 10)).toBe("5 / 10 simulated tokens · 6-max");
	});
});
