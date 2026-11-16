#!/bin/sh
# Validates every XML part of the motion probe decks with xmllint: PPTX parts against the ECMA-376 5th edition
# transitional XSDs (Part 4) and the OPC XSDs (Part 2), after Markup Compatibility processing (mc:Fallback chosen);
# ODP parts against the ODF 1.3 RelaxNG schemas. Writes probes/out/motion-probe.validation.txt.
#
# Schemas: $SLIDOCK_PRIVATE/tools/specs (fetch once, see probes/motion/README.md). Needs xmllint and xsltproc.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
specs=${SLIDOCK_SPECS:-${SLIDOCK_PRIVATE:-$HOME/.slidock}/tools/specs}
out="$root/probes/out/motion-probe.validation.txt"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

for f in ecma376-t/pml.xsd ecma376-t/dml-main.xsd opc/opc-contentTypes.xsd opc/opc-relationships.xsd \
         odf13/OpenDocument-v1.3-schema.rng odf13/OpenDocument-v1.3-manifest-schema.rng; do
  [ -f "$specs/$f" ] || { echo "missing schema $specs/$f" >&2; exit 2; }
done

# Parts that break their schema on purpose (malformed timing values), from the deck manifest: they must fail.
invalid_by_design=$(sed -n 's/.*"schemaInvalidByDesign": "\(.*\)".*/\1/p' "$root/probes/out/motion-probe.pptx.manifest.json")

pass=0
fail=0
designed=0
report() { # part, schema, status, detail
  printf '%-48s %-40s %s%s\n' "$1" "$2" "$3" "${4:+  $4}" >> "$out.tmp"
  case "$3" in
    ok) pass=$((pass + 1)) ;;
    "invalid by design") designed=$((designed + 1)) ;;
    *) fail=$((fail + 1)) ;;
  esac
}
check() { # deck-relative part, file to validate, flag, schema path (relative to specs)
  by_design=no
  for p in $invalid_by_design; do [ "pptx:$p" = "$1" ] && by_design=yes; done
  if msg=$(xmllint --noout "$3" "$specs/$4" "$2" 2>&1); then
    if [ "$by_design" = yes ]; then report "$1" "$4" FAIL "validates but should not (malformed timing probe)"; else report "$1" "$4" ok; fi
  else
    detail=$(printf '%s' "$msg" | grep -v 'fails to validate' | head -3 | sed "s|$work/mce/||g" | tr '\n' ' ')
    if [ "$by_design" = yes ]; then report "$1" "$4" "invalid by design" "$detail"; else report "$1" "$4" FAIL "$detail"; fi
  fi
}

{
  echo "# xmllint $(xmllint --version 2>&1 | head -1 | sed 's/^xmllint: //')"
  echo "# ECMA-376 Part 4 (5th ed., 2016) transitional XSDs, Part 2 (5th ed., 2021) OPC XSDs, OASIS ODF 1.3 RelaxNG"
  echo "# PPTX parts are validated after mc:AlternateContent -> mc:Fallback (mce-fallback.xsl);"
  echo "# the mc:Choice branches (p14, p15, p159) are checked for well-formedness only: no public XSD covers them."
  echo "# ODP content.xml is validated without two attributes LibreOffice writes where the ODF 1.3 schema has none:"
  echo "# smil:keySplines on anim:transitionFilter (a time filter: effects.xml's flash bulb) and smil:dur on anim:command"
  echo "# (effects.xml writes 0.001 s on every command)."
  echo
} > "$out.tmp"

mkdir -p "$work/pptx" "$work/mce"
(cd "$work/pptx" && unzip -q "$root/probes/out/motion-probe.pptx")
cd "$work/pptx"
find . -type f \( -name '*.xml' -o -name '*.rels' \) | sed 's|^\./||' | LC_ALL=C sort > "$work/pptx.list"
choices=0
while read -r part; do
  case "$part" in
    "[Content_Types].xml") check "pptx:$part" "$part" --schema opc/opc-contentTypes.xsd ;;
    *.rels) check "pptx:$part" "$part" --schema opc/opc-relationships.xsd ;;
    ppt/theme/*) check "pptx:$part" "$part" --schema ecma376-t/dml-main.xsd ;;
    *)
      flat="$work/mce/$(printf '%s' "$part" | tr '/' '_')"
      xsltproc "$here/validation/mce-fallback.xsl" "$part" > "$flat"
      check "pptx:$part" "$flat" --schema ecma376-t/pml.xsd
      if grep -q 'mc:AlternateContent' "$part"; then
        if xsltproc "$here/validation/mce-choice.xsl" "$part" | xmllint --noout - 2>/dev/null; then
          choices=$((choices + 1))
        else
          report "pptx:$part (mc:Choice)" "well-formedness" FAIL
        fi
      fi
      ;;
  esac
done < "$work/pptx.list"
echo "# pptx mc:Choice branches well-formed: $choices parts" >> "$out.tmp"

mkdir -p "$work/odp"
(cd "$work/odp" && unzip -q "$root/probes/out/motion-probe.odp")
cd "$work/odp"
if [ "$(cat mimetype)" = "application/vnd.oasis.opendocument.presentation" ]; then report "odp:mimetype" "(content)" ok; else report "odp:mimetype" "(content)" FAIL; fi
check "odp:META-INF/manifest.xml" META-INF/manifest.xml --relaxng odf13/OpenDocument-v1.3-manifest-schema.rng
sed -e 's/\(<anim:transitionFilter[^>]*\) smil:keySplines="[^"]*"/\1/g' \
    -e 's/\(<anim:command[^>]*\) smil:dur="[^"]*"/\1/g' content.xml > "$work/content.xml"
check "odp:content.xml" "$work/content.xml" --relaxng odf13/OpenDocument-v1.3-schema.rng
check "odp:styles.xml" styles.xml --relaxng odf13/OpenDocument-v1.3-schema.rng

echo "# $pass ok, $designed invalid by design (malformed timing values: ST_TLTime is xsd:unsignedInt), $fail failed" >> "$out.tmp"
mv "$out.tmp" "$out"
tail -1 "$out"
[ "$fail" -eq 0 ]
