// web/src/lib/relations.ts: the rules behind components/Relations.svelte (kb/ADR-0031). How a link reads is the
// server's `label`; everything decided here depends on `type` and `direction` only, never on that text.
import type { ItemLink, ItemLinkType, ItemType, LinkedItem } from '#lib/api/types.ts';
import { storyHref } from '#lib/drawer.ts';

/** Every item link name. `satisfies` holds this object to the generated union both ways: a name the schema gains, or
 *  one written here that the schema does not have, fails `npm run check` until this line follows the schema. The key
 *  order is the display order of the protocol's table. */
const NAMES = { follows: true, duplicates: true, relates_to: true } satisfies Record<ItemLinkType, true>;
/** The names the Add select offers, in display order. */
export const LINK_TYPES = Object.keys(NAMES) as ItemLinkType[];

/** A name in its forward wording: the wire word with spaces ("relates to"). The one function in the client that words
 *  a name: the Add select shows it, and an activity entry's words call it (format.ts). The server words the forward
 *  side the same way; the smoke test adds a link by this text and reads the same text back as the group's label. */
export const linkTypeText = (t: ItemLinkType): string => t.replaceAll('_', ' ');

/** The hint for each name, with `{level}` for the item's level word. They restate kb/ADR-0031 R1's meanings from the
 *  side of the item on screen; `follows` also says how to state the reverse, because writes are always in the forward
 *  form and the select offers no "is followed by". `satisfies` makes a name the schema gains fail `npm run check`
 *  until it has an entry here; '' is the entry of a name that needs no hint. */
const HINTS = {
	follows: 'This {level} comes after the item you pick. To say what comes after this {level}, add the item link from that item.',
	duplicates: 'This {level} covers the same work as the item you pick.',
	relates_to: ''
} satisfies Record<ItemLinkType, string>;

/** What the Add form says under the name select once `type` is chosen, in the words of the item's level; '' for none. */
export function linkTypeHint(type: ItemLinkType, level: ItemType): string {
	return HINTS[type].replaceAll('{level}', level);
}

export type LinkGroup = { id: string; label: string; links: ItemLink[] };

/** The links as the section lists them: one group per name and side, in the order the server sent them (follows,
 *  duplicates, relates_to; within a name, this item's side first). A group is keyed by `type` and `direction`; its
 *  label is the first member's, which is the server's wording for that side. */
export function groupLinks(links: readonly ItemLink[]): LinkGroup[] {
	const groups = new Map<string, LinkGroup>();
	for (const l of links) {
		const id = `${l.type}-${l.direction}`;
		const g = groups.get(id);
		if (g) g.links.push(l);
		else groups.set(id, { id, label: l.label, links: [l] });
	}
	return [...groups.values()];
}

/** The path of the DELETE that removes `link` as `itemKey` shows it. Writes are always in the forward form (the item
 *  that follows or duplicates comes first), so a link read from the other side (`in`) is removed by naming the other
 *  item first. `relates_to` is always `out`, and the server takes either order for it. */
export function removal(itemKey: string, link: ItemLink): { key: string; type: ItemLinkType; to: string } {
	return link.direction === 'in' ? { key: link.item.key, type: link.type, to: itemKey } : { key: itemKey, type: link.type, to: link.item.key };
}

/** Where a linked item opens: a story in the drawer over the current view (as a blocker's key does), an epic or an
 *  initiative on its page. */
export function linkedHref(url: { readonly href: string }, item: LinkedItem): string {
	const k = encodeURIComponent(item.key);
	if (item.level === 'initiative') return `/initiatives/${k}`;
	if (item.level === 'epic') return `/epics/${k}`;
	return storyHref(url, item.key);
}

/** The host shown beside a web link's title, or null when the URL is not http(s): the server stores nothing else, and
 *  a stored `javascript:` URL must still never become a live link (as `safePrUrl` holds for pull requests). */
export function webHost(url: string): string | null {
	try {
		const u = new URL(url);
		return u.protocol === 'http:' || u.protocol === 'https:' ? u.host : null;
	} catch {
		return null;
	}
}

export type FollowsNote = { key: string; by: string };

/** The two keys of the informational line, shown after `follows` is added between two stories: a `follows` reads like
 *  a dependency and holds nothing back (kb/ADR-0031 R3). Null for every other name and level. */
export function followsNote(type: ItemLinkType, itemKey: string, level: ItemType, other: { key: string; item: ItemType }): FollowsNote | null {
	return type === 'follows' && level === 'story' && other.item === 'story' ? { key: itemKey, by: other.key } : null;
}
