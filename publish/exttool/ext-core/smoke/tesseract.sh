#!/bin/sh
# The language data the package guarantees (build.sh keeps exactly these): the UI languages, their vertical variants,
# the other common ones, and osd.
set -eu
langs="$(tesseract --list-langs 2>&1)"
for lang in osd eng chi_sim chi_tra spa fra deu kor jpn rus chi_sim_vert chi_tra_vert jpn_vert kor_vert \
  por ita nld pol tur ukr ara fas heb hin ben tha vie ind msa; do
  printf '%s\n' "$langs" | grep -qx "$lang" || { echo "tesseract has no $lang language data" >&2; exit 1; }
done
