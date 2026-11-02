import { translate as tl } from "@ryu/i18n/core";

export function handLabel(handNumber: number): string {
	return tl("Hand #{handNumber}", {
		handNumber: String(Math.max(0, Math.floor(handNumber))),
	});
}

export function stakesLabel(smallBlind: number, bigBlind: number): string {
	return tl("{smallBlind} / {bigBlind} simulated tokens · 6-max", {
		bigBlind,
		smallBlind,
	});
}
