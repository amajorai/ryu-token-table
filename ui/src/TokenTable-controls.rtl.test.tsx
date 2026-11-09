import { expect, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";
import { BuyInPanel } from "./TokenTable-controls.tsx";

// The "Buy in & take seat →" arrow points forward in the reading direction:
// it must mirror under RTL instead of pointing back at the label.

test("the buy-in button's arrow mirrors in right-to-left layouts", () => {
	const html = renderToStaticMarkup(
		<BuyInPanel onJoin={() => undefined} openSeat={2} />
	);
	const arrow = /<span([^>]*)>→<\/span>/u.exec(html);
	expect(arrow).not.toBeNull();
	const attributes = arrow?.[1] ?? "";
	expect(attributes).toContain('aria-hidden="true"');
	expect(attributes).toMatch(
		/class="[^"]*\binline-block\b[^"]*\brtl:-scale-x-100\b/u
	);
});
