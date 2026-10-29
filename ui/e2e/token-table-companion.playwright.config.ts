import { defineConfig, devices } from "@playwright/test";

const appRoot = new URL("..", import.meta.url).pathname;
const baseURL = "http://127.0.0.1:4517";

export default defineConfig({
	testDir: ".",
	testMatch: "token-table-companion.spec.ts",
	fullyParallel: false,
	workers: 1,
	reporter: "list",
	outputDir: "/tmp/ryu-token-table-playwright-results",
	use: {
		...devices["Desktop Chrome"],
		baseURL,
		viewport: { width: 1600, height: 1120 },
	},
	webServer: {
		command:
			"bun run build && bun run vite --config e2e/vite.config.ts --host 127.0.0.1 --port 4517 --strictPort",
		cwd: appRoot,
		url: baseURL,
		reuseExistingServer: false,
		timeout: 120_000,
	},
});
