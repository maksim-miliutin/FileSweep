import js from "@eslint/js";
import stylistic from "@stylistic/eslint-plugin";
import tseslint from "typescript-eslint";

export default tseslint.config(
	{ ignores: ["dist", "src-tauri"] },

	js.configs.recommended,
	...tseslint.configs.strictTypeChecked,
	{
		languageOptions: {
			parserOptions: {
				projectService: true,
				tsconfigRootDir: import.meta.dirname,
			},
		},
	},

	{
		// This file is plain JavaScript, outside the TypeScript project.
		files: ["**/*.js"],
		extends: [tseslint.configs.disableTypeChecked],
	},

	// ESLint is the formatter here as well: tabs and braces on their own
	// line, which Prettier cannot do.
	stylistic.configs.customize({
		indent: "tab",
		braceStyle: "allman",
		quotes: "double",
		semi: true,
		jsx: true,
	}),
);
