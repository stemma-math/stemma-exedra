#!/bin/sh
# Sets the hub and Forgejo of compose.yaml up, once: the secrets they share,
# in docker/.env; Forgejo's admin, whose token the hub acts with; and the
# source that signs people in to Forgejo through the hub.
#
# Set STEMMA_PUBLIC_URL and FORGEJO_PUBLIC_URL first, in the environment or
# in docker/.env, unless both are only used on this machine.
set -eu
cd "$(dirname "$0")"

compose() { docker compose -f compose.yaml "$@"; }
forgejo() { compose exec -T -u git forgejo forgejo "$@"; }
secret() { LC_ALL=C tr -dc 'A-Za-z0-9' </dev/urandom | head -c 40 || true; }

if [ -f .env ] && grep -q '^STEMMA_FORGE_TOKEN=.' .env; then
    echo "Already set up: docker/.env has the secrets."
    exit 0
fi
if [ -f .env ]; then
    . ./.env
fi
client_secret=$(secret)
umask 077
cat >.env <<EOF
STEMMA_PUBLIC_URL=${STEMMA_PUBLIC_URL:-http://localhost:8080}
FORGEJO_PUBLIC_URL=${FORGEJO_PUBLIC_URL:-http://localhost:3000}
STEMMA_FORGE_WEBHOOK_SECRET=$(secret)
STEMMA_FORGE_CLIENT_SECRET=$client_secret
EOF

compose up -d forgejo
until forgejo admin user list >/dev/null 2>&1; do sleep 2; done
forgejo admin user create --admin --username forge-admin --random-password \
    --email forge-admin@stemma.invalid --must-change-password=false
token=$(forgejo admin user generate-access-token --username forge-admin \
    --token-name stemma --scopes all --raw)
echo "STEMMA_FORGE_TOKEN=$token" >>.env

compose up -d --build hub
# Forgejo reads the hub's OpenID Connect discovery when the source is added.
until forgejo admin auth add-oauth --name stemma --provider openidConnect --key forgejo \
    --secret "$client_secret" --scopes openid --scopes profile --scopes email \
    --auto-discover-url http://hub:8080/.well-known/openid-configuration 2>/dev/null; do
    sleep 2
done
echo "Set up. The hub's setup link is in: docker compose -f docker/compose.yaml logs hub"
