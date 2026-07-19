#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)
output=${1:-"$repo_root/docs/current/public-api-v0.3.0.txt"}

tmp=$(mktemp)
index_tmp=$(mktemp)
module_candidates_tmp=$(mktemp)
public_modules_tmp=$(mktemp)
public_pages_tmp=$(mktemp)
metadata_tmp=$(mktemp)
filtered_metadata_tmp=$(mktemp)
compiler_artifacts_tmp=$(mktemp)
doc_target=$(mktemp -d)
cargo_workdir=$(mktemp -d)
isolated_cargo_home=
source_snapshot=$(mktemp -d)
trap 'rm -f "$tmp" "$index_tmp" "$module_candidates_tmp" "$public_modules_tmp" "$public_pages_tmp" "$metadata_tmp" "$filtered_metadata_tmp" "$compiler_artifacts_tmp"; rm -rf "$doc_target" "$cargo_workdir" "$isolated_cargo_home" "$source_snapshot"' EXIT

cd "$repo_root"
capture_script_rel=scripts/capture-public-api.sh
initial_capture_script_sha256=$(sha256sum "$capture_script_rel" | awk '{print $1}')
required_rustc_release=1.95.0
required_rustc_commit=59807616e1fa2540724bfbac14d7976d7e4a3860
required_rustdoc_release=1.95.0
required_rustdoc_commit=59807616e1fa2540724bfbac14d7976d7e4a3860
required_cargo_release=1.95.0
required_cargo_commit=f2d3ce0bd7f24a49f8f72d9000448f8838c4e850
required_host=x86_64-unknown-linux-gnu

