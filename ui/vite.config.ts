import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";
import { viteSingleFile } from "vite-plugin-singlefile";

const here = dirname(fileURLToPath(import.meta.url));

export default defineConfig({
	root: here,
	base: "./",
	plugins: [react(), viteSingleFile()],
	build: {
		outDir: "dist",
		emptyOutDir: true,
		target: "esnext",
		cssCodeSplit: false,
		assetsInlineLimit: Number.POSITIVE_INFINITY,
		modulePreload: { polyfill: false },
		rollupOptions: {
			input: { tokenTable: resolve(here, "index.html") },
			output: { inlineDynamicImports: true },
		},
	},
});
