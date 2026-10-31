import {
	RyuAppEmpty,
	RyuAppMain,
	RyuAppToolbar,
} from "@ryu/blocks/companion/app-ui";
import { Button } from "@ryu/blocks/companion/controls";
import { useLiteralMessages } from "@ryu/i18n/react";
import {
	Popover,
	PopoverContent,
	PopoverTrigger,
} from "@ryu/ui/components/popover.tsx";
import { useCallback, useEffect, useMemo, useState } from "react";
import { createTokenTableClient } from "./bridge.ts";
import { DEFAULT_SNAPSHOT, getActionAmount } from "./model.ts";
import { Board, Seat } from "./TokenTable-cards.tsx";
import {
	ActionDock,
	BuyInPanel,
	HistoryPopover,
} from "./TokenTable-controls.tsx";
import { formatTokens } from "./TokenTable-tokens.ts";
import { handLabel, stakesLabel } from "./table-copy.ts";
import type {
	PokerAction,
	TokenTableRequest,
	TokenTableSnapshot,
} from "./types.ts";

export function TokenTable() {
	const tl = useLiteralMessages();
	const client = useMemo(createTokenTableClient, []);
	const [ready, setReady] = useState(client.mode === "demo");
	const [snapshot, setSnapshot] =
		useState<TokenTableSnapshot>(DEFAULT_SNAPSHOT);
	const [historyOpen, setHistoryOpen] = useState(false);
	const [notice, setNotice] = useState("");

	useEffect(() => {
		const subscription = client.subscribe(setSnapshot);
		let mounted = true;
		client
			.request({ type: "snapshot" })
			.then((response) => {
				if (mounted && !response.ok) {
					setNotice(response.error ?? "The table could not be loaded.");
				}
				if (mounted && response.snapshot) {
					setSnapshot(response.snapshot);
					setReady(true);
				}
			})
			.catch(() => {
				if (mounted) {
					setNotice("Connection paused. Showing the last known table.");
				}
			});
		return () => {
			mounted = false;
			subscription.dispose();
		};
	}, [client]);

	const send = useCallback(
		async (request: TokenTableRequest) => {
			setNotice("");
			try {
				const response = await client.request(request);
				if (response.snapshot) {
					setSnapshot(response.snapshot);
					setReady(true);
				}
				if (!response.ok && response.error) {
					setNotice(response.error);
				}
			} catch (error) {
				setNotice(
					error instanceof Error
						? error.message
						: "The table could not be updated. Try again."
				);
			}
		},
		[client]
	);

	const handleAction = useCallback(
		(action: PokerAction, amount?: number) => {
			if (snapshot.phase === "showdown" && action === "check") {
				void send({ type: "new-hand" });
				return;
			}
			const resolvedAmount = getActionAmount(snapshot, action, amount ?? 12);
			void send({ type: "action", action, amount: resolvedAmount });
		},
		[send, snapshot]
	);

	if (!ready) {
		return (
			<RyuAppEmpty
				description={notice || "Waiting for the host table."}
				title={notice ? "Token Table unavailable" : "Connecting to Token Table"}
			/>
		);
	}

	const activeSeat = snapshot.players.find(
		(player) => player.seat === snapshot.activeSeat
	);
	const openSeat =
		snapshot.players.find((player) => player.status === "empty")?.seat ?? null;

	return (
		<Popover onOpenChange={setHistoryOpen} open={historyOpen}>
			<div className="token-table-app">
				<RyuAppToolbar
					actions={
						<div className="poker-toolbar-actions">
							<span className="poker-hand-number">
								{handLabel(snapshot.handNumber)}
							</span>
							<PopoverTrigger
								render={
									<Button
										aria-label="Open hand history"
										size="icon-sm"
										variant="ghost"
									/>
								}
							>
								<span aria-hidden="true">↶</span>
							</PopoverTrigger>
							<Button
								onClick={() => void send({ type: "leave" })}
								size="sm"
								variant="outline"
							>
								Leave table
							</Button>
						</div>
					}
					title="No-Limit Inference"
				>
					<span className="poker-toolbar-stakes">
						{stakesLabel(snapshot.smallBlind, snapshot.bigBlind)}{" "}
						<span aria-hidden="true">⌄</span>
					</span>
				</RyuAppToolbar>

				<RyuAppMain className="token-table-main">
					<section aria-label="Poker table" className="poker-table-workspace">
						<div className="poker-stage">
							<div className="poker-felt">
								<div className="poker-felt__grid" />
								<div className="poker-pot">
									<span>Pot</span>
									<strong>{formatTokens(snapshot.pot)}</strong>
								</div>
								<Board cards={snapshot.board} />
								{snapshot.players.map((player) => (
									<Seat
										active={player.seat === snapshot.activeSeat}
										key={player.id}
										player={player}
									/>
								))}
								<span
									aria-label="Dealer button"
									className="poker-dealer-button"
								>
									D
								</span>
							</div>
							{snapshot.you.status === "seated" ? null : (
								<BuyInPanel
									onJoin={(buyIn) => void send({ type: "join", buyIn })}
									openSeat={openSeat}
								/>
							)}
							<PopoverContent align="end" aria-label="Hand history">
								<HistoryPopover
									entries={snapshot.history}
									handNumber={snapshot.handNumber}
									onClose={() => setHistoryOpen(false)}
								/>
							</PopoverContent>
							<ActionDock onRequest={handleAction} snapshot={snapshot} />
						</div>
						{notice ? (
							<div aria-live="polite" className="poker-notice" role="status">
								{notice}
							</div>
						) : null}
						<div className="poker-table-footer">
							<span className="poker-table-footer__turn">
								{activeSeat
									? tl("{name}'s turn", { name: activeSeat.name })
									: "Waiting for a seat"}
							</span>
							<span>6-max practice room · simulated tokens only</span>
						</div>
					</section>
				</RyuAppMain>
				<footer className="poker-disclaimer">
					<span aria-hidden="true">◌</span>
					<span>
						Token Table is a practice room. Balances and actions have no quota,
						redemption, or monetary value.
					</span>
				</footer>
			</div>
		</Popover>
	);
}
