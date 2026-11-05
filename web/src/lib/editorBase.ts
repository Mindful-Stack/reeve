// The base an open text editor saves against: the row version it started from and that field's text at the time.

export type EditorBase = { version: number; text: string };

/** On every reload while the editor is open: when the server's text for THIS field still equals the text the editor
 *  started from, nobody changed the field, so the base advances to the server's version (an edit to another field, a
 *  move or the user's own write elsewhere then raises no false conflict). When the field changed, the base stays, so the
 *  save conflicts instead of overwriting a colleague's text. */
export function followBase(base: EditorBase, server: EditorBase): EditorBase {
	return server.text === base.text ? { version: server.version, text: base.text } : base;
}

/** After a `Conflict` only (the user has been told and the row was reloaded): the base becomes what is loaded now, so
 *  trying again saves over the text the user has been shown. Never called on any other failure. */
export function rebase(server: EditorBase): EditorBase {
	return { ...server };
}
