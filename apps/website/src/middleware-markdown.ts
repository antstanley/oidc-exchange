/**
 * Astro middleware for markdown content negotiation.
 *
 * When a request includes `Accept: text/markdown` or `Accept: application/markdown`,
 * or when the URL ends with `.md`, this middleware returns the raw markdown content
 * instead of the rendered HTML page.
 *
 * This works in dev mode and with SSR adapters (e.g., @astrojs/node). For static
 * deployments, the build integration generates `.md` files in the output directory
 * that can be served directly by any static host.
 */

import { defineMiddleware } from 'astro:middleware';

// Bundle all markdown source files at build time via Vite's import.meta.glob.
// The `query: '?raw'` option gives us the raw file content as strings.
const markdownFiles: Record<string, string> = import.meta.glob(
	'./content/docs/**/*.{md,mdx}',
	{ query: '?raw', import: 'default', eager: true },
);

function stripFrontmatter(content: string): string {
	if (!content.startsWith('---')) return content;
	const end = content.indexOf('\n---', 3);
	if (end === -1) return content;
	return content.slice(end + 4).trim();
}

function resolveMarkdown(urlPath: string): string | null {
	// Normalize: /getting-started/introduction/ → getting-started/introduction
	let clean = urlPath.replace(/^\//, '').replace(/\/$/, '').replace(/\.md$/, '');
	if (clean === '') clean = 'index';

	// Try to find the source file — glob keys look like:
	//   ./content/docs/getting-started/introduction.md
	const candidates = [
		`./content/docs/${clean}.md`,
		`./content/docs/${clean}.mdx`,
	];

	for (const key of candidates) {
		if (key in markdownFiles) {
			return stripFrontmatter(markdownFiles[key]);
		}
	}
	return null;
}

export const onRequest = defineMiddleware(async (context, next) => {
	const url = new URL(context.request.url);
	const accept = context.request.headers.get('accept') || '';

	const wantsMarkdown =
		accept.includes('text/markdown') ||
		accept.includes('application/markdown') ||
		url.pathname.endsWith('.md');

	if (!wantsMarkdown) {
		return next();
	}

	const markdown = resolveMarkdown(url.pathname);
	if (!markdown) {
		return next();
	}

	return new Response(markdown, {
		status: 200,
		headers: {
			'Content-Type': 'text/markdown; charset=utf-8',
			'X-Content-Source': 'markdown-api',
			'Cache-Control': 'public, max-age=3600',
		},
	});
});
