import { createFileRoute } from '@tanstack/react-router';
import { getLLMText } from '@/lib/llm';
import { source } from '@/lib/source';

export const Route = createFileRoute('/llms-full.txt')({
	server: {
		handlers: {
			GET: async () => {
				const pages = await Promise.all(source.getPages().map(getLLMText));

				return new Response(pages.join('\n\n'), {
					headers: { 'Content-Type': 'text/plain; charset=utf-8' },
				});
			},
		},
	},
});
