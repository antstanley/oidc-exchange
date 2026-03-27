#!/usr/bin/env node

/**
 * Lightweight static server with markdown content negotiation.
 *
 * Use this to self-host the built documentation site with support for:
 *   - Accept: text/markdown → returns raw markdown instead of HTML
 *   - .md extension → serves the generated markdown file
 *   - Everything else → serves the static HTML site
 *
 * Usage:
 *   npm run build
 *   node serve.mjs
 *
 * Or with a custom port:
 *   PORT=8080 node serve.mjs
 */

import { createServer } from 'node:http';
import { readFile, stat } from 'node:fs/promises';
import { join, extname } from 'node:path';
import { fileURLToPath } from 'node:url';

const DIST = join(fileURLToPath(import.meta.url), '..', 'dist');

const MIME_TYPES = {
	'.html': 'text/html; charset=utf-8',
	'.css': 'text/css; charset=utf-8',
	'.js': 'application/javascript; charset=utf-8',
	'.json': 'application/json; charset=utf-8',
	'.md': 'text/markdown; charset=utf-8',
	'.svg': 'image/svg+xml',
	'.png': 'image/png',
	'.ico': 'image/x-icon',
	'.woff': 'font/woff',
	'.woff2': 'font/woff2',
	'.xml': 'application/xml; charset=utf-8',
	'.txt': 'text/plain; charset=utf-8',
};

async function tryFile(filePath) {
	try {
		const s = await stat(filePath);
		if (s.isFile()) return filePath;
	} catch {}
	return null;
}

async function resolve(pathname) {
	// Try exact path
	let file = await tryFile(join(DIST, pathname));
	if (file) return file;

	// Try with index.html for directories
	if (!extname(pathname)) {
		file = await tryFile(join(DIST, pathname, 'index.html'));
		if (file) return file;

		// Try .html extension
		file = await tryFile(join(DIST, pathname + '.html'));
		if (file) return file;
	}

	return null;
}

const server = createServer(async (req, res) => {
	try {
		const url = new URL(req.url, `http://${req.headers.host}`);
		const accept = req.headers.accept || '';

		// Content negotiation: Accept header requests markdown
		const wantsMarkdown =
			accept.includes('text/markdown') || accept.includes('application/markdown');

		if (wantsMarkdown && !url.pathname.endsWith('.md')) {
			// Rewrite to .md path
			let mdPath = url.pathname.replace(/\/$/, '');
			if (mdPath === '') mdPath = '/index';
			mdPath += '.md';

			const mdFile = await tryFile(join(DIST, mdPath));
			if (mdFile) {
				const content = await readFile(mdFile, 'utf-8');
				res.writeHead(200, {
					'Content-Type': 'text/markdown; charset=utf-8',
					'X-Content-Source': 'markdown-api',
					'Cache-Control': 'public, max-age=3600',
				});
				return res.end(content);
			}
		}

		// Normal file resolution
		const filePath = await resolve(url.pathname);
		if (!filePath) {
			// 404
			const notFoundPage = await tryFile(join(DIST, '404.html'));
			if (notFoundPage) {
				const content = await readFile(notFoundPage);
				res.writeHead(404, { 'Content-Type': 'text/html; charset=utf-8' });
				return res.end(content);
			}
			res.writeHead(404);
			return res.end('Not Found');
		}

		const ext = extname(filePath);
		const contentType = MIME_TYPES[ext] || 'application/octet-stream';
		const content = await readFile(filePath);

		res.writeHead(200, {
			'Content-Type': contentType,
			'Cache-Control': ext === '.html' || ext === '.md' ? 'public, max-age=3600' : 'public, max-age=86400',
		});
		res.end(content);
	} catch (err) {
		console.error(err);
		res.writeHead(500);
		res.end('Internal Server Error');
	}
});

const port = parseInt(process.env.PORT || '4321', 10);
server.listen(port, () => {
	console.log(`Docs server running at http://localhost:${port}`);
	console.log('');
	console.log('  HTML:     curl http://localhost:' + port + '/getting-started/introduction/');
	console.log('  Markdown: curl http://localhost:' + port + '/getting-started/introduction.md');
	console.log('  Accept:   curl -H "Accept: text/markdown" http://localhost:' + port + '/getting-started/introduction/');
});
