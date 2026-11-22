// Every sentence the window shows, in one place, so that another language
// can be put beside this one.

import type { RootKind, SkipReason } from "./backend/types";
import { formatBytes, formatCount } from "./format";

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
	firstGroups: (shown: number, total: number) =>
		`Showing the first ${formatCount(shown)} of ${many(total, "group", "groups")}.`,

	skippedHeading: "Skipped",
	skippedList: "Skipped entries",
	firstSkipped: (shown: number, total: number) =>
		`Showing the first ${formatCount(shown)} of ${many(total, "entry", "entries")}.`,
	reason: (reason: SkipReason, detail: string | null) =>
		detail === null ? reasons[reason] : `${reasons[reason]}: ${detail}`,
};
