import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

export default defineConfig({
	plugins: [react()],

	// Tauri opens the address written in tauri.conf.json. A server that moved
	// to a free port on its own would leave the window blank.
	server: { port: 5173, strictPort: true },
	clearScreen: false,

	// The window is WebView2, which keeps itself current.
	build: { target: "es2022" },

	test: {
		environment: "jsdom",
		setupFiles: ["src/test-setup.ts"],
		restoreMocks: true,
	},
});
