import { access, readdir, readFile } from 'node:fs/promises';
import { dirname, relative, resolve, sep } from 'node:path';

const scriptDirectory = dirname(new URL(import.meta.url).pathname);
const siteRoot = resolve(scriptDirectory, '../docs-site/dist/client');
const pages = await collectHtmlPages(siteRoot);
const failures = new Set();
const anchorCache = new Map();

for (const sourceFile of pages) {
	const sourceRoute = routeForFile(sourceFile);
	const html = await readFile(sourceFile, 'utf8');
	if (sourceRoute.startsWith('/docs/api/') && hasInlineImplementationSignature(html)) {
		failures.add(`${sourceRoute} renders an implementation signature as inline code`);
	}
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

function hasInlineImplementationSignature(html) {
	return /<li>\s*<p><code>(?:(?:unsafe|async)\s+)*(?:fn\s+\w+\(|const\s+(?:fn\s+)?\w+|type\s+\w+)/i.test(html);
}

function hasAbsoluteSameSiteLink(html) {
	return /href=["']https:\/\/byte-engine\.0x44491229\.dev\/docs(?:\/|["'])/i.test(html);
}

if (failures.size > 0) {
	console.error(`Found ${failures.size} documentation validation failures:`);
	for (const failure of [...failures].sort()) console.error(`- ${failure}`);
	process.exit(1);
}

console.log(`Checked internal page links and Rust API fragments across ${pages.length} rendered pages.`);

async function collectHtmlPages(directory) {
	const entries = await readdir(directory, { withFileTypes: true });
	const collected = [];

	for (const entry of entries) {
		const path = resolve(directory, entry.name);
		if (entry.isDirectory()) {
			collected.push(...await collectHtmlPages(path));
		} else if (entry.isFile() && entry.name.endsWith('.html')) {
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
		pathname === '/llms.txt' ||
		pathname === '/llms-full.txt';
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
