#!/bin/sh
# Extract intent-to-command pairs from a local tealdeer/tldr page cache.
#
# Emits tab-separated "description<TAB>command" lines. Commands keep their
# {{slot}} placeholders, which already match the template form a Normalizer
# produces. Pages are CC-BY-4.0 from the tldr-pages project; anything shipped
# from this output must carry that attribution.
#
#   tools/tldr-pairs.sh > pairs.tsv
#   tools/tldr-pairs.sh ~/.cache/tealdeer/tldr-pages/pages.en common linux
set -eu

PAGES="${1:-$HOME/.cache/tealdeer/tldr-pages/pages.en}"
# Only shift when a platform argument follows; in dash a shift past the last
# positional parameter is a fatal special-builtin error.
if [ "$#" -gt 1 ]; then
    shift
    PLATFORMS="$*"
else
    PLATFORMS="common linux"
fi

if [ ! -d "$PAGES" ]; then
    echo "no page cache at $PAGES; run 'tldr --update' first" >&2
    exit 1
fi

DIRS=""
for platform in $PLATFORMS; do
    [ -d "$PAGES/$platform" ] && DIRS="$DIRS $PAGES/$platform"
done
[ -n "$DIRS" ] || { echo "none of: $PLATFORMS" >&2; exit 1; }

# shellcheck disable=SC2086
find $DIRS -name '*.md' -print0 | xargs -0 perl -ne '
    if (/^- (.+?):?\s*$/)          { $description = $1; }
    elsif (/^`(.+)`\s*$/ && defined $description) {
        print "$description\t$1\n";
        undef $description;
    }
'
