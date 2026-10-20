#!/usr/bin/env bash
# Export one freshly pushed paios image (both arches) for release packages:
# tag it with the immutable ref, `docker save | gzip`, upload the tarball to R2
# (content-addressed by its sha256) and write the images.<name> fragment of
# bin/paios-images/paios_images.json. With PAIOS_WRITE_POINTER=1 the fragment
# also becomes R2 paios-images/main/<name>.json, which later builds inherit
# when the image does not change.
#
# Usage: paios_image_export.sh <name> <repo> <metadata.env> <amd64 digest> <aarch64 digest> <out dir>
# Env:   R2_ACCESS_KEY_ID, R2_SECRET_ACCESS_KEY (required), R2_ENDPOINT, R2_BUCKET, PAIOS_WRITE_POINTER
set -euo pipefail

name="$1" repo="$2" metadata="$3" amd64_digest="$4" aarch64_digest="$5" out="$6"
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
R2_ENDPOINT="${R2_ENDPOINT:-https://87e0bdf811b13ee87fd0bcec7a4fd1e7.r2.cloudflarestorage.com}"
R2_BUCKET="${R2_BUCKET:-buckyos-installer}"

: "${R2_ACCESS_KEY_ID:?R2_ACCESS_KEY_ID is required}"
: "${R2_SECRET_ACCESS_KEY:?R2_SECRET_ACCESS_KEY is required}"
export AWS_ACCESS_KEY_ID="$R2_ACCESS_KEY_ID" AWS_SECRET_ACCESS_KEY="$R2_SECRET_ACCESS_KEY"
export AWS_DEFAULT_REGION=auto AWS_REQUEST_CHECKSUM_CALCULATION=when_required AWS_RESPONSE_CHECKSUM_VALIDATION=when_required

r2() { aws --endpoint-url "$R2_ENDPOINT" "$@"; }

immutable_tag="$(sed -n 's/^immutable_tag=//p' "$metadata")"
[[ -n "$immutable_tag" ]] || { echo "$metadata has no immutable_tag (dirty or unknown revision)" >&2; exit 1; }
mkdir -p "$out"
docker info --format 'docker {{.ServerVersion}} storage {{.Driver}} {{json .DriverStatus}}'

platform_args=()
for arch in amd64 aarch64; do
  digest_var="${arch}_digest"
  digest="${!digest_var}"
  [[ "$digest" =~ ^sha256:[0-9a-f]{64}$ ]] || { echo "invalid $arch digest: $digest" >&2; exit 1; }
  platform=linux/amd64
  [[ "$arch" == aarch64 ]] && platform=linux/arm64
  docker pull --platform "$platform" "$repo@$digest"
  docker image inspect "$repo@$digest" > "$out/inspect-$arch.json"
  image_id="$(docker image inspect "$repo@$digest" --format '{{.Id}}')"
  ref="$repo:$immutable_tag-$arch"
  docker tag "$image_id" "$ref"
  tar="$out/$name-$arch-${image_id:7:12}.tar.gz"
  docker save "$ref" | gzip -n > "$tar"
  python3 "$script_dir/paios_images.py" platform --name "$name" --arch "$arch" --ref "$ref" \
    --inspect "$out/inspect-$arch.json" --manifest-digest "$digest" --tar "$tar" \
    --output "$out/platform-$arch.json"
  key="$(python3 -c 'import json, sys; print(json.load(open(sys.argv[1]))["tar"]["path"])' "$out/platform-$arch.json")"
  if r2 s3api head-object --bucket "$R2_BUCKET" --key "$key" >/dev/null 2>"$out/head.err"; then
    echo "R2 already has $key"
  elif grep -qE '\(404\)|Not Found|NoSuchKey' "$out/head.err"; then
    r2 s3 cp "$tar" "s3://$R2_BUCKET/$key" --content-type application/gzip --only-show-errors
    echo "uploaded $key ($(stat -c %s "$tar") bytes)"
  else
    cat "$out/head.err" >&2
    exit 1
  fi
  platform_args+=(--platform "$arch=$out/platform-$arch.json")
  rm -f "$tar"
  docker rmi "$ref" "$repo@$digest" >/dev/null 2>&1 || true
done

fragment="$out/paios-image-$name.json"
python3 "$script_dir/paios_images.py" fragment --name "$name" --repo "$repo" --metadata "$metadata" \
  "${platform_args[@]}" --output "$fragment"
cat "$fragment"

if [[ "${PAIOS_WRITE_POINTER:-0}" == 1 ]]; then
  r2 s3 cp "$fragment" "s3://$R2_BUCKET/paios-images/main/$name.json" \
    --content-type application/json --cache-control no-cache --only-show-errors
  echo "updated pointer paios-images/main/$name.json"
fi
