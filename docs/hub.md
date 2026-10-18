# The hub

`stemma hub` runs the server: the web app, the API under `/api/v1`, and a
SQLite database with the attachments in a data directory.

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
