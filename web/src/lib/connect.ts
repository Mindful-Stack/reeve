// Helpers of the approval page (web/src/routes/connect).

const LOOPBACK = new Set(['127.0.0.1', 'localhost', '[::1]']);

/** Whether the page may navigate to `address`, the app's redirect address as the server answered it: an `https:`
 *  address, or an `http:` one on a loopback host. Only an instance on the static provider (local development and the
 *  smoke) can list an http address; everywhere else the server answers https. Anything else is refused, so a server
 *  bug can never make this page run a `javascript:` address. */
export function safeDestination(address: string): boolean {
	let url: URL;
	try {
		url = new URL(address);
	} catch {
		return false;
	}
	if (url.protocol === 'https:') return url.hostname !== '';
	return url.protocol === 'http:' && LOOPBACK.has(url.hostname);
}
