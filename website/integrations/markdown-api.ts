/**
 * Astro integration that enables serving raw markdown for agent consumption.
 *
 * Two mechanisms:
 *
 * 1. **Build-time**: Generates stripped-frontmatter `.md` files in the output
 *    directory alongside the HTML pages. Any static host can serve these —
 *    agents request `/getting-started/introduction.md` instead of
 *    `/getting-started/introduction/`.
 *
 * 2. **Runtime (dev + SSR)**: Injects middleware that intercepts requests with
 *    `Accept: text/markdown` or `Accept: application/markdown` headers and
 *    returns the raw markdown content. This enables content negotiation on the
 *    same URL — browsers get HTML, agents get markdown.
 */

import type { AstroIntegration } from 'astro';
import { readFileSync, writeFileSync, mkdirSync, readdirSync, statSync } from 'node:fs';
import { join, dirname, relative, extname } from 'node:path';
import { fileURLToPath } from 'node:url';

function stripFrontmatter(content: string): string {
	if (!content.startsWith('---')) return content;
	const end = content.indexOf('\n---', 3);
	if (end === -1) return content;
	return content.slice(end + 4).trim();
}

/** Recursively find all files matching extensions in a directory. */
function walkDir(dir: string, exts: string[]): string[] {
	const results: string[] = [];
	for (const entry of readdirSync(dir)) {
		const full = join(dir, entry);
		const stat = statSync(full);
		if (stat.isDirectory()) {
			results.push(...walkDir(full, exts));
		} else if (exts.includes(extname(full))) {
			results.push(full);
		}
	}
	return results;
}

export default function markdownApi(): AstroIntegration {
	let contentDir: string;

	return {
		name: 'markdown-api',
		hooks: {
			'astro:config:setup': ({ addMiddleware, logger }) => {
				// Inject the content-negotiation middleware for dev + SSR
				addMiddleware({
					entrypoint: new URL('../src/middleware-markdown.ts', import.meta.url).pathname,
					order: 'pre',
				});
				logger.info('Markdown content negotiation middleware registered');
			},

			'astro:config:done': ({ config }) => {
				// Resolve the content docs directory from the project root
				contentDir = fileURLToPath(new URL('src/content/docs/', config.root));
			},

			'astro:build:done': ({ dir, logger }) => {
				const outDir = fileURLToPath(dir);
				const files = walkDir(contentDir, ['.md', '.mdx']);
				let count = 0;

				for (const file of files) {
					const rel = relative(contentDir, file);
					const source = readFileSync(file, 'utf-8');
					const markdown = stripFrontmatter(source);

					// Map to output path: getting-started/introduction.md
					// .mdx files become .md in the output
					const outPath = rel.replace(/\.mdx$/, '.md');
					const outFile = join(outDir, outPath);

					mkdirSync(dirname(outFile), { recursive: true });
					writeFileSync(outFile, markdown, 'utf-8');
					count++;
				}

				logger.info(`Generated ${count} markdown files for agent consumption`);
			},
		},
	};
}
