import Link from 'fumadocs-core/link';
import { Accordion, Accordions } from 'fumadocs-ui/components/accordion';
import { Callout } from 'fumadocs-ui/components/callout';
import { Card, Cards } from 'fumadocs-ui/components/card';
import { Heading } from 'fumadocs-ui/components/heading';
import { Tab, Tabs } from 'fumadocs-ui/components/tabs';
import { TypeTable, type TypeNode } from 'fumadocs-ui/components/type-table';
import type { ReactNode } from 'react';
import { KINDS, SYMBOL_INDEX_URL, tokenText } from '@/lib/rust-api/items';
import type {
	ChildView,
	FieldView,
	ImplBlockView,
	ItemView,
	LandingView,
	MemberView,
	TraitImplView,
	VariantView,
} from '@/lib/rust-api/view';
import type { ApiReference } from '@/lib/rustdoc/model';
import { Declaration, Deprecated, DocsMarkdown, InlineText, Section, Tokens } from './parts';

/** The API landing page: the crate, its top-level modules, and the indexes agents read. */
export function RustLandingPage({ landing }: { landing: LandingView }) {
	return (
		<>
			<p>
				Every page in this section is generated from the engine source. To change a page, change the doc comment in
				the source; nothing here is edited by hand. For tasks, design notes, and definitions, use{' '}
				<Link href="/docs/use">Use</Link>, <Link href="/docs/develop">Contribute</Link>, and{' '}
				<Link href="/docs/reference">Reference</Link> instead.
			</p>
			<Callout title="Reading this with an agent?">
				Every page has a Markdown version: add <code>.md</code> to its URL. The{' '}
				<a href={SYMBOL_INDEX_URL}>symbol index</a> lists every public item with its kind, signature, summary,
				and Markdown URL, one per line.
			</Callout>
			<Heading as="h2" id="crate">
				Crate
			</Heading>
			<Cards>
				<Card
					title={landing.crate}
					description={landing.version ? `Version ${landing.version}. ${landing.root.summary}` : landing.root.summary}
					href={landing.root.route}
				/>
			</Cards>
			<Heading as="h2" id="modules">
				Modules
			</Heading>
			<Cards>
				{landing.modules.map((module) => (
					<Card
						key={module.route}
						title={module.name}
						description={<InlineText text={module.summary} />}
						href={module.route}
					/>
				))}
			</Cards>
		</>
	);
}

/** A module, type, or trait page. */
export function RustItemPage({ item }: { item: ItemView }) {
	return (
		<>
			{item.kind !== 'module' ? <Declaration tokens={item.declaration} /> : null}
			<Deprecated deprecation={item.deprecation} />
			<DocsMarkdown tree={item.docs} />
			{item.children?.map((group) => (
				<ChildGroup key={group.kind} kind={group.kind} title={group.title} entries={group.entries} />
			))}
			{item.externalReexports?.length ? (
				<>
					<Heading as="h2" id="section-reexports">
						Re-exports
					</Heading>
					<ul>
						{item.externalReexports.map((reexport) => (
							<li key={reexport.name}>
								<code>pub use </code>
								{reexport.href ? (
									<a href={reexport.href}>
										<code>{reexport.source}</code>
									</a>
								) : (
									<code>{reexport.source}</code>
								)}
							</li>
						))}
					</ul>
				</>
			) : null}
			{item.fields?.length ? <Fields fields={item.fields} /> : null}
			{item.variants?.length ? <Variants variants={item.variants} /> : null}
			{item.required?.length ? (
				<MemberList id="required-members" title="Required members" members={item.required} />
			) : null}
			{item.provided?.length ? (
				<MemberList id="provided-members" title="Provided members" members={item.provided} />
			) : null}
			{item.inherent?.length ? <Methods typeName={item.name} blocks={item.inherent} /> : null}
			{item.implementors?.length ? (
				<>
					<Heading as="h2" id="implementors">
						Implementors
					</Heading>
					<ul className="be-rust-impl-list">
						{item.implementors.map((implementor, index) => (
							<li key={index}>
								<code>
									<Tokens tokens={implementor.header} />
								</code>
							</li>
						))}
					</ul>
				</>
			) : null}
			{item.traitImpls?.length ? <TraitImplementations traitImpls={item.traitImpls} /> : null}
			<UsedBy acceptedBy={item.acceptedBy} returnedBy={item.returnedBy} />
		</>
	);
}

/**
 * The page title line of an item: its name, kind chip, and fully qualified path with
 * a link for each parent module.
 */
