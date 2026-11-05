import { defineConfig, devices } from '@playwright/test';

// The smoke runs against a real reeve-server that scripts/web-smoke.sh builds, seeds and starts; no `webServer` here.
// Tests sign in through the browser against the static OIDC provider, so nothing is injected.
export default defineConfig({
	testDir: 'tests',
	// One worker, in file order: the override test moves a seeded story, and the tests before it read that story.
	fullyParallel: false,
	workers: 1,
	forbidOnly: !!process.env.CI,
	retries: 0,
	reporter: process.env.CI ? [['list'], ['github']] : 'list',
	use: {
		baseURL: process.env.REEVE_URL,
		trace: 'retain-on-failure'
	},
	projects: [{ name: 'chromium', use: { ...devices['Desktop Chrome'] } }]
});
