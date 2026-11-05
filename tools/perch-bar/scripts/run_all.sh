#!/usr/bin/env bash
# Order in which the study was produced. Each download goes to its own NEW directory under $PERCH_BAR_ROOT/data.
# (The exact command lines I ran are also recorded in meta/manifest.tsv, meta/zip_cmds.sh, meta/pow_cmd.sh.)
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
MS=${PERCH_BAR_ROOT:?set PERCH_BAR_ROOT to an empty directory for data/, work/, results/ and meta/}
: "${PERCH_BAR_MODELS:?set PERCH_BAR_MODELS to the directory holding perch-v2.onnx and the BirdNET+ V3.0 files}"
export PERCH_BAR_ROOT PERCH_BAR_MODELS MS HERE
PY=${PERCH_BAR_PYTHON:-python3}   # needs numpy and onnxruntime
MAN=$MS/meta/manifest.tsv; Z=https://zenodo.org/api/records
mkdir -p $MS/data $MS/work $MS/results $MS/meta/listings
[ -f $MAN ] || printf "utc\turl\tpath\tbytes\tsha256\tkind\n" > $MAN

# 1. metadata files (full downloads)
for spec in "7525805 sne annotations.csv species.csv description.pdf recording_location.txt" "7079124 per annotations.csv species.csv description.pdf recording_location.txt" \
            "7525349 cor annotations.csv species.csv description.pdf recording_location.txt" "7078499 hsn annotations.csv species.csv description.pdf recording_location.csv" \
            "10943500 ken annotations.csv species.csv description.pdf recording_location.txt" "7079380 ssw annotations.csv species.csv description.pdf recording_location.txt"; do
  set -- $spec; rid=$1; tag=$2; shift 2
  for f in "$@"; do python3 -I $HERE/fetch.py "$Z/$rid/files/$f/content" "$MS/data/z${rid}_${tag}_$(echo $f | tr . _)" "$f" $MAN; done
done
python3 -I $HERE/fetch.py "$Z/4656848/files/README.txt/content" $MS/data/z4656848_pow_readme README.txt $MAN
python3 -I $HERE/fetch.py "$Z/4656848/files/annotation_Files.zip/content" $MS/data/z4656848_pow_annotations_zip annotation_Files.zip $MAN
python3 -I $HERE/fetch.py "$Z/21821133/files/schema.json/content" $MS/data/z21821133_ceb_schema schema.json $MAN
python3 -I $HERE/fetch.py "$Z/21821133/files/test_soundscape.csv/content" $MS/data/z21821133_ceb_test_csv test_soundscape.csv $MAN
python3 -I $HERE/fetch.py "$Z/21821133/files/train_xenocanto.csv/content" $MS/data/z21821133_ceb_trainxc_csv train_xenocanto.csv $MAN

# 2. zip central directories -> selection -> member extraction by HTTP Range
for spec in "7525805 soundscape_data.zip sne" "7079124 soundscape_data.zip per" "7525349 soundscape_data.zip cor" "7078499 soundscape_data.zip hsn" "10943500 soundscape_data.zip ken" "7079380 soundscape_data.zip ssw" "4656848 wav_Files.zip pow"; do
  set -- $spec; python3 -I $HERE/list_zip.py "$Z/$1/files/$2/content" $MS/meta/listings/$3.tsv
done
python3 -I $HERE/select_files.py            # writes meta/selection_zip.json
# meta/zip_cmds.sh and meta/pow_cmd.sh (generated from selection_zip.json by a few lines of Python: one zip_extract.py call per set, members from selection_zip.json, ":15000000" appended for the one-hour files) call scripts/zip_extract.py
bash $HERE/../meta/zip_cmds.sh; bash $HERE/../meta/pow_cmd.sh

# 3. CEB tar.gz byte-prefixes -> complete members; ESC-50 clips
python3 -I $HERE/fetch.py "$Z/21821133/files/test_soundscape.tar.gz/content" $MS/data/z21821133_ceb_test_targz_prefix test_soundscape.tar.gz.prefix $MAN --max-bytes 300000000
python3 -I $HERE/extract_targz_prefix.py $MS/data/z21821133_ceb_test_targz_prefix/test_soundscape.tar.gz.prefix $MS/data/ceb_test_flac $MS/meta/ceb_test_members.tsv
python3 -I $HERE/fetch.py "$Z/21821133/files/train_xenocanto.tar.gz/content" $MS/data/z21821133_ceb_trainxc_targz_prefix train_xenocanto.tar.gz.prefix $MAN --max-bytes 160000000
python3 -I $HERE/extract_targz_prefix.py $MS/data/z21821133_ceb_trainxc_targz_prefix/train_xenocanto.tar.gz.prefix $MS/data/ceb_trainxc_flac $MS/meta/ceb_trainxc_members.tsv
python3 -I $HERE/fetch_esc50.py

# 4. checks, inference, analysis
$PY -I $HERE/selftest.py
$PY -I $HERE/taxon_survey.py
$PY -I $HERE/build_catalogue.py
$PY -I $HERE/run_syn.py
$PY -I $HERE/run_noise.py
$PY -I $HERE/run_real.py                     # all sets; Kenya needs the ffmpeg fallback in common.py
$PY -I $HERE/analyse_syn.py
$PY -I $HERE/analyse_real.py
$PY -I $HERE/analyse_noise.py
$PY -I $HERE/analyse_bnp.py
$PY -I $HERE/analyse_rules.py
$PY -I $HERE/analyse_sensitivity.py
$PY -I $HERE/repro_check.py
python3 -I $HERE/record_info.py
python3 -I $HERE/verify_md5.py
python3 -I $HERE/build_data_tables.py
$PY -I $HERE/make_tables.py
python3 -I $HERE/make_data_sources.py
