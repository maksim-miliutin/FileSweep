import type {
	GroupPage,
	GroupQuery,
	HistoryQuery,
	PageQuery,
	SkipPage,
	Status,
	SuggestedRoot,
	UsageHistory,
	UsagePage,
	UsageQuery,
} from "./types";

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
	/** Final once the scan hashes or has ended: files arrive while it walks its folders. */
	history(scanId: number, query: HistoryQuery): Promise<UsageHistory>;
	filesUsed(scanId: number, query: UsageQuery): Promise<UsagePage>;

	/** An empty list means the user changed their mind. */
	pickFolders(): Promise<string[]>;
	suggestedRoots(): Promise<SuggestedRoot[]>;

	/**
	 * Opens the folder of a file of the scan with the file selected. The file
	 * is named by the ids of the page it came in, never by its path.
	 */
	reveal(scanId: number, groupId: number, fileId: number): Promise<void>;
	/** The same for a file of the usage pages, by the id its row carries. */
	revealIndexed(scanId: number, fileId: number): Promise<void>;
}
