import { useState } from "react";

import type { Backend } from "./backend/backend";
import type { GroupView, SkipView, Status } from "./backend/types";
import { text } from "./strings";
import { FIRST_ROWS, isRunning, useRows, useScan } from "./useScan";

interface AppProps
{
	backend: Backend;
}

/** Decides nothing itself: every rule lives behind the backend. */
export function App({ backend }: AppProps)
{
	const scan = useScan(backend);
	const rows = useRows(backend, scan.status);
	const [folders, setFolders] = useState<string[]>([]);
	const [includeHidden, setIncludeHidden] = useState(false);
	const [dialogError, setDialogError] = useState<Error | null>(null);

	const { state } = scan.status;
	const running = isRunning(state);
	const error = scan.error ?? dialogError ?? rows.error;

	const addFolders = async (): Promise<void> =>
	{
		try
		{
			const picked = await backend.pickFolders();
			setFolders(chosen => [...chosen, ...picked.filter(folder => !chosen.includes(folder))]);
			setDialogError(null);
		}
		catch (failure)
		{
			setDialogError(failure instanceof Error ? failure : new Error(String(failure)));
		}
	};

	return (
		<main>
			<h1>{text.title}</h1>

			<section aria-labelledby="folders-heading">
				<h2 id="folders-heading">{text.foldersHeading}</h2>
				{folders.length === 0
					? <p>{text.noFolders}</p>
					: (
							<ul aria-label={text.foldersList}>
								{folders.map(folder => (
									<li key={folder}>
										<span>{folder}</span>
										{" "}
										<button
											type="button"
											aria-label={text.removeFolder(folder)}
											disabled={running}
											onClick={() =>
											{
												setFolders(chosen => chosen.filter(other => other !== folder));
											}}
										>
											{text.remove}
										</button>
									</li>
								))}
							</ul>
						)}
				<p>
					<button type="button" disabled={running} onClick={() => void addFolders()}>
						{text.addFolder}
					</button>
					{" "}
					<label>
						<input
							type="checkbox"
							checked={includeHidden}
							disabled={running}
							onChange={(event) =>
							{
								setIncludeHidden(event.target.checked);
							}}
						/>
						{" "}
						{text.includeHidden}
					</label>
				</p>
				<p>
					<button
						type="button"
						disabled={running || folders.length === 0}
						onClick={() =>
						{
							scan.start(folders, { includeHidden });
						}}
					>
						{text.scan}
					</button>
				</p>
			</section>

			<section aria-labelledby="scan-heading">
				<h2 id="scan-heading">{text.scanHeading}</h2>
				<p role="status">{describe(scan.status)}</p>
				<p>
					{state === "scanning" && <button type="button" onClick={scan.pause}>{text.pause}</button>}
					{state === "paused" && <button type="button" onClick={scan.resume}>{text.resume}</button>}
					{" "}
					{(state === "scanning" || state === "paused")
						&& <button type="button" onClick={scan.cancel}>{text.cancel}</button>}
				</p>
				{error !== null && <p role="alert">{error.message}</p>}
			</section>

			{rows.groups !== null && (
				<section aria-labelledby="groups-heading">
					<h2 id="groups-heading">{text.groupsHeading}</h2>
					<ul aria-label={text.groupsList}>
						{rows.groups.groups.map(group => <Group key={group.id} group={group} />)}
					</ul>
					{rows.groups.total > FIRST_ROWS && <p>{text.firstGroups(FIRST_ROWS, rows.groups.total)}</p>}
				</section>
			)}

			{rows.skipped !== null && (
				<section aria-labelledby="skipped-heading">
					<h2 id="skipped-heading">{text.skippedHeading}</h2>
					<ul aria-label={text.skippedList}>
						{rows.skipped.items.map((entry, place) => (
							// One path can be skipped twice, so it cannot be the key alone.
							<Skipped key={`${String(place)}:${entry.path}`} entry={entry} />
						))}
					</ul>
					{rows.skipped.total > FIRST_ROWS && <p>{text.firstSkipped(FIRST_ROWS, rows.skipped.total)}</p>}
				</section>
			)}
		</main>
	);
}

function describe(status: Status): string
{
	const { state, scan } = status;
	if (scan === null)
	{
		return text.idle;
	}

	const progress = scan.phase === "walking"
		? text.walking(scan.filesSeen)
		: text.hashing(scan.filesDone, scan.filesTotal, scan.bytesDone, scan.bytesTotal);
	const found = text.found(scan.groups, scan.reclaimableBytes, scan.skipped);

	switch (state)
	{
		case "idle":
			return text.idle;
		case "scanning":
			return progress;
		case "paused":
			return `${text.paused} ${progress}`;
		case "stopping":
			return text.stopping;
		case "completed":
			return `${text.completed} ${found}`;
		case "cancelled":
			return `${text.cancelled} ${found}`;
		case "failed":
			return text.failed(scan.error ?? "");
	}
}

function Group({ group }: { group: GroupView })
{
	return (
		<li>
			<p>
				{group.independence === "verified"
					? `${text.group(group.files.length, group.size)}, ${text.canBeFreed(group.reclaimableBytes)}.`
					: `${text.group(group.files.length, group.size)}. ${text.unverified}`}
			</p>
			<ul>
				{group.files.map(file => (
					<li key={file.id}>
						{file.path}
						{file.otherPaths.map(other => <span key={other}>{`, ${text.alsoNamed(other)}`}</span>)}
						{file.linkedElsewhere && <span>{` (${text.linkedElsewhere})`}</span>}
					</li>
				))}
			</ul>
		</li>
	);
}

function Skipped({ entry }: { entry: SkipView })
{
	return (
		<li>
			{entry.path}
			{": "}
			{text.reason(entry.reason, entry.detail)}
		</li>
	);
}
