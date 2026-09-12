import {
	RyuAppEmpty,
	RyuAppMain,
	RyuAppToolbar,
} from "@ryu/blocks/companion/app-ui";
import { Button } from "@ryu/blocks/companion/controls";
import {
	Popover,
	PopoverContent,
	PopoverTrigger,
} from "@ryu/ui/components/popover.tsx";
import { Slider } from "@ryu/ui/components/slider.tsx";
import type { ReactNode } from "react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { createTokenTableClient } from "./bridge.ts";
import {
	DEFAULT_SNAPSHOT,
	getActionAmount,
	getActionAvailability,
	getYou,
} from "./model.ts";
import type {
	HistoryEntry,
	PlayerSeat,
	PokerAction,
	TokenTableRequest,
	TokenTableSnapshot,
} from "./types.ts";

const YOU_ID = "you";
const tokenFormatter = new Intl.NumberFormat("en-US");
const seatNames = [
	"north",
	"northeast",
	"southeast",
	"south",
	"southwest",
	"northwest",
] as const;
const sizePresets = [
	{ label: "25%", ratio: 0.25 },
	{ label: "33%", ratio: 0.33 },
	{ label: "75%", ratio: 0.75 },
	{ label: "133%", ratio: 1.33 },
] as const;

function formatTokens(value: number): string {
	return `${tokenFormatter.format(value)} tokens`;
}

function suitClass(card: string): "red" | "black" {
	return card.endsWith("♥") || card.endsWith("♦") ? "red" : "black";
}

function CardBackMark() {
	return (
		<svg
			aria-hidden="true"
			className="poker-card__back-mark"
			viewBox="0 0 256 260"
		>
			<path d="M239.184 106.203a64.716 64.716 0 0 0-5.576-53.103C219.452 28.459 191 15.784 163.213 21.74A65.586 65.586 0 0 0 52.096 45.22a64.716 64.716 0 0 0-43.23 31.36c-14.31 24.602-11.061 55.634 8.033 76.74a64.665 64.665 0 0 0 5.525 53.102c14.174 24.65 42.644 37.324 70.446 31.36a64.72 64.72 0 0 0 48.754 21.744c28.481.025 53.714-18.361 62.414-45.481a64.767 64.767 0 0 0 43.229-31.36c14.137-24.558 10.875-55.423-8.083-76.483Zm-97.56 136.338a48.397 48.397 0 0 1-31.105-11.255l1.535-.87 51.67-29.825a8.595 8.595 0 0 0 4.247-7.367v-72.85l21.845 12.636c.218.111.37.32.409.563v60.367c-.056 26.818-21.783 48.545-48.601 48.601Zm-104.466-44.61a48.345 48.345 0 0 1-5.781-32.589l1.534.921 51.722 29.826a8.339 8.339 0 0 0 8.441 0l63.181-36.425v25.221a.87.87 0 0 1-.358.665l-52.335 30.184c-23.257 13.398-52.97 5.431-66.404-17.803ZM23.549 85.38a48.499 48.499 0 0 1 25.58-21.333v61.39a8.288 8.288 0 0 0 4.195 7.316l62.874 36.272-21.845 12.636a.819.819 0 0 1-.767 0L41.353 151.53c-23.211-13.454-31.171-43.144-17.804-66.405v.256Zm179.466 41.695-63.08-36.63L161.73 77.86a.819.819 0 0 1 .768 0l52.233 30.184a48.6 48.6 0 0 1-7.316 87.635v-61.391a8.544 8.544 0 0 0-4.4-7.213Zm21.742-32.69-1.535-.922-51.619-30.081a8.39 8.39 0 0 0-8.492 0L99.98 99.808V74.587a.716.716 0 0 1 .307-.665l52.233-30.133a48.652 48.652 0 0 1 72.236 50.391v.205ZM88.061 139.097l-21.845-12.585a.87.87 0 0 1-.41-.614V65.685a48.652 48.652 0 0 1 79.757-37.346l-1.535.87-51.67 29.825a8.595 8.595 0 0 0-4.246 7.367l-.051 72.697Zm11.868-25.58 28.138-16.217 28.188 16.218v32.434l-28.086 16.218-28.188-16.218-.052-32.434Z" />
		</svg>
	);
}

function PlayingCard({
	back = false,
	card,
	compact = false,
}: {
	back?: boolean;
	card?: string;
	compact?: boolean;
}) {
	if (back) {
		return (
			<span
				aria-label="Hidden card"
				className={`poker-card poker-card--back ${compact ? "poker-card--compact" : ""}`}
			>
				<CardBackMark />
			</span>
		);
	}

	const safeCard = card ?? "?";
	return (
		<span
			aria-label={`Card ${safeCard}`}
			className={`poker-card ${compact ? "poker-card--compact" : ""} poker-card--${suitClass(safeCard)}`}
		>
			<strong className="poker-card__rank">{safeCard.slice(0, -1)}</strong>
			<span aria-hidden="true" className="poker-card__suit">
				{safeCard.slice(-1)}
			</span>
		</span>
	);
}