resolve_binary() {
  local name=$1
  local candidate
  candidate=$(command -v "$name") || {
    printf 'required capture executable not found: %s\n' "$name" >&2
    exit 1
  }
  if [[ $candidate != /* ]]; then
    candidate=$(cd "$(dirname "$candidate")" && pwd -P)/$(basename "$candidate")
  fi
  if [[ ! -x $candidate ]]; then
    printf 'capture executable is not executable: %s\n' "$candidate" >&2
    exit 1
  fi
  printf '%s\n' "$candidate"
}

rustc_path=$(resolve_binary rustc)
rustdoc_path=$(resolve_binary rustdoc)
cargo_path=$(resolve_binary cargo)
git_path=$(PATH=/usr/bin:/bin resolve_binary git)
rustc_verbose=$("$rustc_path" --version --verbose)
rustdoc_verbose=$("$rustdoc_path" --version --verbose)
cargo_verbose=$("$cargo_path" --version --verbose)
rustc_release=$(sed -n 's/^release: //p' <<< "$rustc_verbose")
rustc_commit=$(sed -n 's/^commit-hash: //p' <<< "$rustc_verbose")
rustdoc_release=$(sed -n 's/^release: //p' <<< "$rustdoc_verbose")
rustdoc_commit=$(sed -n 's/^commit-hash: //p' <<< "$rustdoc_verbose")
cargo_release=$(sed -n 's/^release: //p' <<< "$cargo_verbose")
cargo_commit=$(sed -n 's/^commit-hash: //p' <<< "$cargo_verbose")
host_triple=$(sed -n 's/^host: //p' <<< "$rustc_verbose")

if [[ $rustc_release != "$required_rustc_release" ||
      $rustc_commit != "$required_rustc_commit" ||
      $rustdoc_release != "$required_rustdoc_release" ||
      $rustdoc_commit != "$required_rustdoc_commit" ||
      $cargo_release != "$required_cargo_release" ||
      $cargo_commit != "$required_cargo_commit" ||
      $host_triple != "$required_host" ]]; then
  printf 'public API capture requires the pinned Phase 0 toolchain:\n' >&2
  printf '  rustc %s (%s)\n  rustdoc %s (%s)\n  cargo %s (%s)\n  host %s\n' \
    "$required_rustc_release" "$required_rustc_commit" \
    "$required_rustdoc_release" "$required_rustdoc_commit" \
    "$required_cargo_release" "$required_cargo_commit" "$required_host" >&2
  printf 'observed:\n%s\n%s\n%s\n' "$rustc_verbose" "$rustdoc_verbose" "$cargo_verbose" >&2
  exit 1
fi
default_output="$repo_root/docs/current/public-api-v0.3.0.txt"
output_parent=$(realpath -e -- "$(dirname "$output")") || {
  printf 'capture output parent does not exist: %s\n' "$(dirname "$output")" >&2
  exit 1
}
output_basename=$(basename "$output")
if [[ $output_basename == . || $output_basename == .. ]]; then
  printf 'capture output must name a file: %s\n' "$output" >&2
  exit 1
fi
output="$output_parent/$output_basename"
if [[ (-e $output || -L $output) && $output != "$default_output" ]]; then
  printf 'refusing to overwrite existing capture output: %s\n' "$output" >&2
  exit 1
fi
run_git_discovery() {
  env -i \
    PATH=/usr/bin:/bin \
    LC_ALL=C \
    HOME="$cargo_workdir" \
    XDG_CONFIG_HOME="$cargo_workdir" \
    GIT_CONFIG_NOSYSTEM=1 \
    GIT_CONFIG_GLOBAL=/dev/null \
    GIT_CONFIG_SYSTEM=/dev/null \
    GIT_NO_REPLACE_OBJECTS=1 \
    GIT_OPTIONAL_LOCKS=0 \
    GIT_CEILING_DIRECTORIES="$(dirname "$repo_root")" \
    "$git_path" -c core.fsmonitor=false -c core.untrackedCache=false "$@"
}
git_dir=$(run_git_discovery -C "$repo_root" rev-parse --absolute-git-dir 2>/dev/null || true)
if [[ (-e $repo_root/.git || -L $repo_root/.git) && -z $git_dir ]]; then
  printf 'repository Git metadata exists but could not be resolved\n' >&2
  exit 1
fi
run_git() {
  run_git_discovery \
    --git-dir="$git_dir" \
    --work-tree="$repo_root" \
    "$@"
}

source_revision_claim=${CCWRAPPED_CAPTURE_REVISION:-}
source_revision_status=
git_head=
if [[ -n $git_dir ]]; then
  git_head=$(run_git rev-parse --verify HEAD)
fi
if [[ -n $git_head ]]; then
  source_revision_claim=${source_revision_claim:-$git_head}
  resolved_claim=$(run_git rev-parse --verify "${source_revision_claim}^{commit}" 2>/dev/null || true)
  if [[ $resolved_claim != "$source_revision_claim" ]]; then
    printf 'CCWRAPPED_CAPTURE_REVISION does not resolve to the exact commit %s\n' \
      "$source_revision_claim" >&2
    exit 1
  fi
  source_revision_status=verified-commit
elif [[ $source_revision_claim =~ ^[0-9a-f]{40}$ ]]; then
  source_revision_status=externally-asserted-gitless
elif [[ $source_revision_claim == unverified ]]; then
  source_revision_status=unverified-gitless
fi
if [[ -z $source_revision_status ]]; then
  printf 'set CCWRAPPED_CAPTURE_REVISION to the 40-hex source revision being captured, or to unverified for a Git-less current-tree capture\n' >&2
  exit 1
fi
for config in .cargo/config .cargo/config.toml; do
  if [[ -e $config ]]; then
    printf 'repository Cargo configuration is unsupported during capture: %s\n' \
      "$config" >&2
    exit 1
  fi
done

original_cargo_home=${CARGO_HOME:-}
if [[ -z $original_cargo_home ]]; then
  if [[ -z ${HOME:-} ]]; then
    printf 'capture requires CARGO_HOME or HOME to locate the offline Cargo cache\n' >&2
    exit 1
  fi
  original_cargo_home=$HOME/.cargo
fi
if [[ $original_cargo_home != "$repo_root" &&
      $original_cargo_home != "$repo_root"/* ]]; then
  isolated_cargo_home=$(mktemp -d \
    "$original_cargo_home/.ccwrapped-capture-cargo.XXXXXX" 2>/dev/null || true)
fi
isolated_cargo_home=${isolated_cargo_home:-$(mktemp -d)}

crate_index_relative_path() {
  local name=${1,,}
  case ${#name} in
    1) printf '1/%s\n' "$name" ;;
    2) printf '2/%s\n' "$name" ;;
    3) printf '3/%s/%s\n' "${name:0:1}" "$name" ;;
    *) printf '%s/%s/%s\n' "${name:0:2}" "${name:2:2}" "$name" ;;
  esac
}

# Stage only cache entries named by the lockfile. Phase 0 compatibility
# capture intentionally supports the current crates.io sparse-index graph;
# other registries and Git sources fail closed instead of cloning unrelated
# user cache state.
LOCK_PATH="$repo_root/Cargo.lock" perl -0777 -ne '
  my $count = 0;
  for my $block (split /\n\[\[package\]\]\n/) {
    my ($name) = $block =~ /^name = "([^"]+)"/m;
    my ($version) = $block =~ /^version = "([^"]+)"/m;
    my ($source) = $block =~ /^source = "([^"]+)"/m;
    my ($checksum) = $block =~ /^checksum = "([0-9a-f]{64})"/m;
    next unless defined $source;
    die "locked Git dependencies are unsupported during Phase 0 capture\n"
      if $source =~ /^git\+/;
    die "non-crates.io registries are unsupported during Phase 0 capture\n"
      unless $source eq "registry+https://github.com/rust-lang/crates.io-index" ||
             $source eq "registry+https://index.crates.io/";
    die "registry package lacks a bounded lock checksum\n"
      unless defined $name && defined $version && defined $checksum;
    die "unsupported registry package name in Cargo.lock\n"
      unless $name =~ /\A[A-Za-z0-9_-]{1,128}\z/;
    die "unsupported registry package version in Cargo.lock\n"
      unless $version =~ /\A[A-Za-z0-9.+_-]{1,128}\z/;
    die "too many locked registry packages\n" if ++$count > 10_000;
    print join("\t", $name, $version, $checksum), "\n";
  }
' "$repo_root/Cargo.lock" > "$filtered_metadata_tmp"

if [[ -s $filtered_metadata_tmp ]]; then
  registry_index_candidates=()
  for candidate in "$original_cargo_home"/registry/index/index.crates.io-*; do
    [[ -d $candidate && -f $candidate/config.json ]] || continue
    candidate_complete=true
    while IFS=$'\t' read -r locked_name _locked_version _locked_checksum; do
      index_relative=$(crate_index_relative_path "$locked_name")
      if [[ ! -f $candidate/.cache/$index_relative ]]; then
        candidate_complete=false
        break
      fi
    done < "$filtered_metadata_tmp"
    "$candidate_complete" && registry_index_candidates+=("$candidate")
  done
  if [[ ${#registry_index_candidates[@]} -ne 1 ]]; then
    printf 'expected one complete crates.io sparse-index cache, found %s\n' \
      "${#registry_index_candidates[@]}" >&2
    exit 1
  fi

  registry_index=${registry_index_candidates[0]}
  registry_namespace=$(basename "$registry_index")
  registry_cache="$original_cargo_home/registry/cache/$registry_namespace"
  isolated_index="$isolated_cargo_home/registry/index/$registry_namespace"
  isolated_cache="$isolated_cargo_home/registry/cache/$registry_namespace"
  mkdir -p "$isolated_index/.cache" "$isolated_cache"
  cp --dereference --reflink=auto -- "$registry_index/config.json" \
    "$isolated_index/config.json"

  declare -A copied_index_entries=()
  while IFS=$'\t' read -r locked_name locked_version _locked_checksum; do
    index_relative=$(crate_index_relative_path "$locked_name")
    if [[ -z ${copied_index_entries[$index_relative]+present} ]]; then
      mkdir -p "$isolated_index/.cache/$(dirname "$index_relative")"
      cp --dereference --reflink=auto -- "$registry_index/.cache/$index_relative" \
        "$isolated_index/.cache/$index_relative"
      copied_index_entries["$index_relative"]=1
    fi
    archive="$registry_cache/$locked_name-$locked_version.crate"
    if [[ -f $archive ]]; then
      cp --dereference --reflink=auto -- "$archive" "$isolated_cache/"
    fi
  done < "$filtered_metadata_tmp"
fi

config_search_dir=$cargo_workdir
while :; do
  for config in "$config_search_dir/.cargo/config" "$config_search_dir/.cargo/config.toml"; do
    if [[ -e $config ]]; then
      printf 'ambient Cargo configuration is unsupported during capture: %s\n' \
        "$config" >&2
      exit 1
    fi
  done
  [[ $config_search_dir == / ]] && break
  config_search_dir=$(dirname "$config_search_dir")
done

run_cargo() {
  (
    cd "$cargo_workdir"
    env -i \
      PATH=/usr/bin:/bin \
      LC_ALL=C \
      TMPDIR="$cargo_workdir" \
      CARGO_HOME="$isolated_cargo_home" \
      CARGO_NET_OFFLINE=true \
      CARGO_TERM_COLOR=never \
      RUSTC="$rustc_path" \
      RUSTDOC="$rustdoc_path" \
      "$cargo_path" "$@"
  )
}

wait_for_process_substitution() {
  local label=$1
  local producer_pid=$!
  local status
  if wait "$producer_pid"; then
    return 0
  else
    status=$?
    printf '%s producer failed with status %s\n' "$label" "$status" >&2
    return "$status"
  fi
}

repository_snapshot_digest() {
  local root=$1 path relative content_sha link_target
  {
    while IFS= read -r -d '' path; do
      relative=${path#"$root"/}
      if [[ -L $path ]]; then
        link_target=$(readlink -- "$path") || return 1
        printf '%s\0symlink\0%s\0' "$relative" "$link_target"
      else
        content_sha=$(sha256sum "$path" | awk '{print $1}') || return 1
        printf '%s\0file\0%s\0' "$relative" "$content_sha"
      fi
    done < <(
      find "$root" -xdev \
        -type d \( -name .git -o -name target \) -prune -o \
        \( -type f -o -type l \) -print0 | LC_ALL=C sort -z
    )
    wait_for_process_substitution 'repository inventory'
  } | sha256sum | awk '{print $1}'
}

# Cargo and rustdoc run only against this private copy. A concurrent edit to the
# checkout can neither alter the bytes being compiled nor create a mismatch
# between the artifact and its recorded source-tree digest.
live_repo_root=$repo_root
if ! live_snapshot_sha256_before=$(repository_snapshot_digest "$live_repo_root"); then
  printf 'repository changed while its private snapshot was created\n' >&2
  exit 1
fi
while IFS= read -r -d '' source_path; do
  relative_path=${source_path#"$live_repo_root"/}
  if ! mkdir -p "$source_snapshot/$(dirname "$relative_path")" ||
     ! cp --reflink=auto --no-dereference -- "$source_path" \
       "$source_snapshot/$relative_path"; then
    printf 'repository changed while its private snapshot was created\n' >&2
    exit 1
  fi
done < <(
  find "$live_repo_root" -xdev \
    -type d \( -name .git -o -name target \) -prune -o \
    \( -type f -o -type l \) -print0
)
wait_for_process_substitution 'repository snapshot inventory'
if ! private_snapshot_sha256=$(repository_snapshot_digest "$source_snapshot") ||
   ! live_snapshot_sha256_after=$(repository_snapshot_digest "$live_repo_root") ||
   [[ $private_snapshot_sha256 != "$live_snapshot_sha256_before" ]] ||
   [[ $private_snapshot_sha256 != "$live_snapshot_sha256_after" ]]; then
  printf 'repository changed while its private snapshot was created\n' >&2
  exit 1
fi
mapfile -d '' -t snapshot_symlinks < <(find "$source_snapshot" -type l -print0)
wait_for_process_substitution 'snapshot symlink inventory'
if [[ ${#snapshot_symlinks[@]} -gt 0 ]]; then
  printf 'symlinked compiler inputs are unsupported during capture:\n' >&2
  printf '  %s\n' "${snapshot_symlinks[@]#"$source_snapshot"/}" >&2
  exit 1
fi
snapshot_capture_script_sha256=$(
  sha256sum "$source_snapshot/$capture_script_rel" | awk '{print $1}'
)
if [[ $snapshot_capture_script_sha256 != "$initial_capture_script_sha256" ]]; then
  printf 'capture extractor changed while its private snapshot was created\n' >&2
  exit 1
fi
repo_root=$source_snapshot
cd "$repo_root"

run_cargo metadata --locked --manifest-path "$repo_root/Cargo.toml" \
  --filter-platform "$host_triple" --format-version 1 > "$metadata_tmp"
perl -MJSON::PP -0777 -ne '
  my $metadata = decode_json($_);
  my $resolve = $metadata->{"resolve"} // die "metadata has no resolve graph\n";
  my $root = $resolve->{"root"} // die "metadata has no root package\n";
  my %nodes = map { $_->{"id"} => $_ } @{$resolve->{"nodes"}};
  my (%active, @queue);
  push @queue, $root;
  while (@queue) {
    my $id = shift @queue;
    next if $active{$id}++;
    my $node = $nodes{$id} // die "resolve node is missing for $id\n";
    for my $dependency (@{$node->{"deps"}}) {
      my @kinds = @{$dependency->{"dep_kinds"} // []};
      next if @kinds && !grep { ($_->{"kind"} // "normal") ne "dev" } @kinds;
      push @queue, $dependency->{"pkg"};
    }
  }
  $metadata->{"packages"} = [grep { $active{$_->{"id"}} } @{$metadata->{"packages"}}];
  $resolve->{"nodes"} = [grep { $active{$_->{"id"}} } @{$resolve->{"nodes"}}];
  print JSON::PP->new->canonical->utf8->encode($metadata);
' "$metadata_tmp" > "$filtered_metadata_tmp"
mv "$filtered_metadata_tmp" "$metadata_tmp"
while IFS=$'\t' read -r registry_name registry_version registry_checksum registry_manifest; do
  [[ -z $registry_name ]] && continue
  registry_package_dir=$(realpath -e -- "$(dirname "$registry_manifest")")
  registry_src_root="$isolated_cargo_home/registry/src/"
  if [[ $registry_package_dir != "$registry_src_root"* ]]; then
    printf 'registry package escaped the isolated source cache: %s\n' \
      "$registry_package_dir" >&2
    exit 1
  fi
  registry_namespace=${registry_package_dir#"$registry_src_root"}
  registry_namespace=${registry_namespace%%/*}
  registry_archive="$isolated_cargo_home/registry/cache/$registry_namespace/$registry_name-$registry_version.crate"
  if [[ ! -f $registry_archive || -L $registry_archive ]]; then
    printf 'cached registry archive is missing or symlinked for %s %s\n' \
      "$registry_name" "$registry_version" >&2
    exit 1
  fi
  registry_archive_sha256=$(sha256sum "$registry_archive" | awk '{print $1}')
  if [[ $registry_archive_sha256 != "$registry_checksum" ]]; then
    printf 'registry archive checksum mismatch for %s %s\n' \
      "$registry_name" "$registry_version" >&2
    exit 1
  fi
done < <(LOCK_PATH="$repo_root/Cargo.lock" perl -MJSON::PP -0777 -ne '
  open my $lock_file, "<", $ENV{"LOCK_PATH"} or die "open Cargo.lock: $!\n";
  local $/;
  my $lock = <$lock_file>;
  my %checksums;
  for my $block (split /\n\[\[package\]\]\n/, $lock) {
    my ($name) = $block =~ /^name = "([^"]+)"/m;
    my ($version) = $block =~ /^version = "([^"]+)"/m;
    my ($source) = $block =~ /^source = "([^"]+)"/m;
    my ($checksum) = $block =~ /^checksum = "([0-9a-f]{64})"/m;
    next unless defined $source && $source =~ /^registry\+/;
    die "registry package lacks a bounded lock checksum\n"
      unless defined $name && defined $version && defined $checksum;
    $checksums{join("\0", $name, $version, $source)} = $checksum;
  }
  my $metadata = decode_json($_);
  for my $package (@{$metadata->{"packages"}}) {
    my $source = $package->{"source"} // next;
    next unless $source =~ /^registry\+/;
    my $key = join("\0", $package->{"name"}, $package->{"version"}, $source);
    die "host dependency lacks a matching Cargo.lock checksum\n"
      unless exists $checksums{$key};
    print join("\t", $package->{"name"}, $package->{"version"},
      $checksums{$key}, $package->{"manifest_path"}), "\n";
  }
' "$metadata_tmp")
wait_for_process_substitution 'registry checksum inventory'
package_version=$(ROOT_MANIFEST="$repo_root/Cargo.toml" perl -MJSON::PP -0777 -ne '
  my $metadata = decode_json($_);
  my @matches = grep { $_->{"manifest_path"} eq $ENV{"ROOT_MANIFEST"} } @{$metadata->{"packages"}};
  die "expected exactly one root package\n" unless @matches == 1;
  print $matches[0]->{"version"};
' "$metadata_tmp")
package_features=$(ROOT_MANIFEST="$repo_root/Cargo.toml" perl -MJSON::PP -0777 -ne '
  my $metadata = decode_json($_);
  my @matches = grep { $_->{"manifest_path"} eq $ENV{"ROOT_MANIFEST"} } @{$metadata->{"packages"}};
  die "expected exactly one root package\n" unless @matches == 1;
  print join("\n", sort keys %{$matches[0]->{"features"} // {}});
' "$metadata_tmp")
if [[ -n $package_features ]]; then
  printf 'package features require an explicit capture matrix:\n%s\n' \
    "$package_features" >&2
  exit 1
fi

mapfile -d '' -t local_manifests < <(perl -MJSON::PP -0777 -ne '
  my $metadata = decode_json($_);
  for my $package (@{$metadata->{"packages"}}) {
    print $package->{"manifest_path"}, "\0" unless defined $package->{"source"};
  }
' "$metadata_tmp")
wait_for_process_substitution 'local package inventory'
local_build_scripts=$(perl -MJSON::PP -0777 -ne '
  my $metadata = decode_json($_);
  for my $package (@{$metadata->{"packages"}}) {
    next if defined $package->{"source"};
    for my $target (@{$package->{"targets"}}) {
      print $target->{"src_path"}, "\n" if grep { $_ eq "custom-build" } @{$target->{"kind"}};
    }
  }
' "$metadata_tmp")
if [[ -n $local_build_scripts ]]; then
  printf 'local build scripts are unsupported during capture:\n%s\n' \
    "$local_build_scripts" >&2
  exit 1
fi
dependency_build_script_entries=()
while IFS=$'\t' read -r dependency_name dependency_version dependency_source build_script; do
  [[ -z $dependency_name ]] && continue
  build_script_sha256=$(sha256sum "$build_script" | awk '{print $1}')
  entry="$dependency_name $dependency_version $dependency_source $build_script_sha256"
  case "$entry" in
    'ahash 0.8.12 registry+https://github.com/rust-lang/crates.io-index d7dd5428c78b80bb3c99068561641ec661f0f94defbda17f85b443e358ab6396' | \
    'blake3 1.8.5 registry+https://github.com/rust-lang/crates.io-index 680019541096959dba4e1b6795ecf1b17233f18d7d2450fe1c5d446d71c9cca5' | \
    'chrono-tz 0.10.4 registry+https://github.com/rust-lang/crates.io-index a95f00ef475e4250661ae26ea1bc1a2562695c44aee8b9bd4a7e175df27c21e7' | \
    'getrandom 0.3.4 registry+https://github.com/rust-lang/crates.io-index d542c9c3bbc2f64a26c758eac8af16178aab52464bdd8bf0b9a767ba87665021' | \
    'libc 0.2.186 registry+https://github.com/rust-lang/crates.io-index 4b348c53d0a0cd0067ef9887b50f60a1fffdc5d00dda5c0e27fae6aa0ce3dee8' | \
    'libsqlite3-sys 0.30.1 registry+https://github.com/rust-lang/crates.io-index 11bebc3787657749279375ab2e875e52259f995dce8916b4907103975e2a76ee' | \
    'num-traits 0.2.19 registry+https://github.com/rust-lang/crates.io-index d3969209fc1c9d201c66ed11820d0b328600d75b3971f8ceebeab04900bc0587' | \
    'proc-macro2 1.0.106 registry+https://github.com/rust-lang/crates.io-index baeb20b52f6b536be8657a566591a507bb2e34a45cf8baa42b135510a0c3c729' | \
    'quote 1.0.45 registry+https://github.com/rust-lang/crates.io-index cd6808c02e476b09a520105e2c6f6d325cccb1ecd542cbbcc836a0ae6f6fb0f1' | \
    'serde 1.0.228 registry+https://github.com/rust-lang/crates.io-index c99c25e1a11f3e51b61dd8c25bdbfbd090e43e5f1f2014f41f28e214dd8310d5' | \
    'serde_core 1.0.228 registry+https://github.com/rust-lang/crates.io-index a5fdacb9913eeede5fc08f39acbd31b2508483fdc20c79d6bc274d74407a1816' | \
    'serde_json 1.0.149 registry+https://github.com/rust-lang/crates.io-index a681e754be844c7dbef957f5d2d00b01f37c5dca160ef1055a8d8f975697a881' | \
    'zerocopy 0.8.50 registry+https://github.com/rust-lang/crates.io-index ffe22497073ff8b34ab9bf631e7d64360db34d58212a8109b3bc2f5e9dcc0490' | \
    'zstd-safe 7.2.4 registry+https://github.com/rust-lang/crates.io-index 2342e59833e2ebca2980884d4f242a6bf1b0143037c212e05514626ad5213505' | \
    'zstd-sys 2.0.16+zstd.1.5.7 registry+https://github.com/rust-lang/crates.io-index d92ade96b5f7c04496b1e928c564fcf52bb7d59439f56262407da77c5628458d' | \
    'zmij 1.0.21 registry+https://github.com/rust-lang/crates.io-index 13201b550236a9ff2186b5303c77341d3b0874989fdaf39613a7452e4ce57817') ;;
    *)
      printf 'unsupported dependency build script: %s\n' "$entry" >&2
      exit 1
      ;;
  esac
  dependency_build_script_entries+=("$entry")
done < <(perl -MJSON::PP -0777 -ne '
  my $metadata = decode_json($_);
  for my $package (@{$metadata->{"packages"}}) {
    next unless defined $package->{"source"};
    for my $target (@{$package->{"targets"}}) {
      if (grep { $_ eq "custom-build" } @{$target->{"kind"}}) {
        print join("\t", $package->{"name"}, $package->{"version"},
          $package->{"source"}, $target->{"src_path"}), "\n";
      }
    }
  }
' "$metadata_tmp")
wait_for_process_substitution 'dependency build-script inventory'
if [[ ${#dependency_build_script_entries[@]} -gt 0 ]]; then
  mapfile -t dependency_build_script_entries < <(
    printf '%s\n' "${dependency_build_script_entries[@]}" | sort -u
  )
  wait_for_process_substitution 'dependency build-script ordering'
fi
dependency_build_scripts_sha256=$({
  for entry in "${dependency_build_script_entries[@]}"; do
    printf '%s\0' "$entry"
  done
} | sha256sum | awk '{print $1}')
dependency_source_entries=()
mapfile -t dependency_packages < <(perl -MJSON::PP -0777 -ne '
  my $metadata = decode_json($_);
  for my $package (@{$metadata->{"packages"}}) {
    next unless defined $package->{"source"};
    print join("\t", $package->{"name"}, $package->{"version"},
      $package->{"source"}, $package->{"manifest_path"}), "\n";
  }
' "$metadata_tmp")
wait_for_process_substitution 'dependency package inventory'
for dependency_package in "${dependency_packages[@]}"; do
  IFS=$'\t' read -r dependency_name dependency_version dependency_source dependency_manifest \
    <<< "$dependency_package"
  [[ -z $dependency_name ]] && continue
  dependency_dir=$(realpath -e -- "$(dirname "$dependency_manifest")")
  if [[ $dependency_dir != "$isolated_cargo_home"/* ]]; then
    printf 'dependency source escaped the isolated Cargo home: %s\n' \
      "$dependency_dir" >&2
    exit 1
  fi
  mapfile -d '' -t dependency_symlinks < <(find "$dependency_dir" -type l -print0)
  wait_for_process_substitution 'dependency symlink inventory'
  if [[ ${#dependency_symlinks[@]} -gt 0 ]]; then
    printf 'symlinked dependency sources are unsupported during capture: %s\n' \
      "$dependency_name $dependency_version" >&2
    exit 1
  fi
  while IFS= read -r -d '' dependency_file; do
    dependency_relative=${dependency_file#"$dependency_dir"/}
    dependency_file_sha256=$(sha256sum "$dependency_file" | awk '{print $1}')
    dependency_source_entries+=(
      "$dependency_name $dependency_version $dependency_source"$'\t'"$dependency_relative"$'\t'"$dependency_file_sha256"
    )
  done < <(
    find "$dependency_dir" -type f \
      ! -path "$dependency_dir/.git/*" \
      ! -path "$dependency_dir/target/*" -print0
  )
  wait_for_process_substitution 'dependency file inventory'
done
if [[ ${#dependency_source_entries[@]} -gt 0 ]]; then
  mapfile -d '' -t dependency_source_entries < <(
    printf '%s\0' "${dependency_source_entries[@]}" | LC_ALL=C sort -zu
  )
  wait_for_process_substitution 'dependency source ordering'
fi
dependency_sources_sha256=$({
  for entry in "${dependency_source_entries[@]}"; do
    printf '%s\0' "$entry"
  done
} | sha256sum | awk '{print $1}')
local_dependency_dirs=()
for manifest in "${local_manifests[@]}"; do
  package_dir=$(realpath -e -- "$(dirname "$manifest")")
  [[ $package_dir == "$repo_root" ]] && continue
  if [[ $package_dir != "$repo_root"/* ]]; then
    printf 'local Cargo package is outside the captured repository: %s\n' \
      "$package_dir" >&2
    exit 1
  fi
  local_dependency_dirs+=("$package_dir")
done
if [[ ${#local_dependency_dirs[@]} -gt 0 ]]; then
  mapfile -d '' -t local_dependency_dirs < <(
    printf '%s\0' "${local_dependency_dirs[@]}" | sort -zu
  )
  wait_for_process_substitution 'local package ordering'
fi

product_pathspecs=(Cargo.toml Cargo.lock build.rs src)
for package_dir in "${local_dependency_dirs[@]}"; do
  product_pathspecs+=("${package_dir#"$repo_root"/}")
done
if [[ -n $git_head ]]; then
  untracked_product_inputs=$(run_git ls-files --others -- "${product_pathspecs[@]}")
  if ! run_git diff --no-ext-diff --no-textconv --quiet \
       "$source_revision_claim" -- "${product_pathspecs[@]}" ||
     [[ -n $untracked_product_inputs ]]; then
    printf 'captured product inputs differ from source revision %s\n' \
      "$source_revision_claim" >&2
    exit 1
  fi
fi

symlink_inputs=()
for path in Cargo.toml Cargo.lock build.rs; do
  [[ -L $path ]] && symlink_inputs+=("$path")
done
while IFS= read -r -d '' path; do
  symlink_inputs+=("$path")
done < <(find src -type l -print0)
wait_for_process_substitution 'root source symlink inventory'
for package_dir in "${local_dependency_dirs[@]}"; do
  while IFS= read -r -d '' path; do
    symlink_inputs+=("${path#"$repo_root"/}")
  done < <(find "$package_dir" -type l \
    ! -path "$package_dir/.git/*" \
    ! -path "$package_dir/target/*" -print0)
  wait_for_process_substitution 'local package symlink inventory'
done
if [[ ${#symlink_inputs[@]} -gt 0 ]]; then
  printf 'symlinked compiler inputs are unsupported during capture:\n' >&2
  printf '  %s\n' "${symlink_inputs[@]}" >&2
  exit 1
fi

hash_tree_paths() {
  local path
  for path in "$@"; do
    printf '%s\0' "$path"
    sha256sum "$path" | awk '{printf "%s%c", $1, 0}'
  done | sha256sum | awk '{print $1}'
}

mapfile -d '' -t product_inputs < <(
  {
    printf '%s\0' Cargo.toml Cargo.lock
    [[ -f build.rs ]] && printf '%s\0' build.rs
    find src -type f -print0
    for package_dir in "${local_dependency_dirs[@]}"; do
      while IFS= read -r -d '' file; do
        printf '%s\0' "${file#"$repo_root"/}"
      done < <(find "$package_dir" -type f \
        ! -path "$package_dir/.git/*" \
        ! -path "$package_dir/target/*" -print0)
      wait_for_process_substitution 'local package file inventory'
    done
  } | sort -zu
)
wait_for_process_substitution 'product input inventory'
source_tree_sha256=$(hash_tree_paths "${product_inputs[@]}")
extractor_tree_sha256=$(hash_tree_paths scripts/capture-public-api.sh)

validate_compiler_inputs() {
  local target_dir=$1
  local compiler_artifacts=$2
  shift 2
  local path resolved dep_file package_dir dependency candidate
  declare -A allowed_inputs=()
  for path in "${product_inputs[@]}" "$@"; do
    resolved=$(realpath -e -- "$path") || {
      printf 'compiler input disappeared before validation: %s\n' "$path" >&2
      exit 1
    }
    allowed_inputs["$resolved"]=1
  done

  declare -A local_dep_file_dirs=()
  while IFS= read -r -d '' dep_file && IFS= read -r -d '' package_dir; do
    dep_file=$(realpath -e -- "$dep_file") || {
      printf 'compiler artifact references missing dep-info: %s\n' "$dep_file" >&2
      exit 1
    }
    package_dir=$(realpath -e -- "$package_dir")
    if [[ -n ${local_dep_file_dirs[$dep_file]+present} &&
          ${local_dep_file_dirs[$dep_file]} != "$package_dir" ]]; then
      printf 'ambiguous local Cargo dep-info ownership: %s\n' "$dep_file" >&2
      exit 1
    fi
    local_dep_file_dirs["$dep_file"]=$package_dir
  done < <(perl -MJSON::PP -ne '
    use File::Basename qw(basename dirname);
    my $message = eval { decode_json($_) };
    die "invalid Cargo JSON message: $@" if $@;
    next unless ($message->{"reason"} // "") eq "compiler-artifact";
    next unless ($message->{"package_id"} // "") =~ /^path\+file:/;
    my $package_dir = dirname($message->{"manifest_path"});
    for my $filename (@{$message->{"filenames"} // []}) {
      my $stem = basename($filename);
      next unless $stem =~ s/\.(?:rmeta|rlib|so)$//;
      $stem =~ s/^lib//;
      my $dep_file = dirname($filename) . "/$stem.d";
      print $dep_file, "\0", $package_dir, "\0" if -f $dep_file;
    }
  ' "$compiler_artifacts")
  wait_for_process_substitution 'local compiler artifact inventory'

  mapfile -d '' -t dep_files < <(find "$target_dir" -type f -name '*.d' -print0)
  wait_for_process_substitution 'compiler dep-info inventory'
  for dep_file in "${dep_files[@]}"; do
    dep_file=$(realpath -e -- "$dep_file")
    [[ -n ${local_dep_file_dirs[$dep_file]+present} ]] || continue
    package_dir=${local_dep_file_dirs[$dep_file]}
    while IFS= read -r -d '' dependency; do
      if [[ $dependency == /* ]]; then
        candidate=$dependency
      elif [[ -e $package_dir/$dependency || -L $package_dir/$dependency ]]; then
        candidate=$package_dir/$dependency
      else
        candidate=$repo_root/$dependency
      fi
      resolved=$(realpath -e -- "$candidate") || {
        printf 'unresolvable compiler dep-info input: %s\n' "$dependency" >&2
        exit 1
      }
      if [[ -z ${allowed_inputs[$resolved]+present} ]]; then
        printf 'compiler dep-info contains input outside the captured closure: %s\n' \
          "$resolved" >&2
        exit 1
      fi
    done < <(perl -0777 -ne '
      s/\\\r?\n//g;
      for my $line (split /\n/) {
        next if $line =~ /^\s*#/;
        next unless $line =~ /^[^:]*:\s*(.*)$/;
        my $dependencies = $1;
        while ($dependencies =~ /((?:\\.|[^\s])+)/g) {
          my $path = $1;
          $path =~ s/\\(.)/$1/g;
          print $path, "\0";
        }
      }
    ' "$dep_file")
    wait_for_process_substitution 'compiler dep-info parser'
  done
}

: > "$compiler_artifacts_tmp"
run_cargo check --locked --manifest-path "$repo_root/Cargo.toml" --lib \
  --target "$host_triple" --target-dir "$doc_target" \
  --message-format=json-render-diagnostics > "$compiler_artifacts_tmp"
validate_compiler_inputs "$doc_target" "$compiler_artifacts_tmp"
# Stable rustdoc does not emit dep-info for an HTML build. A second pinned
# rustc pass with rustdoc's built-in `cfg(doc)` enumerates doc-only modules and
# include macros before the real documentation build consumes them.
run_cargo check --locked --manifest-path "$repo_root/Cargo.toml" --lib \
  --target "$host_triple" --target-dir "$doc_target" \
  --config 'build.rustflags=["--cfg","doc"]' \
  --message-format=json-render-diagnostics >> "$compiler_artifacts_tmp"
validate_compiler_inputs "$doc_target" "$compiler_artifacts_tmp"
run_cargo doc --locked --manifest-path "$repo_root/Cargo.toml" --lib \
  --target "$host_triple" --target-dir "$doc_target" \
  --message-format=json-render-diagnostics >> "$compiler_artifacts_tmp"
validate_compiler_inputs "$doc_target" "$compiler_artifacts_tmp"
mapfile -t doc_roots < <(find "$doc_target" -type d -path '*/doc/ccwrapped' -print | sort)
wait_for_process_substitution 'rustdoc root inventory'
if [[ ${#doc_roots[@]} -ne 1 ]]; then
  printf 'expected one fresh rustdoc crate root, found %s under %s\n' \
    "${#doc_roots[@]}" "$doc_target" >&2
  exit 1
fi
doc_root=${doc_roots[0]}
doc_tree=$(dirname "$doc_root")

extract_decl() {
  perl -0777 -ne '
    sub canonical_link {
      my ($href, $display) = @_;
      if ($href =~ m{^(?:\.\./)+(.+)/(?:struct|enum|trait|type|union|primitive|constant|static|macro)\.([^/]+)\.html$}) {
        my ($module, $name) = ($1, $2);
        $module =~ s{/}{::}g;
        return "$module\::$name";
      }
      return $display;
    }
    if (m{<pre class="rust item-decl"><code>(.*?)</code></pre>}s) {
      $value = $1;
      $value =~ s{<button\b[^>]*>.*?</button>}{}sg;
      $value =~ s{<summary\b[^>]*>.*?</summary>}{}sg;
      $value =~ s{<a\b[^>]*title="(?:[^"]* )?([A-Za-z_][A-Za-z0-9_:]*)"[^>]*>.*?</a>}{$1}sg;
      $value =~ s{<a\b[^>]*href="([^"]+)"[^>]*>(.*?)</a>}{canonical_link($1, $2)}gse;
      $value =~ s/<[^>]+>//g;
      $value =~ s/&lt;/</g;
      $value =~ s/&gt;/>/g;
      $value =~ s/&amp;/&/g;
      $value =~ s/&quot;/"/g;
      $value =~ s/&#39;/'"'"'/g;
      $value =~ s/\s+/ /g;
      $value =~ s/^\s+|\s+$//g;
      print $value;
    }
  '
}

extract_type_api() {
  local type_name=$1
  TYPE_NAME=$type_name perl -0777 -ne '
    sub canonical_link {
      my ($href, $display) = @_;
      if ($href =~ m{^(?:\.\./)+(.+)/(?:struct|enum|trait|type|union|primitive|constant|static|macro)\.([^/]+)\.html$}) {
        my ($module, $name) = ($1, $2);
        $module =~ s{/}{::}g;
        return "$module\::$name";
      }
      return $display;
    }
    sub normalize {
      my ($value) = @_;
      $value =~ s{<a\b[^>]*title="(?:[^"]* )?([A-Za-z_][A-Za-z0-9_:]*)"[^>]*>.*?</a>}{$1}sg;
      $value =~ s{<a\b[^>]*href="([^"]+)"[^>]*>(.*?)</a>}{canonical_link($1, $2)}gse;
      $value =~ s/<[^>]+>//g;
      $value =~ s/&lt;/</g;
      $value =~ s/&gt;/>/g;
      $value =~ s/&amp;/&/g;
      $value =~ s/&quot;/"/g;
      $value =~ s/&#39;/'"'"'/g;
      $value =~ s/\s+/ /g;
      $value =~ s/^\s+|\s+$//g;
      return $value;
    }

    my $name = $ENV{"TYPE_NAME"};
    my @items;
    while (m{<section id="method\.[^"]+" class="method">.*?<h4 class="code-header">(.*?)</h4>}sg) {
      push @items, "method " . normalize($1);
    }
    while (m{<section id="associatedconstant\.[^"]+" class="associatedconstant">.*?<h4 class="code-header">(.*?)</h4>}sg) {
      push @items, "associated-constant " . normalize($1);
    }
    while (m{<h3 class="code-header">(.*?)</h3>}sg) {
      my $value = normalize($1);
      next if $value =~ /^impl core::marker::(?:Freeze|UnsafeUnpin) for /;
      if ($value =~ /^impl(?:<.*?>)?\s+.+\s+for\s+(.+)$/) {
        my $self_type = $1;
        if ($self_type =~ /(?:^|[^A-Za-z0-9_])(?:[A-Za-z_][A-Za-z0-9_]*::)*\Q$name\E(?:$|[^A-Za-z0-9_])/) {
          push @items, "trait " . $value;
        }
      }
    }
    print join("\n", sort @items);
    print "\n" if @items;
  '
}

extract_index_api() {
  local module_path=$1
  MODULE_PATH=$module_path perl -0777 -ne '
    sub canonical_link {
      my ($href, $display) = @_;
      if ($href =~ m{^(?:\.\./)+(.+)/(?:struct|enum|trait|type|union|primitive|constant|static|macro)\.([^/]+)\.html$}) {
        my ($module, $name) = ($1, $2);
        $module =~ s{/}{::}g;
        return "$module\::$name";
      }
      return $display;
    }
    sub normalize {
      my ($value) = @_;
      $value =~ s{<a\b[^>]*title="(?:[^"]* )?([A-Za-z_][A-Za-z0-9_:]*)"[^>]*>.*?</a>}{$1}sg;
      $value =~ s{<a\b[^>]*href="([^"]+)"[^>]*>(.*?)</a>}{canonical_link($1, $2)}gse;
      $value =~ s/<[^>]+>//g;
      $value =~ s/&lt;/</g;
      $value =~ s/&gt;/>/g;
      $value =~ s/&amp;/&/g;
      $value =~ s/&quot;/"/g;
      $value =~ s/&#39;/'"'"'/g;
      $value =~ s/\s+/ /g;
      $value =~ s/^\s+|\s+$//g;
      return $value;
    }

    my $module = $ENV{"MODULE_PATH"};
    my %items = ("module $module" => 1);
    if (m{<dl class="item-table reexports">(.*?)</dl>}s) {
      my $reexports = $1;
      while ($reexports =~ m{<dt>(.*?)</dt>}sg) {
        my $entry = $1;
        next if $entry =~ /title="Restricted Visibility"/;
        if ($entry =~ m{<code>(.*?)</code>}s) {
          $items{"reexport $module :: " . normalize($1)} = 1;
        }
      }
    }
    print "$_\n" for sort keys %items;
  '
}

