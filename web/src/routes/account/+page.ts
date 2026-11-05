// web/src/routes/account/+page.ts: Appearance lands with the personal theme; until then /account opens Tokens.
import { redirect } from '@sveltejs/kit';
export const load = () => redirect(307, '/tokens');