export function RustItemTitle({ item, title }: { item: ItemView; title: ReactNode }) {
	return (
		<div className="be-rust-title">
			{title}
			<span className="be-rust-kind-badge" data-kind={item.kind}>
				{KINDS[item.kind].label}
			</span>
			<nav aria-label="Module path" className="be-rust-breadcrumbs">
				{item.breadcrumbs.map((crumb) => (
					<span key={crumb.href}>
						<Link href={crumb.href}>{crumb.name}</Link>
						<span aria-hidden>::</span>
					</span>
				))}
				<span>{item.name}</span>
			</nav>
		</div>
	);
}

/** Lists one kind of a module's items: submodules as cards, types as a table, and functions and constants in full. */
function ChildGroup({ kind, title, entries }: { kind: ChildView['kind']; title: string; entries: ChildView[] }) {
	const listed = entries.filter((entry) => !entry.section);
	return (
		<>
			<Heading as="h2" id={`section-${kind}`}>
				{title}
			</Heading>
			{kind === 'module' ? (
				<Cards>
					{listed.map((entry) => (
						<Card
							key={entry.route}
							title={entry.name}
							description={<InlineText text={entry.summary} />}
							href={entry.route}
						/>
					))}
				</Cards>
			) : listed.length ? (
				<ItemTable entries={listed} />
			) : null}
			{entries.map((entry) => (entry.section ? <Section key={entry.route} section={entry.section} /> : null))}
		</>
	);
}

function ItemTable({ entries }: { entries: ChildView[] }) {
	return (
		<div className="be-rust-item-table">
			<table>
				<thead>
					<tr>
						<th>Name</th>
						<th>Description</th>
					</tr>
				</thead>
				<tbody>
					{entries.map((entry) => (
						<tr key={`${entry.name}:${entry.route}`}>
							<td>
								<Link href={entry.route}>
									<code className={entry.deprecated ? 'line-through' : undefined}>{entry.name}</code>
								</Link>
								{entry.reexport ? <span className="be-rust-reexport">re-export</span> : null}
							</td>
							<td>
								<InlineText text={entry.summary} />
							</td>
						</tr>
					))}
				</tbody>
			</table>
		</div>
	);
}

function Fields({ fields }: { fields: FieldView[] }) {
	const type: Record<string, TypeNode> = {};
	for (const field of fields) {
		type[field.name] = {
			type: (
				<code>
					<Tokens tokens={field.type} />
				</code>
			),
			description: field.docs ? <DocsMarkdown tree={field.docs} /> : undefined,
			required: true,
		};
	}
	return (
		<>
			<Heading as="h2" id="fields">
				Fields
			</Heading>
			{/* Anchor targets for links to individual fields. */}
			{fields.map((field) => (
				<span key={field.anchor} id={field.anchor} className="be-rust-anchor" />
			))}
			<TypeTable className="be-rust-type-table" type={type} />
		</>
	);
}

function Variants({ variants }: { variants: VariantView[] }) {
	const type: Record<string, TypeNode> = {};
	for (const variant of variants) {
		type[variant.name] = {
			type: (
				<code>
					<Tokens tokens={variant.declaration} />
				</code>
			),
			description: variant.docs ? <DocsMarkdown tree={variant.docs} /> : undefined,
			required: true,
			parameters: variant.fields.map((field) => ({
				name: field.name,
				description: (
					<>
						<code>
							<Tokens tokens={field.type} />
						</code>
						{field.summary ? (
							<>
								{' '}
								<InlineText text={field.summary} />
							</>
						) : null}
					</>
				),
			})),
		};
	}
	return (
		<>
			<Heading as="h2" id="variants">
				Variants
			</Heading>
			{variants.map((variant) => (
				<span key={variant.anchor} id={variant.anchor} className="be-rust-anchor" />
			))}
			<TypeTable className="be-rust-type-table" type={type} />
		</>
	);
}

/** A method overview table that expands into parameters and return types, then every member in full. */
function Methods({ typeName, blocks }: { typeName: string; blocks: ImplBlockView[] }) {
	const overview: Record<string, TypeNode> = {};
	for (const block of blocks) {
		for (const member of block.members) {
			const key = overview[member.name] ? `${member.name} (${member.anchor})` : member.name;
			overview[key] = memberTypeNode(member);
		}
	}
	return (
		<>
			<Heading as="h2" id="implementations">
				Methods
			</Heading>
			<TypeTable className="be-rust-type-table" type={overview} />
			{blocks.map((block, index) => (
				<div key={index} className="be-rust-impl-block">
					{blocks.length > 1 || isGenericImpl(block, typeName) ? (
						<code className="be-rust-impl-header">
							<Tokens tokens={block.header} />
						</code>
					) : null}
					{block.members.map((member) => (
						<Section key={member.anchor} section={member} />
					))}
				</div>
			))}
		</>
	);
}

