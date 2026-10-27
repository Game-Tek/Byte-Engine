import { access, readdir, readFile } from 'node:fs/promises';
import { dirname, relative, resolve, sep } from 'node:path';

const scriptDirectory = dirname(new URL(import.meta.url).pathname);
const repositoryRoot = resolve(scriptDirectory, '..');
const siteRoot = resolve(repositoryRoot, 'docs-site/dist/client');
const productionDocsUrl = 'https://byte-engine.0x44491229.dev/docs';
const pages = await collectFiles(siteRoot, '.html');
const failures = new Set();
const anchorCache = new Map();

for (const sourceFile of pages) {
	const sourceRoute = routeForFile(sourceFile);
	const html = await readFile(sourceFile, 'utf8');
	if (sourceRoute.startsWith('/docs/api/') && hasAbsoluteSameSiteLink(html)) {
		failures.add(`${sourceRoute} contains a hardcoded production documentation link`);
	}

	for (const href of linksIn(html)) {
		const destination = new URL(href, `https://byte-engine.invalid${sourceRoute}`);
		if (destination.origin !== 'https://byte-engine.invalid') continue;
		if (isNonPageRoute(destination.pathname)) continue;

		const targetFile = fileForRoute(destination.pathname);
		if (!await exists(targetFile)) {
			failures.add(`${sourceRoute} -> ${href} (missing page)`);
			continue;
		}

		// This proof of concept owns API links. Some existing handwritten pages
		// put headings inside inactive tabs, which are absent from prerendered HTML.
		if (!destination.hash || !destination.pathname.startsWith('/docs/api/')) continue;
		const expectedAnchor = decodeURIComponent(destination.hash.slice(1));
		const anchors = await anchorsIn(targetFile);
		if (!anchors.has(expectedAnchor)) {
			failures.add(`${sourceRoute} -> ${href} (missing #${expectedAnchor})`);
		}
	}
}

// Rust sources link the site three ways: `/docs/` links in doc comments, docs paths passed to
// `online_docs_url`, and hard-coded production URLs, which are rejected so the base URL override applies.
const rustFiles = await collectFiles(resolve(repositoryRoot, 'crates'), '.rs');
let rustLinkCount = 0;
for (const sourceFile of rustFiles) {
	const source = relative(repositoryRoot, sourceFile);
	const lines = (await readFile(sourceFile, 'utf8')).split('\n');
	for (const [index, line] of lines.entries()) {
		const location = `${source}:${index + 1}`;
		for (const url of productionUrlsIn(line)) {
			const path = url.slice(productionDocsUrl.length).replace(/^\//, '');
			failures.add(`${location} hard-codes ${url}; use online_docs_url("${path}") in code or [text](/docs/${path}) in a doc comment`);
		}
		for (const href of rustDocsLinksIn(line)) {
			rustLinkCount += 1;
			const failure = await checkRustDocsLink(href);
			if (failure) failures.add(`${location} -> ${href} (${failure})`);
		}
	}
}

function hasAbsoluteSameSiteLink(html) {
	return /href=["']https:\/\/byte-engine\.0x44491229\.dev\/docs(?:\/|["'])/i.test(html);
}

if (failures.size > 0) {
	console.error(`Found ${failures.size} documentation validation failures:`);
	for (const failure of [...failures].sort()) console.error(`- ${failure}`);
	process.exit(1);
}

console.log(`Checked internal page links and Rust API fragments across ${pages.length} rendered pages, and ${rustLinkCount} documentation links in ${rustFiles.length} Rust files.`);

// Finds production documentation URLs, allowing only the bare base that `online_docs_url` defaults to.
function productionUrlsIn(line) {
	return [...line.matchAll(/https:\/\/byte-engine\.0x44491229\.dev\/docs[^\s"'`<>)\]]*/g)]
		.map((match) => match[0])
		.filter((url) => !(url === productionDocsUrl && line.includes(`"${productionDocsUrl}"`)));
}

// Finds documentation routes in one Rust line: doc-comment links and docs path string literals.
function rustDocsLinksIn(line) {
	const links = [...line.matchAll(/\]\((\/docs\/[^)\s]*)\)/g)].map((match) => match[1]);
	const definition = line.match(/^\s*\/\/[\/!]\s*\[[^\]]+\]:\s*(\/docs\/\S+)/);
	if (definition) links.push(definition[1]);
	// `online_docs_url` takes paths relative to `/docs`, usually through a `*_DOCS_PATH` constant.
	for (const match of line.matchAll(/"((?:use|reference|develop|api)(?:\/[^"\s]*)?)"/g)) {
		links.push(`/docs/${match[1]}`);
	}
	return links;
}

// Returns why a documentation route from Rust doesn't resolve on the built site, or nothing when it does.
async function checkRustDocsLink(href) {
	const destination = new URL(href, 'https://byte-engine.invalid');
	const targetFile = fileForRoute(destination.pathname);
	if (!await exists(targetFile)) return 'missing page';
	if (!destination.hash) return undefined;
	const expectedAnchor = decodeURIComponent(destination.hash.slice(1));
	return (await anchorsIn(targetFile)).has(expectedAnchor) ? undefined : `missing #${expectedAnchor}`;
}

async function collectFiles(directory, extension) {
	const entries = await readdir(directory, { withFileTypes: true });
	const collected = [];

	for (const entry of entries) {
		const path = resolve(directory, entry.name);
		if (entry.isDirectory()) {
			if (entry.name === 'target' || entry.name === 'node_modules') continue;
			collected.push(...await collectFiles(path, extension));
		} else if (entry.isFile() && entry.name.endsWith(extension)) {
			collected.push(path);
		}
	}

	return collected.sort();
}

function routeForFile(file) {
	const path = relative(siteRoot, file).split(sep).join('/');
	if (path === 'index.html') return '/';
	return `/${path.slice(0, -'index.html'.length)}`;
}

function linksIn(html) {
	return [...html.matchAll(/<a\b[^>]*\bhref=(?:"([^"]*)"|'([^']*)')/gi)]
		.map((match) => decodeHtml(match[1] ?? match[2]));
}

function fileForRoute(pathname) {
	const route = decodeURIComponent(pathname).replace(/^\/+/, '');
	if (route === '') return resolve(siteRoot, 'index.html');
	if (route.endsWith('.html')) return resolve(siteRoot, route);
	return resolve(siteRoot, route, 'index.html');
}

function isNonPageRoute(pathname) {
	return pathname.startsWith('/assets/') ||
		pathname.startsWith('/api/') ||
		pathname.endsWith('.md') ||
		pathname.endsWith('.txt');
}

async function anchorsIn(file) {
	if (anchorCache.has(file)) return anchorCache.get(file);

	const html = await readFile(file, 'utf8');
	const anchors = new Set(
		[...html.matchAll(/\b(?:id|name)=(?:"([^"]*)"|'([^']*)')/gi)]
			.map((match) => decodeHtml(match[1] ?? match[2])),
	);
	anchorCache.set(file, anchors);
	return anchors;
}

async function exists(file) {
	try {
		await access(file);
		return true;
	} catch {
		return false;
	}
}

function decodeHtml(value) {
	return value
		.replaceAll('&amp;', '&')
		.replaceAll('&quot;', '"')
		.replaceAll('&#39;', "'")
		.replace(/&#(\d+);/g, (_, codePoint) => String.fromCodePoint(Number(codePoint)));
}
