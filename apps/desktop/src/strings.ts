// Every sentence the window shows, in one place, so that another language
// can be put beside this one.

import type { RootKind, SkipReason } from "./backend/types";
import { formatBytes, formatCount, formatDate } from "./format";
import type { UsedBy } from "./useUsage";

function many(count: number, one: string, more: string): string
{
	return `${formatCount(count)} ${count === 1 ? one : more}`;
}

const rootKinds: Record<RootKind, string> = {
	downloads: "Downloads",
	desktop: "Desktop",
	documents: "Documents",
	pictures: "Pictures",
};

/** A last use, by the time it came from. */
const usedHow: Record<UsedBy, string> = {
	changed: "last changed",
	created: "created",
	opened: "last opened",
};

/** Short names for the counts by reason, inside a sentence. */
const reasonNames: Record<SkipReason, string> = {
	link: "links",
	onlineOnly: "online only",
	system: "system files",
	hidden: "hidden",
	special: "not ordinary files",
	notADirectory: "not folders",
	alreadyCovered: "already scanned",
	accessDenied: "access denied",
	inUse: "in use",
	vanished: "gone before reading",
	changed: "changed while reading",
	io: "read errors",
};

const reasons: Record<SkipReason, string> = {
	link: "a link, which is not followed",
	onlineOnly: "kept online only, and not downloaded",
	system: "a system file",
	hidden: "hidden",
	special: "not an ordinary file",
	notADirectory: "not a folder",
	alreadyCovered: "already scanned through another folder",
	accessDenied: "access was denied",
	inUse: "in use by another program",
	vanished: "gone before it could be read",
	changed: "changed while it was being read",
	io: "could not be read",
};

export const text = {
	title: "FileSweep",
	notInABrowser: "This page is the window of the FileSweep application. It does nothing in a browser.",

	foldersHeading: "Folders",
	foldersList: "Folders to scan",
	noFolders: "No folder chosen yet.",
	addFolder: "Add folder",
	removeFolder: (folder: string) => `Remove ${folder}`,
	remove: "Remove",
	includeHidden: "Include hidden files",
	suggested: "Suggested:",
	rootKinds,
	addSuggested: (folder: string) => `Add ${folder}`,
	scan: "Scan",

	scanHeading: "Scan",
	pause: "Pause",
	resume: "Resume",
	cancel: "Cancel",

	idle: "Choose the folders to look through, then start a scan. Scanning only reads: nothing is changed or removed.",
	walking: (filesSeen: number) => `Looking for files: ${many(filesSeen, "file", "files")} found so far.`,
	hashing: (done: number, total: number, bytesDone: number, bytesTotal: number) =>
		`Comparing files: ${formatCount(done)} of ${formatCount(total)} read, ${formatBytes(bytesDone)} of ${formatBytes(bytesTotal)}.`,
	paused: "Paused.",
	stopping: "Stopping…",
	found: (groups: number, reclaimableBytes: number, skipped: number) =>
		`${many(groups, "group", "groups")} of identical files, ${formatBytes(reclaimableBytes)} can be freed, ${many(skipped, "entry", "entries")} skipped.`,
	completed: "Done:",
	cancelled: "Cancelled. The scan did not finish, so there may be more than this:",
	failed: (error: string) => `The scan failed: ${error}`,

	groupsHeading: "Duplicates",
	groupsList: "Groups of identical files",
	group: (files: number, size: number) => `${many(files, "file", "files")} of ${formatBytes(size)} each`,
	canBeFreed: (bytes: number) => `${formatBytes(bytes)} can be freed`,
	unverified: "These may be one file on disk under several names, so no space is promised.",
	alsoNamed: (path: string) => `also named ${path}`,
	linkedElsewhere: "has more names outside the scanned folders",
	showInFolder: "Show in folder",
	copyPath: "Copy path",
	copied: "Path copied",
	groupsShown: (first: number, last: number, total: number) =>
		`Groups ${formatCount(first)}–${formatCount(last)} of ${formatCount(total)}`,
	previousPage: "Previous page",
	nextPage: "Next page",

	usageHeading: "By last use",
	usageNote: "When each file was last used, as far as Windows records it: the latest of when it was changed, opened or created. Windows often does not record opening, so a file may have been used later than it says here.",
	usageYears: "Years of last use",
	year: (year: number, before: boolean) => (before ? `Before ${String(year)}` : String(year)),
	yearHolds: (files: number, bytes: number) => `${many(files, "file", "files")}, ${formatBytes(bytes)}`,
	undated: (files: number) => `Left out, with no date at all: ${many(files, "file", "files")}.`,
	usedIn: (year: number, before: boolean) =>
		(before ? `Files last used before ${String(year)}` : `Files last used in ${String(year)}`),
	filesShown: (first: number, last: number, total: number) =>
		`Files ${formatCount(first)}–${formatCount(last)} of ${formatCount(total)}`,
	lastUsed: (ms: number, how: UsedBy) => `${usedHow[how]} ${formatDate(ms)}`,

	skippedHeading: "Skipped",
	skippedList: "Skipped entries",
	entriesShown: (first: number, last: number, total: number) =>
		`Entries ${formatCount(first)}–${formatCount(last)} of ${formatCount(total)}`,
	reasonNames,
	couldNotRead: (counts: string) => `Could not be read: ${counts}.`,
	leftOutByRule: (counts: string) => `Left out by rule: ${counts}.`,
	reason: (reason: SkipReason, detail: string | null) =>
		detail === null ? reasons[reason] : `${reasons[reason]}: ${detail}`,
};
