#!/bin/sh
# The workshop's container runs as the machine's user, whose id the image
# does not know: name it, for the programs that ask who they run as.
if ! whoami >/dev/null 2>&1; then
    echo "stemma:x:$(id -u):$(id -g):Stemma:/home/stemma:/bin/sh" >>/etc/passwd
fi
exec "$@"
