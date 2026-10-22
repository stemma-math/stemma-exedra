# The hub

`stemma hub` runs the server: the web app, the API under `/api/v1`, and a
SQLite database with the attachments in a data directory.

SIGTERM or Ctrl-C stops new connections, ends event streams and closes machine
connections normally. Requests have up to five seconds to finish before exit.

## With Docker

`docker/compose.yaml` runs the hub beside [Forgejo](https://forgejo.org):

```sh
docker/setup.sh                                  # once: secrets, Forgejo's admin, sign-in
docker compose -f docker/compose.yaml up -d
docker compose -f docker/compose.yaml logs hub   # the setup link
```

## The forge

Forgejo keeps the groups' repositories. Each group is an organization, with
its library (`library`) and a repository per paper (`paper-<slug>`). People
sign in to Forgejo with their Stemma account: the hub is its OpenID Connect
provider, and Forgejo's own password form is off (`ENABLE_INTERNAL_SIGNIN`),
as are passwords over HTTP (`ENABLE_BASIC_AUTHENTICATION`); tokens still
work. In Forgejo, a group's admins own its organization, its members write
to its repositories and its guests read them: the hub keeps those teams as
the roles are. Git goes through the hub, under `/git`, which applies the
group's roles.

Names can change in Stemma. Forgejo 16 provides `POST /orgs/{org}/rename`
and the admin's `POST /admin/users/{username}/rename`; the hub uses them,
preserving organizations, repositories and the account's OpenID Connect
subject. It also keeps an organization's displayed name in step. Deleted
accounts keep their forge history under a reserved tombstone name, freeing
the username for a different identity.

## Configuration

A TOML file (`--config`, or `STEMMA_CONFIG`), and environment variables
(`STEMMA_*`) over it: where it listens, its data directory, its public
address, and the `[forge]` section. Put it behind a proxy that serves HTTPS,
and set its public address.

On its first start the hub prints a setup link for the first account, the
hub's admin.

## Accounts and invitations

No one makes an account on their own. There are two kinds of link, both
with an expiry and a number of uses, and both revocable:

- **The hub's**, from Hub administration, makes an account on the hub and
  nothing else. Its page is the sign-up page.
- **A group's**, from the group's settings, adds an existing account to
  the group, with the role it gives. Opened signed out, it asks to log in
  first; it never makes an account.

A browser stays signed in for a day. Computers and scripts use personal
tokens, which keep their own lifetimes.

## Administration

Hub admins open **Hub administration** from the account menu. There they
review group requests, make the hub's invitation links, and search the
hub's groups and people, a page at a time. They grant or remove hub admin
access, and delete:

- **a group**, with everything in it: threads, chats, papers, files, jobs,
  links, and its organization in the forge, with the library and the
  papers' repositories. Its short name must be typed to confirm.
- **an account**: its sign-in, tokens, memberships and authorships go; what
  it wrote stays, shown as written by a deleted account. An account that is
  a group's only admin cannot be deleted until someone else is one.

The last hub admin cannot be removed or deleted. Group admin and hub admin
are separate roles: approving a request makes its requester the new
group's admin.

Any signed-in person can request a group from **Your groups**. A name, short
name and purpose go to the hub's admins. Each person can have one open request
per short name; an existing group's short name cannot be requested. Decisions
remain visible in the requester's groups page. Requests never grant access
until approved, and agent and job tokens cannot use administration endpoints.
