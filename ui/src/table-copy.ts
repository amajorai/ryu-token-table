export function handLabel(handNumber: number): string {
	return `Hand #${Math.max(0, Math.floor(handNumber))}`;
}

export function stakesLabel(smallBlind: number, bigBlind: number): string {
	return `${smallBlind} / ${bigBlind} simulated tokens · 6-max`;
}
