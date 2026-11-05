#!/usr/bin/env bash
# The members the study read from the Powdermill archive, one zip_extract.py call each, by
# HTTP range request. Written from scripts/select_files.py's selection; run by
# scripts/run_all.sh, which sets MS (the working directory) and HERE (scripts/).
set -euo pipefail
: "${MS:?run this through scripts/run_all.sh}" "${HERE:?run this through scripts/run_all.sh}"
python3 -I $HERE/zip_extract.py https://zenodo.org/api/records/4656848/files/wav_Files.zip/content $MS/data/z4656848_pow_audio $MS/meta/zip_members.tsv 'Recording_1/Recording_1_Segment_05.WAV' 'Recording_1/Recording_1_Segment_32.WAV' 'Recording_2/Recording_2_Segment_07.wav' 'Recording_2/Recording_2_Segment_10.wav' 'Recording_4/Recording_4_Segment_11.wav' 'Recording_4/Recording_4_Segment_16.wav'
