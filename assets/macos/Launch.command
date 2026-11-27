#!/bin/sh
# Terminal opens this file on Finder launch. Keep spaces/quotes in paths intact.
set -eu
resources="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
# Finder's environment may not know Homebrew; preserve existing PATH precedence.
PATH="${PATH:-/usr/bin:/bin:/usr/sbin:/sbin}:/opt/homebrew/bin:/usr/local/bin"
export PATH
# Only Finder's .command entry point resizes a window; the CLI bypasses it.
if launch_tty="$(tty 2>/dev/null)"; then
    if ! /usr/bin/osascript -l JavaScript "$resources/maximize-terminal.js" "$launch_tty" >/dev/null 2>&1; then
        printf '%s\n' 'yamusiq: не удалось развернуть окно Terminal; продолжаем в текущем размере.' >&2
    fi
fi
exec "$resources/../MacOS/yamusiq" "$@"
