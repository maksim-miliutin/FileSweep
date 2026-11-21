import { isTauri } from "@tauri-apps/api/core";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./App";
import type { Backend } from "./backend/backend";
import { desktopBackend } from "./backend/desktop";
import { text } from "./strings";

import "./base.css";

/**
 * In a browser there is no shell: the development server gets a backend that
 * plays a scan from memory, and a production build has nothing to offer.
 */
async function chooseBackend(): Promise<Backend | null>
{
	if (isTauri())
	{
		return desktopBackend();
	}

	if (import.meta.env.DEV)
	{
		const { demoBackend } = await import("./backend/demo");

		return demoBackend();
	}

	return null;
}

const root = document.getElementById("root");
if (root === null)
{
	throw new Error("index.html has no element with the id root");
}

const backend = await chooseBackend();

createRoot(root).render(
	<StrictMode>
		{backend === null ? <p>{text.notInABrowser}</p> : <App backend={backend} />}
	</StrictMode>,
);
