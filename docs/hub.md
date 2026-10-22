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
provider. Git goes through the hub, under `/git`, which applies the group's
roles.

## Configuration

A TOML file (`--config`, or `STEMMA_CONFIG`), and environment variables
(`STEMMA_*`) over it: where it listens, its data directory, its public
address, and the `[forge]` section. Put it behind a proxy that serves HTTPS,
and set its public address.

On its first start the hub prints a setup link for the first account, the
hub's admin.

## Administration

Hub admins open **Hub administration** from the account menu to review group requests,
see all groups and their member counts, and grant or remove hub admin access.
The last hub admin cannot be removed. Group admin and hub admin are separate
roles: approving a request makes its requester the new group's admin.

Any signed-in person can request a group from **Your groups**. A name, short
name and purpose go to the hub's admins. Each person can have one open request
per short name; an existing group's short name cannot be requested. Decisions
remain visible in the requester's groups page. Requests never grant access
until approved, and agent and job tokens cannot use administration endpoints.