extract_public_modules() {
  perl -0777 -ne '
    while (m{<dt>(.*?)</dt>}sg) {
      my $entry = $1;
      next if $entry =~ /title="Restricted Visibility"/;
      if ($entry =~ m{<a class="mod"[^>]*title="mod ([^"]+)"}s) {
        print "$1\n";
      }
    }
  '
}

extract_public_item_refs() {
  perl -0777 -ne '
    while (m{<dt>(.*?)</dt>}sg) {
      my $entry = $1;
      next if $entry =~ /title="Restricted Visibility"/;
      while ($entry =~ m{(<a\b[^>]*href="[^"]+"[^>]*>.*?</a>)}sg) {
        my $anchor = $1;
        next unless $anchor =~ /href="([^"]+)"/;
        my $href = $1;
        next unless $href =~ m{(?:^|/)[a-z][a-z0-9_-]*\.([^/\"]+)\.html$};
        my $storage_name = $1;
        $anchor =~ s/^.*?>//s;
        $anchor =~ s{</a>.*$}{}s;
        $anchor =~ s/<[^>]+>//g;
        $anchor =~ s/&amp;/&/g;
        $anchor =~ s/!$//;
        $anchor =~ s/^\s+|\s+$//g;
        my $public_name = length($anchor) ? $anchor : $storage_name;
        print "$href\t$public_name\n";
        last;
      }
    }
  '
}

printf '%s\n' ccwrapped > "$module_candidates_tmp"
while IFS= read -r index; do
  extract_public_modules < "$index" >> "$module_candidates_tmp"
done < <(find "$doc_root" -type f -name index.html | sort)
wait_for_process_substitution 'rustdoc module inventory'
sort -u -o "$module_candidates_tmp" "$module_candidates_tmp"

printf '%s\n' ccwrapped > "$public_modules_tmp"
while :; do
  before_count=$(wc -l < "$public_modules_tmp")
  while IFS= read -r module_path; do
    [[ $module_path == ccwrapped ]] && continue
    parent=${module_path%::*}
    if grep -Fqx -- "$parent" "$public_modules_tmp"; then
      printf '%s\n' "$module_path" >> "$public_modules_tmp"
    fi
  done < "$module_candidates_tmp"
  sort -u -o "$public_modules_tmp" "$public_modules_tmp"
  after_count=$(wc -l < "$public_modules_tmp")
  [[ $after_count -eq $before_count ]] && break
done

while IFS= read -r module_path; do
  if [[ $module_path == ccwrapped ]]; then
    index="$doc_root/index.html"
  else
    module_directory=${module_path#ccwrapped::}
    module_directory=${module_directory//:://}
    index="$doc_root/$module_directory/index.html"
  fi
  if [[ ! -f $index ]]; then
    printf 'public module %s has no rustdoc index at %s\n' "$module_path" "$index" >&2
    exit 1
  fi

  extract_index_api "$module_path" < "$index" >> "$index_tmp"
  index_directory=$(dirname "$index")
  while IFS=$'\t' read -r href public_name; do
    page=$(realpath -m -- "$index_directory/$href")
    if [[ $page == "$doc_tree"/* && -f $page ]]; then
      printf '%s\t%s::%s\n' "$page" "$module_path" "$public_name" >> "$public_pages_tmp"
    fi
  done < <(extract_public_item_refs < "$index")
  wait_for_process_substitution 'rustdoc public item inventory'
done < "$public_modules_tmp"
sort -u -o "$index_tmp" "$index_tmp"
sort -u -o "$public_pages_tmp" "$public_pages_tmp"

{
  printf '%s\n' \
    'artifact-format public-api-signatures/v7' \
    'extractor-version rustdoc-html/v11' \
    "rustc-release $rustc_release" \
    "rustc-commit $rustc_commit" \
    "rustdoc-release $rustdoc_release" \
    "rustdoc-commit $rustdoc_commit" \
    "cargo-release $cargo_release" \
    "cargo-commit $cargo_commit" \
    "target $host_triple" \
    'feature-surface default-no-package-features' \
    'crate ccwrapped' \
    "crate-version $package_version" \
    "source-revision-claim $source_revision_claim" \
    "source-revision-status $source_revision_status" \
    "source-tree-sha256 $source_tree_sha256" \
    "extractor-tree-sha256 $extractor_tree_sha256"
  printf 'dependency-build-scripts-sha256 %s\n' "$dependency_build_scripts_sha256"
  printf 'dependency-sources-sha256 %s\n' "$dependency_sources_sha256"

  cat "$index_tmp"

  while IFS=$'\t' read -r file fq_path; do
    relative=${file#"$doc_root"/}
    basename=$(basename "$relative")
    if [[ ! $basename =~ ^([a-z][a-z0-9_-]*)\.(.+)\.html$ ]]; then
      continue
    fi
    item_kind=${BASH_REMATCH[1]}
    item_name=${BASH_REMATCH[2]}
    [[ $item_name == *'!' ]] && continue
    declaration=$(extract_decl < "$file")
    if [[ $item_kind != macro && ! $declaration =~ (^|\])pub[[:space:]] ]]; then
      continue
    fi
    printf 'item %s %s :: %s\n' "$item_kind" "$fq_path" "${declaration:-declaration-unavailable}"

    if [[ $item_kind == struct || $item_kind == enum || $item_kind == union ]]; then
      extract_type_api "$item_name" < "$file" | while IFS= read -r detail; do
        [[ -n $detail ]] && printf '%s %s :: %s\n' "${detail%% *}" "$fq_path" "${detail#* }"
      done
    fi
  done < "$public_pages_tmp"
} > "$tmp"

if grep -Eq 'Show [0-9]+ (fields|variants)|Copy item path' "$tmp"; then
  printf 'rustdoc presentation text leaked into the API artifact\n' >&2
  exit 1
fi

final_capture_script_sha256=$(
  sha256sum "$live_repo_root/$capture_script_rel" | awk '{print $1}'
)
if [[ $final_capture_script_sha256 != "$initial_capture_script_sha256" ]]; then
  printf 'capture extractor changed while capture was running\n' >&2
  exit 1
fi

if [[ $output == "$default_output" ]]; then
  mv "$tmp" "$output"
else
  mv -n "$tmp" "$output"
  if [[ -e $tmp ]]; then
    printf 'capture output appeared before publication; refusing to overwrite: %s\n' \
      "$output" >&2
    exit 1
  fi
fi
rm -f "$index_tmp"
rm -f "$module_candidates_tmp" "$public_modules_tmp" "$public_pages_tmp" "$metadata_tmp" "$filtered_metadata_tmp" "$compiler_artifacts_tmp"
rm -rf "$doc_target" "$cargo_workdir" "$isolated_cargo_home" "$source_snapshot"
trap - EXIT
