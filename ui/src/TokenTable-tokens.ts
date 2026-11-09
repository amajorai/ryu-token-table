import { translate as tl } from "@ryu/i18n/core";

export const YOU_ID = "you";

/** "1,250 tokens" in the language the Companion shows. */
export function formatTokens(value: number): string {
	return tl("{count, plural, one {# token} other {# tokens}}", {
		count: value,
	});
}
