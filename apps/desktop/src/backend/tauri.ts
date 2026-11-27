import type { Backend } from "./backend";
import {
	decodeError,
	decodeGroupPage,
	decodeNothing,
	decodeSkipPage,
	decodeStatus,
	decodeSuggestedRoots,
	decodeUsageHistory,
	decodeUsagePage,
} from "./decode";

/** Tauri's `invoke`, or anything that stands in for it. */
export type Invoke = (command: string, payload: Record<string, unknown>) => Promise<unknown>;

/**
 * The names of the commands and of their arguments are the ones in
 * contract/commands.json; Tauri reads the arguments in camelCase.
 */
export function tauriBackend(invoke: Invoke, pickFolders: () => Promise<string[]>): Backend
{
	async function ask<T>(command: string, payload: Record<string, unknown>, decode: (answer: unknown) => T): Promise<T>
	{
		let answer: unknown;
		try
		{
			answer = await invoke(command, payload);
		}
		catch (refusal)
		{
			throw decodeError(refusal);
		}

		return decode(answer);
	}

	return {
		start: (roots, options) =>
			ask("scan_start", { roots: [...roots], includeHidden: options.includeHidden }, decodeStatus),
		pause: () => ask("scan_pause", {}, decodeStatus),
		resume: () => ask("scan_resume", {}, decodeStatus),
		cancel: () => ask("scan_cancel", {}, decodeStatus),
		status: () => ask("scan_status", {}, decodeStatus),
		groups: (scanId, query) => ask("scan_groups", { scanId, query }, decodeGroupPage),
		skipped: (scanId, query) => ask("scan_skipped", { scanId, query }, decodeSkipPage),
		history: (scanId, query) => ask("scan_history", { scanId, query }, decodeUsageHistory),
		filesUsed: (scanId, query) => ask("scan_files_used", { scanId, query }, decodeUsagePage),
		pickFolders,
		suggestedRoots: () => ask("suggested_roots", {}, decodeSuggestedRoots),
		reveal: (scanId, groupId, fileId) => ask("reveal_file", { scanId, groupId, fileId }, decodeNothing),
		revealIndexed: (scanId, fileId) => ask("reveal_indexed_file", { scanId, fileId }, decodeNothing),
	};
}
