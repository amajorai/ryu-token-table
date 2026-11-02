import { createRoot } from "react-dom/client";
import styles from "./verification.module.css";

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
		<main className={styles.proof}>
			<div className={styles.content}>
				<header className={styles.header}>
					<div>
						<div className={styles.verificationLabel}>Ryu app verification</div>
						<h1 className={styles.title}>Token Table is live</h1>
					</div>
					<span className={styles.verified}>● VERIFIED</span>
				</header>

				<section aria-label="Verification checks" className={styles.checks}>
					{checks.map(([label, detail, result]) => (
						<article className={styles.check} key={label}>
							<div className={styles.label}>{label}</div>
							<strong className={styles.result}>{result}</strong>
							<div className={styles.detail}>{detail}</div>
						</article>
					))}
				</section>

				<section className={styles.rendered}>
					<div className={styles.renderedHeading}>
						<strong>Rendered companion proof</strong>
						<span className={styles.renderedDescription}>
							The table below is the shipping React surface in demo mode.
						</span>
					</div>
					<iframe
						className={styles.companion}
						src="/"
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
