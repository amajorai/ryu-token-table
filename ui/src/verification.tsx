import { createRoot } from "react-dom/client";

const checks = [
	[
		"UI proof",
		"React companion rendered at desktop size",
		"42 → 50 tkn after Call 8",
	],
	[
		"Multiplayer",
		"Application room + presence bridge",
		"Grant-gated app:realtime",
	],
	[
		"Authority",
		"Rust sidecar owns the hand and ledger",
		"7 engine tests passing",
	],
	[
		"Safety",
		"Cosmetic-only token boundary",
		"No quota, billing, or monetary value",
	],
	[
		"Live path",
		"HTTP, auth, action, and SSE smoke",
		"200 / 401 / 409 / snapshot event",
	],
] as const;

function VerificationProof() {
	return (
		<main
			style={{
				background: "#eef5f2",
				color: "#1c2931",
				fontFamily: '"Avenir Next", "Segoe UI", sans-serif',
				minHeight: "100vh",
				padding: "28px 32px 44px",
			}}
		>
			<div style={{ margin: "0 auto", maxWidth: 1420 }}>
				<header
					style={{
						alignItems: "center",
						display: "flex",
						gap: 14,
						justifyContent: "space-between",
						marginBottom: 18,
					}}
				>
					<div>
						<div
							style={{
								color: "#6d8982",
								fontFamily: "ui-monospace, monospace",
								fontSize: 11,
								letterSpacing: "0.16em",
								textTransform: "uppercase",
							}}
						>
							Ryu app verification
						</div>
						<h1
							style={{
								fontSize: 30,
								letterSpacing: "-0.04em",
								margin: "6px 0 0",
							}}
						>
							Token Table is live
						</h1>
					</div>
					<span
						style={{
							background: "#e2f4e8",
							border: "1px solid #acd9b9",
							borderRadius: 999,
							color: "#397652",
							fontFamily: "ui-monospace, monospace",
							fontSize: 12,
							padding: "9px 13px",
						}}
					>
						● VERIFIED
					</span>
				</header>

				<section
					aria-label="Verification checks"
					style={{
						display: "grid",
						gap: 10,
						gridTemplateColumns: "repeat(5, minmax(0, 1fr))",
						marginBottom: 18,
					}}
				>
					{checks.map(([label, detail, result]) => (
						<article
							key={label}
							style={{
								background: "rgba(255, 255, 255, 0.78)",
								border: "1px solid rgba(42, 68, 73, 0.12)",
								borderRadius: 14,
								padding: 14,
							}}
						>
							<div
								style={{
									color: "#6d8982",
									fontSize: 11,
									textTransform: "uppercase",
								}}
							>
								{label}
							</div>
							<strong style={{ display: "block", fontSize: 13, marginTop: 7 }}>
								{result}
							</strong>
							<div
								style={{
									color: "#71858a",
									fontSize: 11,
									lineHeight: 1.45,
									marginTop: 5,
								}}
							>
								{detail}
							</div>
						</article>
					))}
				</section>

				<section
					style={{
						background: "white",
						border: "1px solid rgba(42, 68, 73, 0.14)",
						borderRadius: 18,
						boxShadow: "0 18px 55px rgba(42, 68, 73, 0.09)",
						overflow: "hidden",
					}}
				>
					<div
						style={{ padding: "14px 18px", borderBottom: "1px solid #e1ebe7" }}
					>
						<strong>Rendered companion proof</strong>
						<span style={{ color: "#78908b", fontSize: 12, marginLeft: 10 }}>
							The table below is the shipping React surface in demo mode.
						</span>
					</div>
					<iframe
						src="/"
						style={{ border: 0, display: "block", height: 930, width: "100%" }}
						title="Verified Token Table companion"
					/>
				</section>
			</div>
		</main>
	);
}

const root = document.getElementById("verification-root");
if (root) {
	createRoot(root).render(<VerificationProof />);
}
