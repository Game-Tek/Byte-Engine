import { createFileRoute, notFound } from '@tanstack/react-router';
import { rustApiModel } from '@/lib/rust-api/load';
import { renderSymbolIndex } from '@/lib/rust-api/markdown';

export const Route = createFileRoute('/docs/api/latest/symbols.txt')({
	server: {
		handlers: {
			GET: () => {
				if (!rustApiModel) throw notFound();
				return new Response(renderSymbolIndex(rustApiModel), {
					headers: { 'Content-Type': 'text/plain; charset=utf-8' },
				});
			},
		},
	},
});
