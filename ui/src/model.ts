import type {
	PokerAction,
	SeatStatus,
	TablePhase,
	TokenTableSnapshot,
} from "./types.ts";

export const YOU_ID = "you";

export const DEFAULT_SNAPSHOT: TokenTableSnapshot = {
	activeSeat: 4,
	board: ["A♠", "7♦", "K♣", "2♥", "9♣"],
	bigBlind: 10,
	connection: "demo",
	handId: "demo-hand-0421",
	handNumber: 421,
	history: [
		{ actor: "Mara", action: "raises", amount: 8 },
		{ actor: "You", action: "calls", amount: 8, accent: true },
		{ actor: "Noah", action: "checks" },
	],
	lastAction: "Mara raised to 8",
	phase: "river",
	players: [
		{
			avatar: "M",
			bet: 8,
			cards: null,
			color: "coral",
			id: "mara",
			name: "Mara",
			seat: 1,
			stack: 184,
			status: "thinking",
		},
		{
			avatar: "J",
			bet: 8,
			cards: null,
			color: "violet",
			id: "jules",
			name: "Jules",
			seat: 2,
			stack: 96,
			status: "active",
		},
		{
			avatar: "N",
			bet: 0,
			cards: null,
			color: "sky",
			id: "noah",
			name: "Noah",
			seat: 3,
			stack: 246,
			status: "active",
		},
		{
			avatar: "Y",
			bet: 8,
			cards: ["Q♠", "10♠"],
			color: "amber",
			id: YOU_ID,
			name: "You",
			seat: 4,
			stack: 132,
			status: "active",
		},
		{
			avatar: "R",
			bet: 0,
			cards: null,
			color: "mint",
			id: "rhea",
			name: "Rhea",
			seat: 5,
			stack: 288,
			status: "folded",
		},
		{
			avatar: "T",
			bet: 0,
			cards: null,
			color: "blue",
			id: "theo",
			name: "Theo",
			seat: 6,
			stack: 74,
			status: "active",
		},
	],
	pot: 42,
	smallBlind: 5,
	serverTime: "just now",
	toCall: 8,
	you: { balance: 640, seat: 4, status: "seated" },
};

export function getYou(snapshot: TokenTableSnapshot) {
	return snapshot.players.find((player) => player.id === YOU_ID) ?? null;
}

export function getActionAvailability(snapshot: TokenTableSnapshot) {
	const yourTurn =
		snapshot.you.status === "seated" &&
		snapshot.activeSeat === snapshot.you.seat;
	const player = getYou(snapshot);
	const canAct = yourTurn && snapshot.phase !== "showdown" && player !== null;
	return {
		allIn: canAct && (player?.stack ?? 0) > 0,
		bet: canAct && snapshot.toCall === 0,
		call: canAct && snapshot.toCall > 0,
		check: canAct && snapshot.toCall === 0,
		fold: canAct,
		raise:
			canAct && snapshot.toCall > 0 && (player?.stack ?? 0) > snapshot.toCall,
		yourTurn,
	};
}

export function getActionAmount(
	snapshot: TokenTableSnapshot,
	action: PokerAction,
	selectedAmount: number
) {
	const player = getYou(snapshot);
	const stack = player?.stack ?? 0;
	if (action === "all-in") {
		return stack;
	}
	if (action === "call") {
		return Math.min(snapshot.toCall, stack);
	}
	return Math.min(Math.max(1, selectedAmount), stack);
}

export function applyDemoAction(
	snapshot: TokenTableSnapshot,
	action: PokerAction,
	selectedAmount: number
): TokenTableSnapshot {
	const availability = getActionAvailability(snapshot);
	if (!availability[action === "all-in" ? "allIn" : action]) {
		return snapshot;
	}

	const player = getYou(snapshot);
	if (!player) {
		return snapshot;
	}
	const amount =
		action === "fold" ? 0 : getActionAmount(snapshot, action, selectedAmount);
	const label =
		action === "all-in"
			? "goes all-in"
			: `${action}${amount > 0 ? ` ${amount}` : ""}`;
	const nextPlayers = snapshot.players.map((seat) =>
		seat.id === YOU_ID
			? {
					...seat,
					bet: seat.bet + amount,
					stack: seat.stack - amount,
					status: "active" as const,
				}
			: seat
	);
	const nextPot = snapshot.pot + amount;
	const nextPhase: TablePhase = action === "fold" ? "showdown" : snapshot.phase;
	return {
		...snapshot,
		activeSeat: action === "fold" ? snapshot.activeSeat : 2,
		history: [
			...snapshot.history.slice(-5),
			{
				actor: "You",
				action: label,
				amount: amount > 0 ? amount : undefined,
				accent: true,
			},
		],
		lastAction:
			action === "fold" ? "You folded — hand complete" : `You ${label}`,
		phase: nextPhase,
		players: nextPlayers,
		pot: nextPot,
		toCall:
			action === "fold" || action === "check"
				? 0
				: Math.max(0, snapshot.toCall - amount),
	};
}

export function applyDemoJoin(
	snapshot: TokenTableSnapshot,
	buyIn: number
): TokenTableSnapshot {
	const safeBuyIn = Math.min(500, Math.max(100, buyIn));
	const nextPlayers = snapshot.players.map((seat) =>
		seat.id === YOU_ID
			? {
					...seat,
					bet: 0,
					cards: ["Q♠", "10♠"] as [string, string],
					stack: safeBuyIn,
					status: "active" as const,
				}
			: seat
	);
	return {
		...snapshot,
		activeSeat: 4,
		connection: "demo",
		lastAction: "You joined the table",
		players: nextPlayers,
		you: {
			balance: Math.max(0, snapshot.you.balance - safeBuyIn),
			seat: 4,
			status: "seated",
		},
	};
}

export function applyDemoLeave(
	snapshot: TokenTableSnapshot
): TokenTableSnapshot {
	return {
		...snapshot,
		activeSeat: 2,
		lastAction: "Seat open — waiting for you",
		players: snapshot.players.map((seat) =>
			seat.id === YOU_ID
				? { ...seat, cards: null, status: "empty" as const }
				: seat
		),
		you: { ...snapshot.you, seat: null, status: "waiting" as SeatStatus },
	};
}

export function startDemoHand(
	snapshot: TokenTableSnapshot
): TokenTableSnapshot {
	return {
		...DEFAULT_SNAPSHOT,
		connection: snapshot.connection,
		handId: `demo-hand-${String(snapshot.handNumber + 1).padStart(4, "0")}`,
		handNumber: snapshot.handNumber + 1,
		players: DEFAULT_SNAPSHOT.players.map((seat) =>
			seat.id === YOU_ID
				? {
						...seat,
						stack:
							snapshot.players.find((player) => player.id === YOU_ID)?.stack ??
							seat.stack,
						status: "active" as const,
					}
				: seat
		),
		you: { ...snapshot.you, status: "seated", seat: 4 },
	};
}
