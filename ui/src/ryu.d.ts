import type {
	TokenTableAppReply,
	TokenTableJson,
	TokenTableRequestBody,
	TokenTableSnapshotReply,
} from "./schemas.ts";
export interface AppRequestInput {
	body?: TokenTableRequestBody;
	method?: "DELETE" | "GET" | "PATCH" | "POST" | "PUT";
	path: string;
}

export interface RyuApp {
	/** Forward a relative path to this app's own public sidecar mount. */
	request(input: AppRequestInput): Promise<TokenTableAppReply>;
}

export interface RyuTokenTableEvent {
	data: TokenTableJson | TokenTableSnapshotReply;
	name: string;
}

export interface RyuTokenTableConnection {
	access: "read" | "write";
	close(): Promise<void>;
	memberId: string;
	presence: TokenTableJson[];
	publish(
		name: string,
		data: TokenTableJson | TokenTableSnapshotReply
	): Promise<void>;
	roomId: string;
}

export interface RyuTokenTableConnectHandlers {
	onClose?: (event: { code: number; reason: string }) => void;
	onError?: (error: Error | TokenTableJson) => void;
	onEvent?: (event: RyuTokenTableEvent) => void;
	onPresence?: (presence: {
		data: TokenTableJson | TokenTableSnapshotReply;
	}) => void;
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
