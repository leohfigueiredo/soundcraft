#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# pianobook.sh — work with piano libraries from pianobook.co.uk.
#
# What this can and cannot do, plainly:
#   * LISTING works without an account: the per-category catalogue is public, so this
#     prints the packs of every category, as many as asked.
#   * The site's own "Highest rated" ordering is produced in the browser from review
#     data it does not expose (its filter endpoint rejects requests without a session
#     nonce). So the list comes in the site's public order, and the ranking numbers
#     are not available to a script.
#   * DOWNLOADING needs a free account: the pack pages answer "Log in to download"
#     unless a session cookie is sent. The browser's cookie store cannot be reused
#     here (Edge/Chrome encrypt them with a key held by the GNOME keyring, and
#     `secret-tool` is not installed on this machine), so the packs have to be
#     fetched in a browser and then dropped here with `import`.
#
# Usage:
#   scripts/pianobook.sh list                 # 10 packs of every category
#   scripts/pianobook.sh list pianos 20       # one category, N packs
#   scripts/pianobook.sh import ~/Downloads   # install downloaded packs
#   scripts/pianobook.sh players              # which player to load them with
# ---------------------------------------------------------------------------
set -uo pipefail

DEST=${PIANOBOOK_DEST:-/run/media/leonardo/Elements/Pianote/plugins/pianos/pianobook}
UA="Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0 Safari/537.36"
BASE=https://www.pianobook.co.uk
CATEGORIES=(pianos pads-synths strings choirs winds guitars-bass organ drums-perc metallic found-sounds kalimba other)

fetch() { timeout 60 curl -sL -A "$UA" "$1"; }

list_category() { # category count
    local cat=$1 want=$2 html
    html=$(fetch "$BASE/instrument/$cat/") || return 1
    # Each card links to /packs/<slug>/ twice; keep the first occurrence order.
    grep -oE "href=\"$BASE/packs/[^\"]+\"" <<<"$html" |
        sed 's|.*/packs/||; s|/"$||' | awk '!seen[$0]++' | head -"$want" |
        while read -r slug; do
            # The card title sits in the link's title attribute; fall back to the slug.
            # The card's name is the link's title attribute on the same anchor.
            local title
            title=$(grep -oE "href=\"$BASE/packs/$slug/\"[^>]*title=\"[^\"]+\"" <<<"$html" | head -1 |
                sed 's/.*title="//; s/"$//') || true
            printf '  %-38s %s/packs/%s/\n' "${title:-$slug}" "$BASE" "$slug"
        done
}

list_all() { # count
    local want=$1 cat
    for cat in "${CATEGORIES[@]}"; do
        printf '\n## %s\n' "$cat"
        list_category "$cat" "$want" || echo "  (falhou ao listar)"
    done
}

players() {
    cat <<'EOF'
How to load a Pianobook pack depends on the format inside the zip:

  *.dspreset  + samples   → Decent Sampler (Linux VST3, free, needs a free account)
  *.nki                   → Kontakt (already installed here through yabridge: Kontakt 7/8)
  *.sfz                   → an SFZ player (DSK SFz Player, or sfizz)
  *.exs / EXS24           → macOS only; ignore on Linux

The packs that ship a Kontakt version are the shortest path on this machine, because
Kontakt 7/8 already load and open their editor here. Decent Sampler is the other option
and is the format most packs publish; it installs as a normal Linux VST3.

SoundCraft scans whatever is linked into ~/.vst3, so once the player is installed the
library just needs its folder kept anywhere (this script puts it under
$DEST/<pack>/).
EOF
}

import_path() { # path
    local src=${1:?usage: import <file-or-directory>}
    mkdir -p "$DEST"
    local -a zips=()
    if [[ -d $src ]]; then
        mapfile -t zips < <(find "$src" -maxdepth 2 -iname '*.zip' -newermt '-30 days' 2>/dev/null | sort)
    elif [[ -f $src ]]; then
        zips=("$src")
    fi
    if ((${#zips[@]} == 0)); then
        echo "no .zip found in $src (Pianobook downloads are zips)"
        return 1
    fi
    local z name out
    for z in "${zips[@]}"; do
        name=$(basename "$z" .zip)
        out="$DEST/$name"
        if [[ -d $out ]]; then echo "[skip] $name (already imported)"; continue; fi
        mkdir -p "$out"
        if unzip -qo "$z" -d "$out"; then
            echo "[ok  ] $name -> $out"
        else
            echo "[FAIL] $name (unzip refused it)"; rmdir "$out" 2>/dev/null
            continue
        fi
        # Some packs nest one more directory with the pack's name: flatten that level.
        if [[ $(find "$out" -maxdepth 1 -mindepth 1 -type d | wc -l) -eq 1 && $(find "$out" -maxdepth 1 -mindepth 1 -type f | wc -l) -eq 0 ]]; then
            local inner
            inner=$(find "$out" -maxdepth 1 -mindepth 1 -type d | head -1)
            if [[ -n $inner ]]; then
                shopt -s dotglob nullglob
                local -a move=("$inner"/*)
                shopt -u dotglob nullglob
                ((${#move[@]})) && mv -f "${move[@]}" "$out"/ && rmdir "$inner" 2>/dev/null
            fi
        fi
        # Report what the pack actually contains, which decides the player.
        printf '       formatos: '
        find "$out" -maxdepth 3 \( -iname '*.dspreset' -o -iname '*.nki' -o -iname '*.sfz' -o -iname '*.nkm' \) -printf '%f ' 2>/dev/null |
            sed 's/.*\.//; s/ //' | tr '\n' ' '
        find "$out" -maxdepth 3 -iname '*.dspreset' -print -quit | grep -q . && printf 'decent-sampler '
        find "$out" -maxdepth 3 -iname '*.nki' -print -quit | grep -q . && printf 'kontakt '
        find "$out" -maxdepth 3 -iname '*.sfz' -print -quit | grep -q . && printf 'sfz '
        echo
    done
    echo
    echo "Packs instalados em: $DEST"
    echo "Carregue-os no player adequado:  $0 players"
}

case "${1:---list}" in
    list)
        shift
        if [[ -n ${1:-} && ${1:-} =~ ^[0-9]+$ ]]; then list_all "$1"; else
            cat=${1:-all}; want=${2:-10}
            if [[ $cat == all ]]; then list_all "$want"; else
                echo "## $cat"; list_category "$cat" "$want"
            fi
        fi ;;
    import) shift; import_path "${1:-}" ;;
    players) players ;;
    -h | --help) sed -n '2,30p' "$0" | sed 's/^# \{0,1\}//' ;;
    *) echo "pianobook: unknown option $1 (try --help)" >&2; exit 2 ;;
esac
