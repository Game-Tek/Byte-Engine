'use client';

import { createCodeUsageGeneratorRegistry } from 'fumadocs-openapi/requests/generators';
import { registerDefault } from 'fumadocs-openapi/requests/generators/all';
import { createOpenAPIPage } from 'fumadocs-openapi/ui';

const codeUsages = createCodeUsageGeneratorRegistry();
registerDefault(codeUsages);

export const InspectorOpenAPIPage = createOpenAPIPage({
	codeUsages,
	playground: {
		// The inspector intentionally has no cross-origin browser surface.
		enabled: false,
	},
	storageKeyPrefix: 'byte-engine-inspector-',
});
