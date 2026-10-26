import type { ApiItem, ApiMember, ApiModel, Token } from '@/lib/rustdoc/model';
import { describe, KIND_ORDER, KINDS, moduleChildren, SYMBOL_INDEX_URL, tokenText } from './items';

/**
 * Renders a Rust API page as Markdown for agents: the same content as the HTML page,
 * with plain signatures, tables instead of interactive components, and links to the
 * `.md` version of every linked page. `key` is `null` for the crate landing page.
 */
export function renderRustPageMarkdown(model: ApiModel, key: string | null): string {
	if (key === null) return landingMarkdown(model);
	const item = model.items[key];
	return item.kind === 'module' ? moduleMarkdown(model, item) : itemMarkdown(item);
}

/**
 * Lists every public item and member on one line each, so an agent can find an API
 * with one fetch and a text search: `path | kind | signature | summary | markdown URL`.
 */
export function renderSymbolIndex(model: ApiModel): string {
	const lines = [
		`# ${model.crate} ${model.version ?? ''} symbol index`.trimEnd(),
		'# One public item or member per line: path | kind | signature | summary | Markdown URL',
		'',
	];
	const items = Object.values(model.items).sort((left, right) => left.key.localeCompare(right.key));
	for (const item of items) {
		lines.push(symbolLine(item.key, KINDS[item.kind].label, headline(item.declaration), describe(item), item.route));
		const members = [
			...(item.inherent ?? []).flatMap((block) => block.members),
			...(item.required ?? []),
			...(item.provided ?? []),
		];
		for (const member of members) {
			lines.push(
				symbolLine(`${item.key}::${member.name}`, memberLabel(member), headline(member.declaration), member.summary, `${item.route}#${member.anchor}`),
			);
		}
		for (const field of item.fields ?? []) {
			lines.push(symbolLine(`${item.key}::${field.name}`, 'field', `${field.name}: ${tokenText(field.type)}`, field.summary, `${item.route}#${field.anchor}`));
		}
		for (const variant of item.variants ?? []) {
			lines.push(symbolLine(`${item.key}::${variant.name}`, 'variant', headline(variant.declaration), variant.summary, `${item.route}#${variant.anchor}`));
		}
	}
	return `${lines.join('\n')}\n`;
}

function symbolLine(path: string, kind: string, signature: string, summary: string, route: string): string {
	return [path, kind, signature, summary, markdownUrl(route)].map((cell) => cell.replaceAll('|', '\\|')).join(' | ');
}

function landingMarkdown(model: ApiModel): string {
	const root = model.items[model.root];
	const modules = moduleChildren(model, root).filter(({ item }) => item.kind === 'module');
	return [
		`The \`${model.crate}\` crate${model.version ? `, version ${model.version}` : ''}. ${describe(root)}`,
		'',
		`Every public item, with its signature and summary, is listed one per line in the symbol index: ${SYMBOL_INDEX_URL}`,
		'',
		`- [\`${root.name}\`](${markdownUrl(root.route)}): the crate root.`,
		...modules.map(({ item }) => `- [\`${item.name}\`](${markdownUrl(item.route)}): ${describe(item)}`),
	].join('\n');
}

function moduleMarkdown(model: ApiModel, module: ApiItem): string {
	const sections = [heading(module), module.docs ? linkDocs(module.docs) : ''];
	const children = moduleChildren(model, module);
	for (const kind of KIND_ORDER) {
		const group = children.filter(({ item }) => item.kind === kind);
		if (group.length === 0) continue;
		sections.push(`## ${KINDS[kind].title}`);
		const listed = group.filter(({ item, reexport }) => item.page || reexport || item.parent !== module.key);
		if (listed.length) {
			sections.push(
				table(
					['Name', 'Description'],
					listed.map(({ item, name, reexport }) => [
						`[\`${name}\`](${markdownUrl(item.route)})${reexport ? ' (re-export)' : ''}`,
						describe(item),
					]),
				),
			);
		}
		for (const { item } of group.filter((entry) => !listed.includes(entry))) {
			sections.push(`### \`${item.name}\``, codeBlock(item.declaration), shiftHeadings(linkDocs(item.docs), 3));
		}
	}
	if (module.externalReexports?.length) {
		sections.push('## Re-exports', module.externalReexports.map((reexport) => `- \`pub use ${reexport.source};\``).join('\n'));
	}
	return joinSections(sections);
}

