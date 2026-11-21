import type { GroupPage, GroupQuery, PageQuery, SkipPage, Status } from "./types";

export interface StartOptions
{
	includeHidden: boolean;
}

/**
 * The window talks to nothing else: the real backend calls the Tauri
 * commands, and the fake one lets the interface run in tests and in a plain
 * browser. A call that is turned down rejects with a `BackendError`.
 */
export interface Backend
{
	start(roots: readonly string[], options: StartOptions): Promise<Status>;
	pause(): Promise<Status>;
	resume(): Promise<Status>;
	cancel(): Promise<Status>;
	status(): Promise<Status>;
	groups(scanId: number, query: GroupQuery): Promise<GroupPage>;
	skipped(scanId: number, query: PageQuery): Promise<SkipPage>;

	/** An empty list means the user changed their mind. */
	pickFolders(): Promise<string[]>;
}
