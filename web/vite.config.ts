import adapter from '@sveltejs/adapter-static';
import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vitest/config';

// SvelteKit 3 takes its configuration here (svelte.config.js is no longer read).
const server = 'http://127.0.0.1:7474';

export default defineConfig(({ mode }) => ({
	plugins: [
		sveltekit({
			compilerOptions: {
				// `npm test` runs the suite twice: as the dev server compiles the components, then (mode `test-production`) as
				// `vite build` does. Some code behaves differently between the two: a `{@const}` is read eagerly in dev and
				// lazily in production, so a callback that reads it late sees a value the dev build never shows.
				...(mode === 'test-production' ? { dev: false } : {}),
				// Force runes mode for the project, except for libraries. Can be removed in svelte 6.
				runes: ({ filename }) =>
					filename.split(/[/\\]/).includes('node_modules') ? undefined : true
			},
			// A single-page build: every unknown path is served index.html by reeve-server (kb/ADR-0006 R2). No kit.csp: the
			// server hashes the inline bootstrap script in index.html itself.
			adapter: adapter({
				pages: 'build',
				assets: 'build',
				fallback: 'index.html',
				precompress: false,
				strict: true
			})
		})
	],
	build: {
		// The server's CSP has no `data:` in font-src; emit every asset as a file instead of inlining it.
		assetsInlineLimit: 0
	},
	server: {
		// `npm run dev` talks to a local reeve-server so the session cookie stays first-party.
		proxy: { '/api': server, '/auth/login': server, '/auth/callback': server, '/auth/logout': server, '/auth/status': server }
	},
	// Tests load Svelte's browser build (with the jsdom environment in the file that needs it): under the server build
	// `$effect` never runs, which would hide what #lib/events.svelte.ts does.
	resolve: mode.startsWith('test') ? { conditions: ['browser'] } : undefined,
	test: { include: ['src/**/*.test.ts'], environment: 'node' }
}));