function Avatar({ player }: { player: PlayerSeat }) {
	return (
		<span
			aria-hidden="true"
			className={`poker-avatar poker-avatar--${player.color}`}
		>
			{player.avatar}
		</span>
	);
}

function Seat({ player, active }: { player: PlayerSeat; active: boolean }) {
	const isYou = player.id === YOU_ID;
	const seatClass = seatNames[player.seat - 1] ?? "north";
	const showCards = player.status !== "folded";

	return (
		<article
			aria-current={active ? "true" : undefined}
			aria-label={`${player.name}, ${formatTokens(player.stack)} remaining${active ? ", current turn" : ""}`}
			className={`poker-seat poker-seat--${seatClass} ${active ? "poker-seat--active" : ""} ${isYou ? "poker-seat--you" : ""} ${player.status === "folded" ? "poker-seat--folded" : ""}`}
			data-seat={player.seat}
		>
			{showCards ? (
				<div
					aria-label={`${player.name}'s cards`}
					className="poker-seat__cards"
				>
					{isYou && player.cards ? (
						<>
							<PlayingCard card={player.cards[0]} compact />
							<PlayingCard card={player.cards[1]} compact />
						</>
					) : (
						<>
							<PlayingCard back compact />
							<PlayingCard back compact />
						</>
					)}
				</div>
			) : null}
			<div className="poker-player-card">
				<div className="poker-player-card__identity">
					<Avatar player={player} />
					<div className="poker-player-card__copy">
						<span className="poker-player-card__name">
							<strong>{isYou ? "You" : player.name}</strong>
							<span aria-label="Verified" className="poker-verified">
								✓
							</span>
						</span>
						<span>{formatTokens(player.stack)}</span>
					</div>
					{active ? (
						<span aria-label="Current turn" className="poker-turn-dot" />
					) : null}
				</div>
			</div>
			{player.bet > 0 ? (
				<span className="poker-seat__bet">+{formatTokens(player.bet)}</span>
			) : null}
			{player.status === "thinking" ? (
				<span className="poker-seat__status">••• Thinking · 24s</span>
			) : null}
			{player.status === "folded" ? (
				<span className="poker-seat__status poker-seat__status--muted">
					folded
				</span>
			) : null}
		</article>
	);
}

function Board({ cards }: { cards: string[] }) {
	const emptyCards = Math.max(0, 5 - cards.length);
	return (
		<div aria-label="Community cards" className="poker-board" role="group">
			{cards.map((card) => (
				<PlayingCard card={card} key={card} />
			))}
			{Array.from({ length: emptyCards }, (_, index) => (
				<PlayingCard back key={`empty-${index}`} />
			))}
		</div>
	);
}

function IconButton({
	"aria-label": ariaLabel,
	children,
	onClick,
}: {
	"aria-label": string;
	children: ReactNode;
	onClick: () => void;
}) {
	return (
		<Button
			aria-label={ariaLabel}
			className="poker-icon-button"
			onClick={onClick}
			size="icon-sm"
			variant="ghost-muted"
		>
			{children}
		</Button>
	);
}

function HistoryPopover({
	entries,
	onClose,
}: {
	entries: HistoryEntry[];
	onClose: () => void;
}) {
	return (
		<aside aria-label="Hand history" className="poker-history-popover">
			<div className="poker-history-popover__header">
				<div>
					<span className="poker-kicker">Hand #821</span>
					<h2>Hand history</h2>
				</div>
				<IconButton aria-label="Close hand history" onClick={onClose}>
					×
				</IconButton>
			</div>
			<ol className="poker-history-list">
				{entries.map((entry, index) => (
					<li
						className={`poker-history-item ${entry.accent ? "poker-history-item--accent" : ""}`}
						key={`${entry.actor}-${entry.action}-${index}`}
					>
						<span aria-hidden="true" className="poker-history-item__marker" />
						<div>
							<strong>{entry.actor}</strong>
							<span>
								{entry.action}
								{entry.amount ? ` · ${formatTokens(entry.amount)}` : ""}
							</span>
						</div>
					</li>
				))}
			</ol>
			<p className="poker-history-popover__note">
				◇&nbsp; Simulated tokens only. No quota or monetary value.
			</p>
		</aside>
	);
}

function ActionButton({
	action,
	disabled,
	hint,
	label,
	onClick,
	primary = false,
}: {
	action: string;
	disabled: boolean;
	hint?: string;
	label: string;
	onClick: () => void;
	primary?: boolean;
}) {
	return (
		<Button
			aria-label={hint ? `${label}, ${hint}` : label}
			className={`poker-dock-action ${primary ? "poker-dock-action--primary" : ""}`}
			data-action={action}
			disabled={disabled}
			onClick={onClick}
			size="sm"
			variant={primary ? "mono" : "outline"}
		>
			<span>{label}</span>
			{hint ? <small>{hint}</small> : null}
		</Button>
	);
}

