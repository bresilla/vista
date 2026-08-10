#!/bin/sh
# Extract intent-to-command pairs from a local tealdeer/tldr page cache.
#
# Emits tab-separated "description<TAB>command" lines. Pages are CC-BY-4.0 from
# the tldr-pages project; anything shipped from this output must carry that
# attribution.
#
#   tools/tldr-pairs.sh > pairs.tsv
#   tools/tldr-pairs.sh --skeleton > skeletons.tsv
#   tools/tldr-pairs.sh --skeleton ~/.cache/tealdeer/tldr-pages/pages.en common
#
# Commands keep their {{slot}} placeholders, which match the template form a
# Normalizer produces. With --skeleton the placeholders are resolved instead:
# a flag choice keeps its long form and a value is dropped, leaving the literal
# words a repair can align against.
#
#   git commit {{[-m|--message]}} "{{message}}"   ->   git commit --message
set -eu

SKELETON=0
if [ "${1:-}" = "--skeleton" ]; then
    SKELETON=1
    shift
fi

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
    BEGIN { $skeleton = shift @ARGV eq "1"; }
    if (/^- (.+?):?\s*$/)          { $description = $1; }
    elsif (/^`(.+)`\s*$/ && defined $description) {
        my $command = $1;
        if ($skeleton) {
            # a flag choice keeps its long form, a value placeholder is dropped
            $command =~ s/\{\{\[([^\]]*)\]\}\}/my @o = split m!\|!, $1; $o[-1]/ge;
            $command =~ s/\{\{[^}]*\}\}//g;
            $command =~ s/["'"'"']//g;
            $command =~ s/\s+/ /g;
            $command =~ s/^\s+|\s+$//g;
            # a nested placeholder defeats the substitution above; drop it
            next if $command eq "" || $command =~ /[{}]/;
        }
        print "$description\t$command\n";
        undef $description;
    }
' "$SKELETON"
