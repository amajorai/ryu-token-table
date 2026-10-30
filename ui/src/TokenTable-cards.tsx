import { useLiteralMessages } from "@ryu/i18n/react";
import { formatTokens, YOU_ID } from "./TokenTable-tokens.ts";
import type { PlayerSeat } from "./types.ts";

const seatNames = [
	"north",
	"northeast",
	"southeast",
	"south",
	"southwest",
	"northwest",
] as const;

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

export function PlayingCard({
	back = false,
	card,
	compact = false,
}: {
	back?: boolean;
	card?: string;
	compact?: boolean;
}) {
	const tl = useLiteralMessages();
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
			aria-label={tl("Card {safeCard}", { safeCard })}
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

export function Seat({
	player,
	active,
}: {
	player: PlayerSeat;
	active: boolean;
}) {
	const tl = useLiteralMessages();
	const isYou = player.id === YOU_ID;
	const seatClass = seatNames[player.seat - 1] ?? "north";
	const showCards = player.status !== "folded";

	return (
		<article
			aria-current={active ? "true" : undefined}
			aria-label={tl(
				"{name}, {tokens} remaining{turn, select, active {, current turn} other {}}",
				{
					name: player.name,
					tokens: formatTokens(player.stack),
					turn: active ? "active" : "idle",
				}
			)}
			className={`poker-seat poker-seat--${seatClass} ${active ? "poker-seat--active" : ""} ${isYou ? "poker-seat--you" : ""} ${player.status === "folded" ? "poker-seat--folded" : ""}`}
			data-seat={player.seat}
		>
			{showCards ? (
				<div
					aria-label={tl("{name}'s cards", { name: player.name })}
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

export function Board({ cards }: { cards: string[] }) {
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
