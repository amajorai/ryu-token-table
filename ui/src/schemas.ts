import { z } from "zod";
import type {
	WireActionRecord,
	WireCard,
	WirePlayerSnapshot,
	WireTableSnapshot,
	WireWinner,
} from "./types.ts";

const extra = z.json().optional();
export const wirePokerActionSchema = z.enum([
	"fold",
	"check",
	"call",
	"bet",
	"raise",
	"all_in",
	"all-in",
]);
export type WirePokerAction = z.output<typeof wirePokerActionSchema>;
export const tokenTableJsonSchema = z.json();
export type TokenTableJson = z.output<typeof tokenTableJsonSchema>;
export const wireCardSchema = z
	.object({ rank: z.number(), suit: z.string() })
	.catchall(extra) satisfies z.ZodType<WireCard>;
export const wirePlayerSchema = z
	.object({
		committed: z.number(),
		display_name: z.string(),
		folded: z.boolean(),
		hole_cards: z.array(wireCardSchema),
		in_hand: z.boolean(),
		player_id: z.string(),
		seat: z.number().nullable(),
		stack: z.number(),
		street_committed: z.number(),
	})
	.catchall(extra) satisfies z.ZodType<WirePlayerSnapshot>;
export const wireActionSchema = z
	.object({
		action: wirePokerActionSchema,
		action_id: z.string(),
		action_seq: z.number(),
		amount: z.number().nullish(),
		player_id: z.string(),
	})
	.catchall(extra) satisfies z.ZodType<WireActionRecord>;
export const wireWinnerSchema = z
	.object({
		player_id: z.string(),
		payout: z.number(),
		hand_rank: z.array(z.number()),
	})
	.catchall(extra) satisfies z.ZodType<WireWinner>;
export const tablePhaseSchema = z.enum([
	"waiting",
	"preflop",
	"flop",
	"turn",
	"river",
	"showdown",
]);
const snapshotProjection = z
	.object({
		action_seq: z.number(),
		big_blind: z.number(),
		big_blind_seat: z.number().nullable(),
		board: z.array(wireCardSchema),
		button_seat: z.number().nullable(),
		current_bet: z.number(),
		current_turn: z.string().nullable(),
		hand_id: z.string().nullable(),
		hand_number: z.number(),
		last_action: wireActionSchema.nullable(),
		max_seats: z.number(),
		min_raise: z.number(),
		name: z.string(),
		phase: z.union([tablePhaseSchema, z.literal("complete")]),
		players: z.array(wirePlayerSchema),
		pot: z.number(),
		settled_pot: z.number(),
		small_blind: z.number(),
		small_blind_seat: z.number().nullable(),
		starting_stack: z.number(),
		table_id: z.string(),
		viewerUpgradeRequired: z.boolean().optional(),
		winners: z.array(wireWinnerSchema).optional(),
	})
	.catchall(extra) satisfies z.ZodType<WireTableSnapshot>;
export const wireSnapshotSchema = z.custom<WireTableSnapshot>(
	(value) => snapshotProjection.safeParse(value).success
);
export const snapshotEnvelopeSchema = z
	.object({
		snapshot: wireSnapshotSchema,
		accepted: z.boolean().optional(),
		action_id: z.string().optional(),
		replayed: z.boolean().optional(),
		playerId: z.string().optional(),
		viewerToken: z.string().optional(),
	})
	.catchall(extra);
export const viewerCredentialSchema = z
	.object({
		playerId: z.string(),
		viewerToken: z.string().regex(/^[a-fA-F0-9]{64}$/),
	})
	.catchall(extra);
export type ViewerCredential = z.output<typeof viewerCredentialSchema>;
export const wireTablesSchema = z
	.object({
		tables: z.array(z.union([wireSnapshotSchema, snapshotEnvelopeSchema])),
	})
	.catchall(extra);
export interface TokenTableSnapshotEnvelope {
	accepted?: boolean;
	action_id?: string;
	playerId?: string;
	replayed?: boolean;
	snapshot: WireTableSnapshot;
	viewerToken?: string;
}
export type TokenTableSnapshotReply =
	| WireTableSnapshot
	| TokenTableSnapshotEnvelope;
export type TokenTableAppReply =
	| TokenTableSnapshotReply
	| TokenTableSnapshotReply[]
	| { tables: TokenTableSnapshotReply[] }
	| ViewerCredential;

interface ViewerAuth {
	viewerToken?: string;
}
export interface CreateTableBody extends ViewerAuth {
	big_blind: number;
	max_seats: number;
	name: string;
	small_blind: number;
	starting_stack: number;
}
export interface ActionBody extends ViewerAuth {
	action: WirePokerAction;
	action_id: string;
	amount?: number;
	expected_action_seq: number;
	player_id: string;
}
export interface JoinTableBody extends ViewerAuth {
	display_name: string;
	stack: number;
}
export interface SeatPlayerBody extends ViewerAuth {
	player_id: string;
	seat: number;
}
export interface LeaveTableBody extends ViewerAuth {
	player_id: string;
}
export type TokenTableRequestBody =
	| CreateTableBody
	| ActionBody
	| JoinTableBody
	| SeatPlayerBody
	| LeaveTableBody
	| ViewerAuth;

export const sliderAmountSchema = z
	.union([z.number(), z.array(z.number())])
	.transform((value) => (Array.isArray(value) ? (value[0] ?? 1) : value));

export const createTableBodySchema = z
	.object({
		big_blind: z.number(),
		max_seats: z.number(),
		name: z.string(),
		small_blind: z.number(),
		starting_stack: z.number(),
		viewerToken: z.string().optional(),
	})
	.catchall(extra) satisfies z.ZodType<CreateTableBody>;
export const actionBodySchema = z
	.object({
		action: wirePokerActionSchema,
		action_id: z.string(),
		amount: z.number().optional(),
		expected_action_seq: z.number(),
		player_id: z.string(),
		viewerToken: z.string().optional(),
	})
	.catchall(extra) satisfies z.ZodType<ActionBody>;
export const joinTableBodySchema = z
	.object({
		display_name: z.string(),
		stack: z.number(),
		viewerToken: z.string().optional(),
	})
	.catchall(extra) satisfies z.ZodType<JoinTableBody>;
export const seatPlayerBodySchema = z
	.object({
		player_id: z.string(),
		seat: z.number(),
		viewerToken: z.string().optional(),
	})
	.catchall(extra) satisfies z.ZodType<SeatPlayerBody>;
export const leaveTableBodySchema = z
	.object({ player_id: z.string(), viewerToken: z.string().optional() })
	.catchall(extra) satisfies z.ZodType<LeaveTableBody>;
export const viewerBodySchema = z
	.object({ viewerToken: z.string() })
	.catchall(extra);