function ActionDock({
	snapshot,
	onRequest,
}: {
	snapshot: TokenTableSnapshot;
	onRequest: (action: PokerAction, amount?: number) => void;
}) {
	const [amount, setAmount] = useState(32);
	const [selectedPreset, setSelectedPreset] = useState("75%");
	const availability = getActionAvailability(snapshot);
	const you = getYou(snapshot);
	const maxAmount = Math.max(1, you?.stack ?? 1);
	const canChooseAmount = availability.bet || availability.raise;
	const callAction: PokerAction = availability.call ? "call" : "check";
	const callLabel = availability.call ? "Call" : "Check";
	const callHint = availability.call ? formatTokens(snapshot.toCall) : "—";
	const sizingAction: PokerAction = availability.bet ? "bet" : "raise";
	const sizingLabel = availability.bet ? "Bet" : "Raise";

	const selectPreset = (label: string, ratio: number) => {
		setSelectedPreset(label);
		setAmount(
			Math.min(maxAmount, Math.max(1, Math.round(snapshot.pot * ratio)))
		);
	};
	const setRange = (value: number | readonly number[]) => {
		setSelectedPreset("");
		setAmount(typeof value === "number" ? value : (value[0] ?? 1));
	};

	return (
		<section aria-label="Your actions" className="poker-action-dock">
			<div className="poker-sizing-dock">
				<div
					aria-label="Bet size presets"
					className="poker-size-presets"
					role="group"
				>
					{sizePresets.map(({ label, ratio }) => (
						<Button
							aria-pressed={selectedPreset === label}
							disabled={!canChooseAmount}
							key={label}
							onClick={() => selectPreset(label, ratio)}
							size="sm"
							type="button"
							variant={selectedPreset === label ? "secondary" : "ghost"}
						>
							{label}
						</Button>
					))}
				</div>
				<Slider
					aria-label="Bet amount"
					disabled={!canChooseAmount}
					max={maxAmount}
					min={1}
					name="bet-amount"
					onValueChange={setRange}
					value={[Math.min(amount, maxAmount)]}
				/>
				<strong className="poker-sizing-dock__amount">
					{formatTokens(Math.min(amount, maxAmount))}
				</strong>
			</div>
			<div className="poker-dock-actions">
				<ActionButton
					action="fold"
					disabled={!availability.fold}
					label="Fold"
					onClick={() => onRequest("fold")}
				/>
				<ActionButton
					action={callAction}
					disabled={
						callAction === "call" ? !availability.call : !availability.check
					}
					hint={callHint}
					label={callLabel}
					onClick={() => onRequest(callAction)}
				/>
				<ActionButton
					action={sizingAction}
					disabled={!canChooseAmount}
					hint={
						canChooseAmount ? formatTokens(Math.min(amount, maxAmount)) : "—"
					}
					label={sizingLabel}
					onClick={() => onRequest(sizingAction, amount)}
					primary
				/>
			</div>
			{snapshot.phase === "showdown" ? (
				<Button
					className="poker-next-hand"
					onClick={() => onRequest("check")}
					size="sm"
					variant="secondary"
				>
					Deal next hand <span aria-hidden="true">↗</span>
				</Button>
			) : null}
			<p className="poker-action-note">
				{snapshot.lastAction} ·{" "}
				{snapshot.connection === "demo" ? "Demo table" : "Live table"} ·
				simulated tokens only
			</p>
		</section>
	);
}

function BuyInPanel({
	onJoin,
	openSeat,
}: {
	onJoin: (amount: number) => void;
	openSeat: number | null;
}) {
	const [buyIn, setBuyIn] = useState(200);
	return (
		<div className="poker-join-panel">
			<span aria-hidden="true" className="poker-join-panel__mark">
				♢
			</span>
			<span className="poker-kicker">
				{openSeat === null ? "Table is full" : `Seat ${openSeat} is open`}
			</span>
			<h2>Join the table</h2>
			<p>
				Bring a simulated stack to this hand. Tokens have no quota or monetary
				value.
			</p>
			<div className="poker-buy-in">
				<Button
					aria-label="Decrease buy-in"
					disabled={buyIn <= 100}
					onClick={() => setBuyIn(Math.max(100, buyIn - 50))}
					size="sm"
					type="button"
					variant="outline"
				>
					−
				</Button>
				<strong>{formatTokens(buyIn)}</strong>
				<Button
					aria-label="Increase buy-in"
					disabled={buyIn >= 500}
					onClick={() => setBuyIn(Math.min(500, buyIn + 50))}
					size="sm"
					type="button"
					variant="outline"
				>
					+
				</Button>
			</div>
			<Button
				className="poker-join-button"
				disabled={openSeat === null}
				onClick={() => onJoin(buyIn)}
				size="sm"
				variant="default"
			>
				Buy in &amp; take seat <span aria-hidden="true">→</span>
			</Button>
		</div>
	);
}

export function TokenTable() {
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
							<span className="poker-hand-number">Hand #821</span>
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
						– 0.5 / 1 Mtok – 6-max <span aria-hidden="true">⌄</span>
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
									? `${activeSeat.name}'s turn`
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
