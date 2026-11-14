import { describe, expect, it } from 'vitest';
import { safeDestination } from './connect';

describe('safeDestination', () => {
	// Catches a navigation to a script or data address, to plain http off loopback, and to something that is no address.
	it('allows https, and http only on loopback', () => {
		for (const ok of ['https://claude.ai/api/mcp/auth_callback?code=x&state=y', 'http://127.0.0.1:7477/callback?code=x', 'http://localhost:7477/cb', 'http://[::1]:7477/cb']) {
			expect(safeDestination(ok), ok).toBe(true);
		}
		for (const bad of ['javascript:alert(1)', 'data:text/html,<p>x', 'http://claude.ai/cb', 'http://127.0.0.1.evil.example/cb', 'ftp://claude.ai/cb', '/connect', '', 'https://']) {
			expect(safeDestination(bad), bad).toBe(false);
		}
	});
});
