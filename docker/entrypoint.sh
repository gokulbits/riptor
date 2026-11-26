#!/bin/sh
# Starts riptord as PUID:PGID, or as the user the container already runs as.
set -eu

umask "${UMASK:-022}"

# Anything that isn't an option runs as given, e.g. `docker run ... riptor-cli --help`.
if [ "$#" -gt 0 ] && [ "${1#-}" = "$1" ]; then
    exec "$@"
fi

if [ "$(id -u)" != 0 ]; then
    exec riptord --data-dir /config "$@"
fi

PUID="${PUID:-1000}"
PGID="${PGID:-1000}"
for id in "$PUID" "$PGID"; do
    case "$id" in
        '' | *[!0-9]*)
            echo "riptor: PUID and PGID must be numbers" >&2
            exit 1
            ;;
    esac
done
if [ "$PUID" = 0 ]; then
    echo "riptor: running as root (PUID=0); pick another PUID if you can" >&2
fi

mkdir -p /config /downloads
# Only when the owner changed, so a big /config isn't walked every start.
if [ "$(stat -c %u:%g /config)" != "$PUID:$PGID" ]; then
    chown -R "$PUID:$PGID" /config
fi
# A fresh, empty volume; never someone's existing files.
if [ "$(stat -c %u /downloads)" = 0 ] && [ -z "$(ls -A /downloads)" ]; then
    chown "$PUID:$PGID" /downloads
fi
# Left over from a run as another user, it would be refused.
rm -rf "${RIPTOR_SOCKET_DIR:-/tmp/riptor}"

exec setpriv --reuid="$PUID" --regid="$PGID" --clear-groups --no-new-privs \
    riptord --data-dir /config "$@"
