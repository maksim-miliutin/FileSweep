import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";

import type { Backend } from "./backend";
import { tauriBackend } from "./tauri";

export function desktopBackend(): Backend
{
	return tauriBackend(invoke, async () =>
	{
		const picked = await open({ directory: true, multiple: true });

		return picked ?? [];
	});
}