function itemMarkdown(item: ApiItem): string {
	const sections = [heading(item), `Import: \`use ${item.path.join('::')};\``];
	if (item.source) sections.push(`Source: \`${item.source.file}:${item.source.line}\``);
	if (item.deprecation) sections.push(`Deprecated${item.deprecation.since ? ` since ${item.deprecation.since}` : ''}. ${item.deprecation.note ?? ''}`.trim());
	sections.push(codeBlock(item.declaration));
	if (item.docs) sections.push(shiftHeadings(linkDocs(item.docs), 1));

	if (item.fields?.length) {
		sections.push(
			'## Fields',
			table(
				['Field', 'Type', 'Description'],
				item.fields.map((field) => [`\`${field.name}\``, `\`${tokenText(field.type)}\``, field.summary]),
			),
		);
	}
	if (item.variants?.length) {
		sections.push(
			'## Variants',
			table(
				['Variant', 'Description'],
				item.variants.map((variant) => [`\`${tokenText(variant.declaration)}\``, variant.summary]),
			),
		);
	}
	if (item.required?.length) sections.push('## Required members', ...item.required.map(memberMarkdown));
	if (item.provided?.length) sections.push('## Provided members', ...item.provided.map(memberMarkdown));
	if (item.inherent?.length) {
		sections.push('## Methods');
		for (const block of item.inherent) {
			if (item.inherent.length > 1) sections.push(`\`${tokenText(block.header)}\``);
			sections.push(...block.members.map(memberMarkdown));
		}
	}
	if (item.implementors?.length) {
		sections.push('## Implementors', item.implementors.map((implementor) => `- \`${tokenText(implementor.header)}\``).join('\n'));
	}
	if (item.traitImpls?.length) {
		const engine = item.traitImpls.filter((traitImpl) => traitImpl.origin === 'engine');
		const others = item.traitImpls.filter((traitImpl) => traitImpl.origin !== 'engine');
		sections.push('## Trait implementations');
		if (engine.length) sections.push(engine.map((traitImpl) => `- \`${tokenText(traitImpl.header)}\``).join('\n'));
		if (others.length) {
			sections.push(
				`Also implements: ${others.map((traitImpl) => `\`${traitImpl.traitName}\`${traitImpl.derived ? ' (derived)' : ''}`).join(', ')}.`,
			);
		}
	}
	if (item.acceptedBy?.length) sections.push('## Accepted by', referenceList(item.acceptedBy));
	if (item.returnedBy?.length) sections.push('## Returned by', referenceList(item.returnedBy));
	return joinSections(sections);
}

function memberMarkdown(member: ApiMember): string {
	return joinSections([
		`### \`${member.name}\``,
		codeBlock(member.declaration),
		member.deprecation ? `Deprecated. ${member.deprecation.note ?? ''}`.trim() : '',
		shiftHeadings(linkDocs(member.docs), 3),
	]);
}

function heading(item: ApiItem): string {
	return `${KINDS[item.kind].name} \`${item.key}\``;
}

function referenceList(references: NonNullable<ApiItem['acceptedBy']>): string {
	return references.map((reference) => `- [\`${reference.label}\`](${markdownUrl(reference.href)})${reference.summary ? `: ${reference.summary}` : ''}`).join('\n');
}

function memberLabel(member: ApiMember): string {
	return { method: 'method', assoc_const: 'const', assoc_type: 'type' }[member.kind];
}

/** The first line of a declaration, without an opening brace, as a one-line signature. */
function headline(tokens: Token[]): string {
	const text = tokenText(tokens);
	const firstLine = text.split('\n')[0].replace(/\s*\{\s*$/, '');
	return text.includes('\n') && !/[;)]$/.test(firstLine) ? `${firstLine} { … }` : firstLine;
}

function codeBlock(tokens: Token[]): string {
	return tokens.length ? `\`\`\`rust\n${tokenText(tokens)}\n\`\`\`` : '';
}

function table(header: string[], rows: string[][]): string {
	const cell = (value: string) => value.replaceAll('|', '\\|').replace(/\s*\n\s*/g, ' ');
	return [
		`| ${header.join(' | ')} |`,
		`| ${header.map(() => '---').join(' | ')} |`,
		...rows.map((row) => `| ${row.map(cell).join(' | ')} |`),
	].join('\n');
}

/** Points site links at the Markdown version of each page. */
function linkDocs(markdown: string): string {
	return markdown.replace(/\]\((\/docs\/[^)\s#]*)(#[^)\s]*)?\)/g, (_, path: string, hash = '') => `](${markdownUrl(path + hash)})`);
}

/** Returns the `.md` URL of a site route, keeping any anchor. */
export function markdownUrl(route: string): string {
	if (!route.startsWith('/docs')) return route;
	const [path, hash] = route.split('#');
	const markdownPath = path === '/docs' ? '/docs/index.md' : `${path.replace(/\/$/, '')}.md`;
	return hash ? `${markdownPath}#${hash}` : markdownPath;
}

/** Moves Markdown headings down by `shift` levels, outside code blocks. */
function shiftHeadings(markdown: string, shift: number): string {
	let inFence = false;
	return markdown
		.split('\n')
		.map((line) => {
			if (/^\s*(```|~~~)/.test(line)) inFence = !inFence;
			if (inFence) return line;
			return line.replace(/^(#{1,6})(\s)/, (_, hashes: string, space: string) => `${'#'.repeat(Math.min(6, hashes.length + shift))}${space}`);
		})
		.join('\n');
}

function joinSections(sections: string[]): string {
	return sections.filter((section) => section.trim() !== '').join('\n\n');
}
