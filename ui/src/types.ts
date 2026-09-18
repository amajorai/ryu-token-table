export type SeatStatus = "seated" | "waiting" | "buying";
export type TablePhase = "preflop" | "flop" | "turn" | "river" | "showdown";
export type PokerAction =
	| "fold"
	| "check"
	| "call"
	| "bet"
	| "raise"
	| "all-in";

export interface PlayerSeat {
	avatar: string;
	bet: number;
	cards: [string, string] | null;
	color: string;
	id: string;
	name: string;
	seat: number;
	stack: number;
	status: "active" | "thinking" | "folded" | "empty";
}

export interface WireCard {
	rank: number;
	suit: string;
}

export interface WirePlayerSnapshot {
	committed: number;
	display_name: string;
	folded: boolean;
	hole_cards: WireCard[];
	in_hand: boolean;
	player_id: string;
	seat: number | null;
	stack: number;
	street_committed: number;
}

export interface WireActionRecord {
	action: string;
	action_id: string;
	action_seq: number;
	amount?: number | null;
	player_id: string;
}

export interface WireTableSnapshot {
	action_seq: number;
	big_blind: number;
	big_blind_seat: number | null;
	board: WireCard[];
	button_seat: number | null;
	current_bet: number;
	current_turn: string | null;
	hand_id: string | null;
	hand_number: number;
	last_action: WireActionRecord | null;
	max_seats: number;
	min_raise: number;
	name: string;
	phase: string;
	players: WirePlayerSnapshot[];
	pot: number;
	settled_pot: number;
	small_blind: number;
	small_blind_seat: number | null;
	starting_stack: number;
	table_id: string;
}

export interface HistoryEntry {
	accent?: boolean;
	action: string;
	actor: string;
	amount?: number;
}

export interface TokenTableSnapshot {
	activeSeat: number;
	bigBlind: number;
	board: string[];
	connection: "connected" | "demo" | "reconnecting";
	handId: string;
	handNumber: number;
	history: HistoryEntry[];
	lastAction: string;
	phase: TablePhase;
	players: PlayerSeat[];
	pot: number;
	serverTime: string;
	smallBlind: number;
	toCall: number;
	you: {
		balance: number;
		seat: number | null;
		status: SeatStatus;
	};
}

export type TokenTableRequest =
	| { type: "snapshot" }
	| { type: "action"; action: PokerAction; amount?: number }
	| { type: "join"; buyIn: number }
	| { type: "leave" }
	| { type: "new-hand" };

export interface TokenTableResponse {
	error?: string;
	ok: boolean;
	snapshot?: TokenTableSnapshot;
}

export interface TokenTableSubscription {
	dispose(): void;
}
