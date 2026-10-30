import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import path from "node:path";
import {
	namedAssertionIsPresent,
	reconcileLocalInteractionMap,
} from "../../../../apps-store/companion-suite.mjs";

const e2eDirectory = import.meta.dirname;
const interactionMap = JSON.parse(
	readFileSync(path.join(e2eDirectory, "companion-interactions.json"), "utf8")
);
const specSource = readFileSync(
	path.join(e2eDirectory, "token-table-companion.spec.ts"),
	"utf8"
);

describe("Token Table package-owned Companion interaction map", () => {
	test("maps all 16 exact interaction rows to named browser assertions", () => {
		expect(interactionMap.scope).toBe("complete");
		expect(interactionMap.expectedRows).toBe(16);
		expect(interactionMap.assertions).toHaveLength(16);

		const rows = interactionMap.assertions.map((assertion) => assertion.row);
		const reconciled = reconcileLocalInteractionMap(
			"token-table",
			rows,
			interactionMap
		);
		expect(reconciled).toHaveLength(16);

		for (const { assertion } of reconciled) {
			expect(
				namedAssertionIsPresent(
					specSource,
					assertion.testTitle,
					assertion.assertionMarker
				)
			).toBe(true);
		}
	});

	test("rejects stale or missing source identities", () => {
		const rows = interactionMap.assertions.map((assertion) => assertion.row);
		const stale = interactionMap.assertions.map((assertion, index) =>
			index === 0
				? { ...assertion, row: { ...assertion.row, line: "999" } }
				: assertion
		);
		expect(() =>
			reconcileLocalInteractionMap("token-table", rows, {
				...interactionMap,
				assertions: stale,
			})
		).toThrow("does not match exactly one current central census row");
		expect(() =>
			reconcileLocalInteractionMap(
				"token-table",
				rows.slice(0, -1),
				interactionMap
			)
		).toThrow("complete interaction map expects 16 central rows but found 15");
	});
});
