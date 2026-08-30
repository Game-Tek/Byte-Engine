import {
	createCsrfMiddleware,
	createMiddleware,
	createStart,
} from '@tanstack/react-start';
import { isMarkdownPreferred } from 'fumadocs-core/negotiation';
import { getLLMText } from '@/lib/llm';
import { source } from '@/lib/source';

const markdownMiddleware = createMiddleware().server(
	async ({ next, pathname, request }) => {
		if (
			request.method !== 'GET' ||
			!isMarkdownPreferred(request) ||
			(pathname !== '/docs' && !pathname.startsWith('/docs/'))
		) {
			return next();
		}

		const slugs = pathname
			.slice('/docs'.length)
			.split('/')
			.filter(Boolean);
		const page = source.getPage(slugs);
		if (!page) return next();

		return new Response(await getLLMText(page), {
			headers: {
				'Content-Type': 'text/markdown; charset=utf-8',
				Vary: 'Accept',
			},
		});
	},
);

const csrfMiddleware = createCsrfMiddleware({
	filter: ({ handlerType }) => handlerType === 'serverFn',
});

export const startInstance = createStart(() => ({
	requestMiddleware: [csrfMiddleware, markdownMiddleware],
}));
