import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

// Not `new URL(name, import.meta.url)`: Vite takes that form for an asset of
// the page and refuses a file outside the project.
const folder = resolve(dirname(fileURLToPath(import.meta.url)), "../../../contract");

/**
 * An example from the contract folder at the root of the repository: what the
 * Rust side is tested against as well.
 */
export function contract(name: string): unknown
{
	return JSON.parse(readFileSync(resolve(folder, name), "utf8")) as unknown;
}

/** The same, for an example that is a JSON object. */
export function contractObject(name: string): Record<string, unknown>
{
	const example = contract(name);
	if (typeof example !== "object" || example === null || Array.isArray(example))
	{
		throw new Error(`${name} is not a JSON object`);
	}

	return example as Record<string, unknown>;
}
