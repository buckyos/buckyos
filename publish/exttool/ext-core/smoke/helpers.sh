#!/bin/sh
set -eu
dir="$(mktemp -d)"
trap 'rm -rf "$dir"' EXIT
cd "$dir"
git init -q repo
git -C repo -c user.name=t -c user.email=t@t commit -q --allow-empty -m first
git -C repo log --oneline | grep -q first
curl --version | grep -q curl
printf '{"a":[1,2]}' | jq -e '.a[1] == 2' >/dev/null
mkdir -p tree/sub && : > tree/sub/x
tree tree | grep -q x
python3 -c 'import zipfile; zipfile.ZipFile("a.zip","w").writestr("inside.txt","hello")'
unzip -q a.zip -d out && grep -q hello out/inside.txt
printf hello > f && xz -k f && xz -dc f.xz | grep -q hello
uv --version >/dev/null
npm --version >/dev/null
npx --version >/dev/null
ln -s "$(command -v jq)" jq-link && ln -s jq-link jq-link2
./jq-link2 --version | grep -q jq
