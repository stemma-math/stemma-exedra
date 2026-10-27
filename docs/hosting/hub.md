# The hub and its forge

One person runs a hub for everyone. It is the `stemma` binary itself,
`stemma hub`, beside a [Forgejo](https://forgejo.org) that keeps the
groups' repositories. The hub serves the web app, the API the app and
machines use, and a SQLite database with attachments in its data folder.

The hub is light. It runs no agents, builds or checkouts: people's
computers do. It only stores, coordinates, and makes HTTP calls to Forgejo
and, for the workshop's orchestrators, to a model provider.

## With Docker Compose

The repository's `docker/` folder runs both, wired together:

```sh
git clone https://github.com/stemma-math/stemma-exedra && cd stemma-exedra
export STEMMA_PUBLIC_URL=https://stemma.example.org
export FORGEJO_PUBLIC_URL=https://forge.stemma.example.org
docker/setup.sh                                    # once
docker compose -f docker/compose.yaml up -d
docker compose -f docker/compose.yaml logs hub     # the setup link
```

`setup.sh` runs once. It writes the secrets the two share to
`docker/.env`, creates Forgejo's admin, whose token the hub acts with, and
makes Forgejo sign people in through the hub.

As it is, both listen only on this machine, over HTTP: the hub on port
8080 and Forgejo on 3000. Put each behind a reverse proxy that serves
HTTPS at its public address.

On its first start, the hub prints a link to create the first account,
which is the hub's admin. Or, from the command line:

```sh
docker compose -f docker/compose.yaml exec hub stemma hub create-admin --username ada
```

## Updating

Pull the new version, then rebuild and restart:

```sh
git pull
docker compose -f docker/compose.yaml up -d --build
```

Machines must run the same version as the hub; tell your users to update
too. While Stemma is 0.x, an update may need a fresh database: the
release notes say so.

## What to back up

- The hub's data volume: the database, attachments, and `workshop.key`,
  which encrypts the groups' model keys. Without that file, the stored
  keys are lost.
- Forgejo's data volume: every repository.

## The forge

Each group is an organization in Forgejo, with:

- `library`, its Lean library;
- `recipes`, its workshop recipes;
- `paper-<slug>`, one per working paper.

People sign in to Forgejo with their Stemma account: the hub is its
OpenID Connect provider, and Forgejo's own passwords are off. Roles are
kept in step: a group's admins own its organization, members write and
guests read. Only admins, and the workshop, may push to `main`.

Git goes through the hub, at its own address, which checks roles and
tokens before passing on. The hub keeps two reserved Forgejo accounts that
nobody signs in to: **worker**, which opens the workshop's pull requests,
and **workshop**, the git orchestrator, which merges them.

## Administration

Hub admins open **Hub administration** from the account menu to:

- make invitation links, which create accounts;
- approve or decline requests for new groups;
- search groups and people, grant or remove hub admin, and delete groups
  or accounts.

A deleted account's writing stays, shown as written by a deleted account.
The last hub admin cannot be removed.

## Model keys

A group's admins may give the workshop an API key, in **Group settings ›
Workshop**. The hub stores it encrypted with `workshop.key`, and never
sends it to anyone's computer. But the hub must decrypt it to call the
provider, so **whoever runs the hub could read it**. Say so to your groups:
each should use a key made for this, with a spending limit.
