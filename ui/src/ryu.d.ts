export interface AppRequestInput {
	body?: unknown;
	method?: "DELETE" | "GET" | "PATCH" | "POST" | "PUT";
	path: string;
}

export interface RyuApp {
	/** Forward a relative path to this app's own public sidecar mount. */
	request(input: AppRequestInput): Promise<unknown>;
}

export interface RyuTokenTableEvent {
	data: unknown;
	name: string;
}

export interface RyuTokenTableConnection {
	access: "read" | "write";
	close(): Promise<void>;
	memberId: string;
	presence: unknown[];
	publish(name: string, data: unknown): Promise<void>;
	roomId: string;
}

export interface RyuTokenTableConnectHandlers {
	onClose?: (event: { code: number; reason: string }) => void;
	onError?: (error: unknown) => void;
	onEvent?: (event: RyuTokenTableEvent) => void;
	onPresence?: (presence: { data: unknown }) => void;
}

export interface RyuTokenTable {
	/** Connect to the host-owned application room; the host hides tokens and URLs. */
	connect(
		input: { roomId: string },
		handlers?: RyuTokenTableConnectHandlers
	): Promise<RyuTokenTableConnection>;
}

export interface RyuBridge {
	app?: RyuApp;
	context?: { playerId?: string; roomId?: string; tableId?: string } | null;
	tokenTable?: RyuTokenTable;
}

declare global {
	interface Window {
		ryu?: RyuBridge;
	}
}
