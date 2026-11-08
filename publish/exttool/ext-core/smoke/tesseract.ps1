$ErrorActionPreference = 'Continue'
# The language data the package guarantees (build.sh keeps exactly these).
$langs = & tesseract --list-langs 2>&1 | ForEach-Object { "$_".Trim() }
$wanted = 'osd', 'eng', 'chi_sim', 'chi_tra', 'spa', 'fra', 'deu', 'kor', 'jpn', 'rus',
  'chi_sim_vert', 'chi_tra_vert', 'jpn_vert', 'kor_vert',
  'por', 'ita', 'nld', 'pol', 'tur', 'ukr', 'ara', 'fas', 'heb', 'hin', 'ben', 'tha', 'vie', 'ind', 'msa'
foreach ($lang in $wanted) {
  if ($langs -notcontains $lang) { Write-Output "tesseract has no $lang language data"; exit 1 }
}
