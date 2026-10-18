// Invitations, as people paste them: the link an admin sent, or its token.

/** The token of an invitation link, or the token itself. */
export function invitationToken(written: string): string {
  const text = written.trim();
  const match = /\/invite\/([^/?#\s]+)/.exec(text);
  return match ? decodeURIComponent(match[1]) : text;
}
