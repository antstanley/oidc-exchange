// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';
import svelte from '@astrojs/svelte';
import markdownApi from './integrations/markdown-api.js';

// https://astro.build/config
export default defineConfig({
	site: 'https://oidc-exchange.dev',
	redirects: {
		'/': '/getting-started/introduction/',
	},
	integrations: [
		starlight({
			title: 'oidc-exchange',
			description: 'A Rust service that validates OIDC tokens and exchanges them for self-issued JWTs.',
			social: [
				{ icon: 'github', label: 'GitHub', href: 'https://github.com/example/oidc-exchange' },
			],
			editLink: {
				baseUrl: 'https://github.com/example/oidc-exchange/edit/main/website/',
			},
			sidebar: [
				{
					label: 'Getting Started',
					items: [
						{ slug: 'getting-started/introduction' },
						{ slug: 'getting-started/installation' },
						{ slug: 'getting-started/quick-start' },
						{ slug: 'getting-started/why-oidc-exchange' },
					],
				},
				{
					label: 'Guides',
					items: [
						{ slug: 'guides/configuration' },
						{ slug: 'guides/providers' },
						{ slug: 'guides/api-reference' },
						{ slug: 'guides/client-integration' },
						{ slug: 'guides/nodejs' },
						{ slug: 'guides/python' },
						{ slug: 'guides/docker' },
					],
				},
				{
					label: 'Deployment',
					items: [
						{ slug: 'deployment/overview' },
						{
							label: 'AWS',
							collapsed: false,
							items: [
								{ slug: 'deployment/aws-lambda' },
								{ slug: 'deployment/ecs-fargate' },
							],
						},
						{
							label: 'Linux Server',
							collapsed: false,
							items: [
								{ slug: 'deployment/linux-server' },
								{ slug: 'deployment/linux-postgres' },
								{ slug: 'deployment/linux-sqlite' },
							],
						},
						{ slug: 'deployment/container' },
					],
				},
				{
					label: 'Architecture',
					items: [
						{ slug: 'architecture/overview' },
						{ slug: 'architecture/adapters' },
					],
				},
				{
					label: 'Contributing',
					items: [
						{ slug: 'contributing/development' },
					],
				},
			],
		}),
		svelte(),
		markdownApi(),
	],
});
