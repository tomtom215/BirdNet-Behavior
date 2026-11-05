#!/usr/bin/env bash
# The members the study read from the zip archives (SNE, HAW, SSW, COR, PER, KEN), one zip_extract.py call each, by
# HTTP range request. Written from scripts/select_files.py's selection; run by
# scripts/run_all.sh, which sets MS (the working directory) and HERE (scripts/).
set -euo pipefail
: "${MS:?run this through scripts/run_all.sh}" "${HERE:?run this through scripts/run_all.sh}"
python3 -I $HERE/zip_extract.py https://zenodo.org/api/records/7525805/files/soundscape_data.zip/content $MS/data/z7525805_sne_audio $MS/meta/zip_members.tsv 'HSN_095_20150712_090105.flac' 'HSN_058_20150711_070105.flac' 'HSN_089_20150712_075805.flac' 'HSN_054_20150711_065805.flac' 'HSN_047_20150711_062105.flac' 'HSN_085_20150712_073105.flac' 'HSN_076_20150712_061805.flac' 'HSN_015_20150709_081805.flac'
python3 -I $HERE/zip_extract.py https://zenodo.org/api/records/7078499/files/soundscape_data.zip/content $MS/data/z7078499_haw_audio $MS/meta/zip_members.tsv 'UHH_352_S01_20161205_151000.flac' 'UHH_048_S01_20161124_071000.flac' 'UHH_211_S01_20161201_092000.flac' 'UHH_500_S02_20210326_182000.flac' 'UHH_483_S04_20190410_225000.flac'
python3 -I $HERE/zip_extract.py https://zenodo.org/api/records/7079380/files/soundscape_data.zip/content $MS/data/z7079380_ssw_audio $MS/meta/zip_members.tsv 'SSW_173_20170603_150016Z.flac':15000000 'SSW_212_20170617_220022Z.flac':15000000 'SSW_040_20170311_120007Z.flac':15000000
python3 -I $HERE/zip_extract.py https://zenodo.org/api/records/7525349/files/soundscape_data.zip/content $MS/data/z7525349_cor_audio $MS/meta/zip_members.tsv 'NES_029_S02_20191004_063001.flac':15000000 'NES_019_S02_20190904_053000.flac':15000000 'NES_017_S02_20190901_160000.flac':15000000
python3 -I $HERE/zip_extract.py https://zenodo.org/api/records/7079124/files/soundscape_data.zip/content $MS/data/z7079124_per_audio $MS/meta/zip_members.tsv 'PER_011_S05_20190120_100007Z.flac':15000000 'PER_013_S08_20190120_100007Z.flac':15000000 'PER_004_S05_20190116_100007Z.flac':15000000
python3 -I $HERE/zip_extract.py https://zenodo.org/api/records/10943500/files/soundscape_data.zip/content $MS/data/z10943500_ken_audio $MS/meta/zip_members.tsv 'KEN_026_20220122_065805.flac':15000000 'KEN_013_20211231_033846.flac':15000000 'KEN_015_20211231_064530.flac':15000000
