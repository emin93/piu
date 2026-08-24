#!/bin/zsh

set -euo pipefail

git_vendor_dir=${0:a:h}
git_source_dir="$git_vendor_dir/source"
git_runtime_dir="$git_vendor_dir/runtime"
git_version=2.55.0
git_archive="$git_source_dir/git-$git_version.tar.xz"
git_signature="$git_source_dir/git-$git_version.tar.sign"
git_archive_sha=457fdb04dc8728e007d4688695e6912e6f680727920f2a40bf11eacc17505357
git_signature_sha=8673501946204c38ebfed09603c1f3a041ed8d12b31f0aa06a474d41e359e254

if [[ $(uname -s) != Darwin || $(uname -m) != arm64 ]]; then
  print -u2 "The bundled Git runtime must be built on Apple Silicon macOS."
  exit 1
fi

verify_sha() {
  local expected=$1
  local artifact=$2
  local actual
  actual=$(shasum -a 256 "$artifact" | awk '{ print $1 }')
  if [[ $actual != $expected ]]; then
    print -u2 "Unexpected SHA-256 for $artifact: $actual"
    exit 1
  fi
}

verify_sha "$git_archive_sha" "$git_archive"
verify_sha "$git_signature_sha" "$git_signature"

git_build_dir=$(mktemp -d "${TMPDIR:-/tmp}/piu-git.XXXXXX")
trap 'rm -rf "$git_build_dir"' EXIT

tar -xJf "$git_archive" -C "$git_build_dir"

make -C "$git_build_dir/git-$git_version" -j"$(sysctl -n hw.logicalcpu)" \
  prefix=/ \
  MACOSX_DEPLOYMENT_TARGET=15.0 \
  APPLE_COMMON_CRYPTO=YesPlease \
  NO_OPENSSL=YesPlease \
  NO_GETTEXT=YesPlease \
  NO_TCLTK=YesPlease \
  NO_PERL=YesPlease \
  NO_PYTHON=YesPlease \
  NO_RUST=YesPlease \
  NO_BASH_COMPLETION=YesPlease \
  SKIP_DASHED_BUILT_INS=YesPlease \
  INSTALL_SYMLINKS=YesPlease

rm -rf "$git_runtime_dir"
make -C "$git_build_dir/git-$git_version" install \
  prefix=/ \
  DESTDIR="$git_runtime_dir" \
  MACOSX_DEPLOYMENT_TARGET=15.0 \
  APPLE_COMMON_CRYPTO=YesPlease \
  NO_OPENSSL=YesPlease \
  NO_GETTEXT=YesPlease \
  NO_TCLTK=YesPlease \
  NO_PERL=YesPlease \
  NO_PYTHON=YesPlease \
  NO_RUST=YesPlease \
  NO_BASH_COMPLETION=YesPlease \
  SKIP_DASHED_BUILT_INS=YesPlease \
  INSTALL_SYMLINKS=YesPlease

install -d -m 755 "$git_runtime_dir/share/licenses/git"
install -m 644 "$git_build_dir/git-$git_version/COPYING" \
  "$git_runtime_dir/share/licenses/git/COPYING"

"$git_vendor_dir/verify-macos-arm64.sh" "$git_runtime_dir"
