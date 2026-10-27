import { createFileRoute } from '@tanstack/react-router';
import { llms } from 'fumadocs-core/source';
import { SYMBOL_INDEX_URL } from '@/lib/rust-api/items';
import { source } from '@/lib/source';

// Point agents at the one-line-per-symbol Rust API index before the page list.
const symbolIndexNote = `Rust API symbol index, one public item per line: ${SYMBOL_INDEX_URL}\n\n`;

export const Route = createFileRoute('/llms.txt')({
	server: {
		handlers: {
			GET: () =>
				new Response(symbolIndexNote + llms(source).index(), {
					headers: { 'Content-Type': 'text/plain; charset=utf-8' },
				}),
		},
	},
});
