import {
	markCompanionAppRoot,
	subscribeCompanionTheme,
} from "@ryu/app-host/companion-theme";
import { RyuAppShell } from "@ryu/blocks/companion/app-ui";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { TokenTable } from "./TokenTable.tsx";
import "./token-table.css";

const container = document.getElementById("ryu-plugin-root");
if (container) {
	markCompanionAppRoot(container);
	subscribeCompanionTheme();
	createRoot(container).render(
		<StrictMode>
			<RyuAppShell surface="standard">
				<TokenTable />
			</RyuAppShell>
		</StrictMode>
	);
}
