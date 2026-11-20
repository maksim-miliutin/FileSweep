import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";

// Testing Library unmounts by itself only when the test runner's functions
// are globals, and here they are imported.
afterEach(cleanup);
