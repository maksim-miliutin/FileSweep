import { useEffect, useId, useState } from "react";

import type { Backend } from "./backend/backend";
import type { GroupView, SkipView, Status, SuggestedRoot } from "./backend/types";
import { BackendError } from "./backend/types";
import { copyText } from "./clipboard";
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
	const [suggestions, setSuggestions] = useState<SuggestedRoot[]>([]);
	const [folderError, setFolderError] = useState<Error | null>(null);

	const { state } = scan.status;
	const scanId = scan.status.scan?.id ?? null;
	const running = isRunning(state);
	const error = scan.error ?? folderError ?? rows.error;
	const offers = suggestions.filter(offer => !folders.includes(offer.path));

	useEffect(() =>
	{
		let current = true;
		backend.suggestedRoots().then(
			(found) =>
			{
				if (current)
				{
					setSuggestions(found);
				}
			},
			(failure: unknown) =>
			{
				if (current)
				{
					setFolderError(asError(failure));
				}
			},
		);

		return () =>
		{
			current = false;
		};
	}, [backend]);

	const add = (picked: readonly string[]): void =>
	{
		setFolders(chosen => [...chosen, ...picked.filter(folder => !chosen.includes(folder))]);
	};

	const addFolders = async (): Promise<void> =>
	{
		try
		{
			add(await backend.pickFolders());
			setFolderError(null);
		}
		catch (failure)
		{
			setFolderError(asError(failure));
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
				{offers.length > 0 && (
					<p>
						{text.suggested}
						{offers.map(offer => (
							<span key={offer.kind}>
								{" "}
								<button
									type="button"
									aria-label={text.addSuggested(text.rootKinds[offer.kind])}
									title={offer.path}
									disabled={running}
									onClick={() =>
									{
										add([offer.path]);
									}}
								>
									{text.rootKinds[offer.kind]}
								</button>
							</span>
						))}
					</p>
				)}
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

			{rows.groups !== null && scanId !== null && (
				<section aria-labelledby="groups-heading">
					<h2 id="groups-heading">{text.groupsHeading}</h2>
					<ul aria-label={text.groupsList}>
						{rows.groups.groups.map(group => (
							// A new scan numbers its groups from zero again; with the scan in the
							// key, a message under a file of the old scan is not carried over.
							<Group
								key={`${String(scanId)}:${String(group.id)}`}
								group={group}
								onReveal={async fileId => backend.reveal(scanId, group.id, fileId)}
							/>
						))}
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

function asError(failure: unknown): Error
{
	return failure instanceof Error ? failure : new Error(String(failure));
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

function Group({ group, onReveal }: { group: GroupView; onReveal: (fileId: number) => Promise<void> })
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
						{" "}
						<FileActions path={file.path} onReveal={async () => onReveal(file.id)} />
					</li>
				))}
			</ul>
		</li>
	);
}

interface Said
{
	failed: boolean;
	words: string;
}

function FileActions({ path, onReveal }: { path: string; onReveal: () => Promise<void> })
{
	const [said, setSaid] = useState<Said | null>(null);
	const pathId = useId();

	const act = async (action: () => Promise<void>, done: string | null): Promise<void> =>
	{
		try
		{
			await action();
			setSaid(done === null ? null : { failed: false, words: done });
		}
		catch (failure)
		{
			setSaid({ failed: true, words: whatWentWrong(failure) });
		}
	};

	return (
		<>
			<span id={pathId} hidden>{path}</span>
			<button type="button" aria-describedby={pathId} onClick={() => void act(onReveal, null)}>
				{text.showInFolder}
			</button>
			{" "}
			<button type="button" aria-describedby={pathId} onClick={() => void act(async () => copyText(path), text.copied)}>
				{text.copyPath}
			</button>
			{said !== null && <span role={said.failed ? "alert" : "status"}>{` ${said.words}`}</span>}
		</>
	);
}

/** The system's own words come with the code, when it gave any. */
function whatWentWrong(failure: unknown): string
{
	if (failure instanceof BackendError && typeof failure.detail.message === "string")
	{
		return `${failure.message}: ${failure.detail.message}`;
	}

	return failure instanceof Error ? failure.message : String(failure);
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
