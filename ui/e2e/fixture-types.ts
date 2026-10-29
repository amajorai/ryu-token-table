import type {
	AppRequestPayload,
	RealtimeConnectPayload,
} from "@ryu/app-host/rpc";
import type { WireTableSnapshot } from "../src/types.ts";

export type HostCall = AppRequestPayload;

export interface RealtimeCall {
	event: string;
	input: {
		connection_id: string;
		data: WireTableSnapshot;
		event: string;
	};
}

export interface TokenTableFixture {
	appCalls: HostCall[];
	getSnapshot(): WireTableSnapshot;
	realtimeConnections: RealtimeConnectPayload[];
	realtimePublishes: RealtimeCall[];
}

declare global {
	interface Window {
		__tokenTableFixture?: TokenTableFixture;
	}
}
