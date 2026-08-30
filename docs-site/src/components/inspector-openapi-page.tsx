'use client';

import { createCodeUsageGeneratorRegistry } from 'fumadocs-openapi/requests/generators';
import { registerDefault } from 'fumadocs-openapi/requests/generators/all';
import { createOpenAPIPageBase } from 'fumadocs-openapi/ui/base';
import { createShikiFactory } from 'fumadocs-core/highlight/shiki';
import { createHighlighterCore } from 'shiki/core';
import { createJavaScriptRegexEngine } from 'shiki/engine/javascript';
import bash from 'shiki/langs/bash.mjs';
import csharp from 'shiki/langs/csharp.mjs';
import go from 'shiki/langs/go.mjs';
import java from 'shiki/langs/java.mjs';
import javascript from 'shiki/langs/javascript.mjs';
import json from 'shiki/langs/json.mjs';
import python from 'shiki/langs/python.mjs';
import rust from 'shiki/langs/rust.mjs';
import githubDark from 'shiki/themes/github-dark.mjs';
import githubLight from 'shiki/themes/github-light.mjs';

const codeUsages = createCodeUsageGeneratorRegistry();
registerDefault(codeUsages);

// OpenAPI examples use a known set of languages. Keeping that set explicit
// prevents every Shiki grammar from becoming part of the deployed Worker.
const shiki = createShikiFactory({
	init: (options) =>
		createHighlighterCore({
			engine: createJavaScriptRegexEngine(),
			langAlias: options?.langAlias,
			langs: [bash, csharp, go, java, javascript, json, python, rust],
			themes: [githubLight, githubDark],
		}),
});

export const InspectorOpenAPIPage = createOpenAPIPageBase({
	codeUsages,
	playground: {
		// The inspector intentionally has no cross-origin browser surface.
		enabled: false,
	},
	shiki,
	storageKeyPrefix: 'byte-engine-inspector-',
});
