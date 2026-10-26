import Link from 'fumadocs-core/link';
import { CodeBlock, Pre } from 'fumadocs-ui/components/codeblock';
import { Heading } from 'fumadocs-ui/components/heading';
import { Callout } from 'fumadocs-ui/components/callout';
import defaultMdxComponents from 'fumadocs-ui/mdx';
import { toJsxRuntime } from 'hast-util-to-jsx-runtime';
import type { ReactNode } from 'react';
import { Fragment, jsx, jsxs } from 'react/jsx-runtime';
import type { ApiDeprecation, Token } from '@/lib/rustdoc/model';
import type { DocsTree, SectionView } from '@/lib/rust-api/view';

/** Renders declaration tokens with highlighting roles, linking every referenced item. */
export function Tokens({ tokens }: { tokens: Token[] }) {
	return (
		<>
			{tokens.map((token, index) => {
				const className = token.kind ? `be-tok-${token.kind}` : undefined;
				return token.href ? (
					<Link key={index} className={`${className ?? ''} be-tok-link`} href={token.href}>
						{token.text}
					</Link>
				) : (
					<span key={index} className={className}>
						{token.text}
					</span>
				);
			})}
		</>
	);
}

/** Renders a Rust declaration in a copyable code block. */
export function Declaration({ tokens }: { tokens: Token[] }) {
	if (tokens.length === 0) return null;
	return (
		<CodeBlock className="be-rust-declaration">
			<Pre>
				<code>
					{/* Fumadocs code blocks style one `span.line` per line, the structure Shiki emits. */}
					{splitLines(tokens).map((line, index) => (
						<span key={index} className="line">
							<Tokens tokens={line} />
						</span>
					))}
				</code>
			</Pre>
		</CodeBlock>
	);
}

function splitLines(tokens: Token[]): Token[][] {
	const lines: Token[][] = [[]];
	for (const token of tokens) {
		const parts = token.text.split('\n');
		parts.forEach((text, index) => {
			if (index > 0) lines.push([]);
			if (text) lines.at(-1)!.push({ ...token, text });
		});
	}
	return lines;
}

/** Renders a plain-text summary, showing its `backtick` spans as inline code. */
export function InlineText({ text }: { text: string }) {
	return (
		<>
			{text.split(/(`[^`]+`)/).map((part, index) =>
				part.startsWith('`') && part.endsWith('`') && part.length > 1 ? (
					<code key={index}>{part.slice(1, -1)}</code>
				) : (
					part
				),
			)}
		</>
	);
}

/** Renders doc comment Markdown that the server already converted and highlighted. */
export function DocsMarkdown({ tree }: { tree: DocsTree | null }) {
	if (!tree) return null;
	return toJsxRuntime(tree, {
		Fragment,
		jsx,
		jsxs,
		components: defaultMdxComponents,
	}) as ReactNode;
}

export function Deprecated({ deprecation }: { deprecation: ApiDeprecation | null }) {
	if (!deprecation) return null;
	return (
		<Callout type="warn" title={deprecation.since ? `Deprecated since ${deprecation.since}` : 'Deprecated'}>
			{deprecation.note ?? 'Avoid this item in new code.'}
		</Callout>
	);
}

export function SourceLink({ href }: { href?: string }) {
	if (!href) return null;
	return (
		<a className="be-rust-source-link" href={href} rel="noreferrer" target="_blank">
			Source
		</a>
	);
}

/** One member, or one function, constant, or macro on its module page: its anchor, declaration, and docs. */
export function Section({ section, headingLevel = 'h3' }: { section: SectionView; headingLevel?: 'h3' | 'h4' }) {
	return (
		<section className="be-rust-member">
			<div className="be-rust-member-heading">
				<Heading as={headingLevel} id={section.anchor}>
					<code>{section.name}</code>
				</Heading>
				<SourceLink href={section.sourceUrl} />
			</div>
			<Declaration tokens={section.declaration} />
			<Deprecated deprecation={section.deprecation} />
			<DocsMarkdown tree={section.docs} />
		</section>
	);
}
