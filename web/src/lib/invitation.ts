// Invitations, as people paste them: the link an admin sent, or its token.

/** The token of an invitation link, a group's (`/invite/<token>`) or the
 * hub's (`/signup?token=<token>`), or the token itself. */
export function invitationToken(written: string): string {
  const text = written.trim();
  const match = /\/invite\/([^/?#\s]+)/.exec(text) ?? /[?&]token=([^&#\s]+)/.exec(text);
  return match ? decodeURIComponent(match[1]) : text;
}