function memberTypeNode(member: MemberView): TypeNode {
	return {
		type: member.signature ? (
			<code>
				{member.signature.returns ? <Tokens tokens={member.signature.returns} /> : '()'}
			</code>
		) : (
			<code>{member.kind === 'assoc_const' ? 'const' : 'type'}</code>
		),
		description: (
			<>
				{member.summary ? (
					<p>
						<InlineText text={member.summary} />
					</p>
				) : null}
				<a href={`#${member.anchor}`}>Details</a>
			</>
		),
		deprecated: member.deprecation !== null,
		required: true,
		parameters: member.signature?.parameters.map((parameter) => ({
			name: parameter.name,
			description: (
				<code>
					<Tokens tokens={parameter.type} />
				</code>
			),
		})),
		returns: member.signature?.returns ? (
			<code>
				<Tokens tokens={member.signature.returns} />
			</code>
		) : undefined,
	};
}

function isGenericImpl(block: ImplBlockView, typeName: string): boolean {
	return tokenText(block.header) !== `impl ${typeName}`;
}

function MemberList({ id, title, members }: { id: string; title: string; members: MemberView[] }) {
	return (
		<>
			<Heading as="h2" id={id}>
				{title}
			</Heading>
			{members.map((member) => (
				<Section key={member.anchor} section={member} />
			))}
		</>
	);
}

/** Engine traits expand to their members; standard and external traits are listed compactly. */
function TraitImplementations({ traitImpls }: { traitImpls: TraitImplView[] }) {
	const engine = traitImpls.filter((traitImpl) => traitImpl.origin === 'engine');
	const others = traitImpls.filter((traitImpl) => traitImpl.origin !== 'engine');
	return (
		<>
			<Heading as="h2" id="trait-implementations">
				Trait implementations
			</Heading>
			{engine.length ? (
				<Accordions type="multiple">
					{engine.map((traitImpl, index) => (
						<Accordion
							key={index}
							id={`impl-${traitImpl.traitName}-${index}`}
							title={tokenText(traitImpl.header)}
						>
							<code className="be-rust-impl-header">
								<Tokens tokens={traitImpl.header} />
							</code>
							{traitImpl.members.map((member) => (
								<Section key={member.anchor} section={member} headingLevel="h4" />
							))}
						</Accordion>
					))}
				</Accordions>
			) : null}
			{others.length ? (
				<ul className="be-rust-trait-chips" aria-label="Standard and external traits">
					{others.map((traitImpl, index) => (
						<li key={index} title={tokenText(traitImpl.header)}>
							{traitImpl.traitHref ? (
								<a href={traitImpl.traitHref} rel="noreferrer" target="_blank">
									{traitImpl.traitName}
								</a>
							) : (
								traitImpl.traitName
							)}
							{traitImpl.derived ? <span className="be-rust-derived">derive</span> : null}
						</li>
					))}
				</ul>
			) : null}
		</>
	);
}

function UsedBy({ acceptedBy, returnedBy }: { acceptedBy?: ApiReference[]; returnedBy?: ApiReference[] }) {
	const tabs = [
		acceptedBy?.length ? { label: `Accepted by (${acceptedBy.length})`, references: acceptedBy } : undefined,
		returnedBy?.length ? { label: `Returned by (${returnedBy.length})`, references: returnedBy } : undefined,
	].filter((tab) => tab !== undefined);
	if (tabs.length === 0) return null;
	return (
		<>
			<Heading as="h2" id="used-by">
				Used by
			</Heading>
			<Tabs items={tabs.map((tab) => tab.label)}>
				{tabs.map((tab) => (
					<Tab key={tab.label} value={tab.label}>
						<ul>
							{tab.references.map((reference) => (
								<li key={reference.href}>
									<Link href={reference.href}>
										<code>{reference.label}</code>
									</Link>
									{reference.summary ? (
										<>
											{' — '}
											<InlineText text={reference.summary} />
										</>
									) : null}
								</li>
							))}
						</ul>
					</Tab>
				))}
			</Tabs>
		</>
	);
}
