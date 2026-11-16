#!/usr/bin/env bash
# Builds the font probe decks: fonts-probe.fodp (our own text) converted by LibreOffice headless (convert.py over UNO),
# in a pinned Docker image, with the OFL fonts of fonts.lock installed and EmbedFonts on, to probes/out/fonts-probe.odp
# and .pptx. Writes probes/out/fonts-probe.manifest.json (the inputs, the image, the LibreOffice version, the outputs
# and the font parts each embeds) and copies each family's licence to probes/out/licenses/.
#
# The fonts are downloaded once (pinned google/fonts commit, sha256-checked) into
# $SLIDOCK_PRIVATE/tools/probe-fonts/<commit>/ (default ~/.slidock). The container is removed when it exits.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
out="$root/probes/out"
image="slidock-probes-lo:trixie-20261030"
private="${SLIDOCK_PRIVATE:-$HOME/.slidock}"
commit="$(awk '$1 == "commit" { print $2 }' "$here/fonts.lock")"
fonts="$private/tools/probe-fonts/$commit"
work="$root/target/probe-fonts"

mkdir -p "$fonts" "$out/licenses"
while read -r sha path; do
  case "$sha" in '#'* | commit | '') continue ;; esac
  file="$fonts/$path"
  if [ ! -f "$file" ]; then
    mkdir -p "$(dirname "$file")"
    url="https://raw.githubusercontent.com/google/fonts/$commit/$(printf '%s' "$path" | sed 's/\[/%5B/g; s/\]/%5D/g')"
    curl -sSfL -o "$file" "$url"
  fi
  actual="$(shasum -a 256 "$file" | awk '{ print $1 }')"
  if [ "$actual" != "$sha" ]; then
    echo "$path: sha256 $actual, expected $sha" >&2
    exit 1
  fi
done < "$here/fonts.lock"

docker image inspect "$image" > /dev/null 2>&1 || docker build -t "$image" "$here"

# A fresh work folder under target/ (never the repository's own files).
case "$work" in "$root"/target/probe-fonts) ;; *) echo "unexpected work folder $work" >&2; exit 1 ;; esac
if [ -d "$work" ]; then rm -r -- "$work"; fi
mkdir -p "$work/fonts" "$work/out"
find "$fonts" -name '*.ttf' -exec cp {} "$work/fonts/" \;
cp "$here/fonts-probe.fodp" "$here/convert.py" "$work/"

docker run --rm --network none \
  -v "$work:/work" \
  -e HOME=/tmp/home \
  "$image" sh -c '
    mkdir -p /tmp/home/.fonts && cp /work/fonts/*.ttf /tmp/home/.fonts/ && fc-cache -f > /dev/null &&
    soffice --version > /work/out/version.txt &&
    (soffice --headless --invisible --norestore "--accept=pipe,name=slidock-probe;urp;" > /dev/null 2>&1 &) &&
    python3 /work/convert.py /work/fonts-probe.fodp /work/out'

cp "$work/out/fonts-probe.odp" "$work/out/fonts-probe.pptx" "$out/"
for family in lato barlowcondensed ptserif caveat; do
  cp "$fonts/ofl/$family/OFL.txt" "$out/licenses/$family-OFL.txt"
done

digest="$(docker image inspect --format '{{.Id}}' "$image")"
version="$(tr -d '\n' < "$work/out/version.txt")"
{
  printf '{\n  "generator": "probes/fonts/build.sh",\n  "source": "probes/fonts/fonts-probe.fodp",\n'
  printf '  "image": "%s",\n  "imageId": "%s",\n  "libreoffice": "%s",\n' "$image" "$digest" "$version"
  printf '  "fontsCommit": "%s",\n  "decks": [\n' "$commit"
  first=1
  for deck in fonts-probe.odp fonts-probe.pptx; do
    [ $first -eq 1 ] || printf ',\n'
    first=0
    sha="$(shasum -a 256 "$out/$deck" | awk '{ print $1 }')"
    parts="$( (unzip -Z1 "$out/$deck" | grep -Ei '^(Fonts/|ppt/fonts/)' || true) | sort | sed 's/.*/"&"/' | paste -sd, -)"
    printf '    { "file": "%s", "sha256": "%s", "fontParts": [%s] }' "$deck" "$sha" "$parts"
  done
  printf '\n  ]\n}\n'
} > "$out/fonts-probe.manifest.json"
cat "$out/fonts-probe.manifest.json"
