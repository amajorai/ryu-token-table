import { resolve } from "node:path";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

const e2eDirectory = import.meta.dirname;

export default defineConfig({
	root: e2eDirectory,
	resolve: { dedupe: ["react", "react-dom"] },
	plugins: [react()],
	server: {
		host: "127.0.0.1",
		port: 4517,
		strictPort: true,
		fs: {
			allow: [resolve(e2eDirectory, "../../../..")],
		},
	},
});
