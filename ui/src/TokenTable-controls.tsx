import { Button } from "@ryu/blocks/companion/controls";
import { useLiteralMessages } from "@ryu/i18n/react";
import { Slider } from "@ryu/ui/components/slider.tsx";
import type { ReactNode } from "react";
import { useState } from "react";
import { getActionAvailability, getYou } from "./model.ts";
import { sliderAmountSchema } from "./schemas.ts";
import { formatTokens } from "./TokenTable-tokens.ts";
import { handLabel } from "./table-copy.ts";
import type { HistoryEntry, PokerAction, TokenTableSnapshot } from "./types.ts";

const sizePresets = [
	{ label: "25%", ratio: 0.25 },
	{ label: "33%", ratio: 0.33 },
	{ label: "75%", ratio: 0.75 },
	{ label: "133%", ratio: 1.33 },
] as const;

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
			onClick={onClick}
			size="icon-sm"
			variant="ghost-muted"
		>
			{children}
		</Button>
	);
}

export function HistoryPopover({
	entries,
	handNumber,
	onClose,
}: {
	entries: HistoryEntry[];
	handNumber: number;
	onClose: () => void;
}) {
	return (
		<aside aria-label="Hand history" className="poker-history-popover">
			<div className="poker-history-popover__header">
				<div>
					<span className="poker-hand-label">{handLabel(handNumber)}</span>
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

export function ActionDock({
	snapshot,
	onRequest,
}: {
	snapshot: TokenTableSnapshot;
	onRequest: (action: PokerAction, amount?: number) => void;
}) {
	const tl = useLiteralMessages();
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
		setAmount(sliderAmountSchema.parse(value));
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
				{tl(
					"{lastAction} · {connection, select, demo {Demo table} other {Live table}} · simulated tokens only",
					{
						connection: snapshot.connection === "demo" ? "demo" : "live",
						lastAction: snapshot.lastAction,
					}
				)}
			</p>
		</section>
	);
}

export function BuyInPanel({
	onJoin,
	openSeat,
}: {
	onJoin: (amount: number) => void;
	openSeat: number | null;
}) {
	const tl = useLiteralMessages();
	const [buyIn, setBuyIn] = useState(200);
	return (
		<div className="poker-join-panel">
			<span aria-hidden="true" className="poker-join-panel__mark">
				♢
			</span>
			<span className="poker-hand-label">
				{openSeat === null
					? "Table is full"
					: tl("Seat {openSeat} is open", { openSeat: String(openSeat) })}
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
