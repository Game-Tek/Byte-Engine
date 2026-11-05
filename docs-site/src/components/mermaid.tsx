'use client';

import type { MermaidConfig } from 'mermaid';
import { useTheme } from 'next-themes';
import { useEffect, useId, useState } from 'react';

/** The site's colors for each theme. Mermaid derives colors from them, so they must be literal colors, not CSS variables. */
const palettes = {
	// `accent` is `--be-green` from `app.css`.
	dark: { text: '#e6f2e9', muted: '#8a9a8f', node: '#0e1a12', note: '#141815', noteBorder: '#2a332d', label: '#13301c', accent: '#40d866' },
	// `accent` is the darker green the site uses for text on light pages.
	light: { text: '#0f2415', muted: '#5c6b61', node: '#f1fbf3', note: '#f6f7f6', noteBorder: '#d9dfdb', label: '#dff5e5', accent: '#1f8438' },
};

/**
 * Mermaid's `neo` look with the site's colors: quiet green borders on dark-green nodes, without shadows.
 */
function themeVariables(dark: boolean): MermaidConfig['themeVariables'] {
	const { text, muted, node, note, noteBorder, label, accent } = dark ? palettes.dark : palettes.light;
	// The accent at `alpha` opacity, as an 8-digit hex color.
	const tint = (alpha: number) => accent + Math.round(alpha * 255).toString(16).padStart(2, '0');
	return {
		darkMode: dark,
		fontSize: '14px',
		radius: 10,
		strokeWidth: 1,
		// A plain border replaces the neo gradient.
		nodeBorder: tint(0.45),
		dropShadow: 'none',
		primaryColor: node,
		primaryTextColor: text,
		lineColor: muted,
		textColor: text,
		clusterBkg: tint(0.04),
		clusterBorder: tint(0.35),
		titleColor: accent,
		actorBkg: node,
		actorBorder: tint(0.45),
		actorTextColor: text,
		actorLineColor: tint(0.3),
		signalColor: muted,
		signalTextColor: text,
		noteBkgColor: note,
		noteBorderColor: noteBorder,
		noteTextColor: muted,
		labelBoxBkgColor: label,
		labelBoxBorderColor: tint(0.45),
		labelTextColor: accent,
		loopTextColor: accent,
	};
}

/**
 * Renders a ```mermaid code block, which `remarkMdxMermaid` turns into `<Mermaid chart="..." />`.
 *
 * Mermaid needs the DOM, so the diagram renders on the client, and again whenever the theme changes.
 */
export function Mermaid({ chart }: { chart: string }) {
	const id = `mermaid-${useId().replaceAll(':', '')}`;
	const { resolvedTheme } = useTheme();
	const [svg, setSvg] = useState<string>();

	useEffect(() => {
		let cancelled = false;
		// Load Mermaid only on pages that draw a diagram; it's large.
		void import('mermaid').then(async ({ default: mermaid }) => {
			mermaid.initialize({
				startOnLoad: false,
				securityLevel: 'strict',
				look: 'neo',
				theme: 'base',
				// Mermaid measures labels with this font, so give it the page's resolved font rather than `inherit`.
				fontFamily: getComputedStyle(document.body).fontFamily,
				themeVariables: themeVariables(resolvedTheme === 'dark'),
				// Wrap node labels at the width of their longest line rather than at Mermaid's narrow default.
				flowchart: { wrappingWidth: 320 },
			});
			const { svg } = await mermaid.render(id, chart);
			if (!cancelled) setSvg(svg);
		});
		return () => {
			cancelled = true;
		};
	}, [chart, id, resolvedTheme]);

	if (!svg) {
		// Show the source until the diagram renders, so the content is never missing.
		return <pre className="not-prose overflow-x-auto text-xs">{chart}</pre>;
	}
	return (
		<div
			className="be-mermaid not-prose"
			// Mermaid renders with `securityLevel: 'strict'`, which sanitizes the SVG.
			dangerouslySetInnerHTML={{ __html: svg }}
		/>
	);
}
