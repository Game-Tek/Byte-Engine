import type { OpenAPIV3_2 } from 'fumadocs-openapi';
import { createOpenAPI } from 'fumadocs-openapi/server';

import inspectorSchema from '../../openapi/inspector.json';

export const inspectorOpenAPI = createOpenAPI({
	input: {
		inspector: inspectorSchema as OpenAPIV3_2.Document,
	},
});
