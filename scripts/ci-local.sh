#!/usr/bin/env bash
#
# ci-local.sh — run every GitHub Actions check locally, exactly as CI runs it.
#
# Mirrors .github/workflows/ one-for-one (see AGENTS.md "Before saying a change
# is ready to commit"). This is the script the pre-commit hook calls, and the
# one to run by hand before pushing.
#
#   scripts/ci-local.sh                 # everything (full CI parity)
#   scripts/ci-local.sh --changed       # only suites affected by staged files
#   scripts/ci-local.sh --fast          # everything except the audit jobs
#   scripts/ci-local.sh --dry-run       # show which checks would run
#   scripts/ci-local.sh --list          # list check ids and exit
#   scripts/ci-local.sh fmt clippy      # run only the named checks
#
# Environment:
#   SKIP_CHECKS=a,b                     # skip these check ids
#   NEXUS_CI_LOCAL_CARGO_JOBS=1         # cargo -j value (auto: 1 on <10GB RAM)
#   NEXUS_CI_LOCAL_NO_NETWORK=1         # force-skip network-dependent checks
#
# Every check reports PASS, FAIL or SKIP. A check whose tool is missing is
# reported SKIP, never PASS — a check that could not run is not a green check.
# The exit status is non-zero if any check FAILed.

set -uo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel 2>/dev/null)" || {
    echo "ci-local.sh: not inside a git repository" >&2
    exit 1
}
cd "$REPO_ROOT" || exit 1

# ── output helpers ──────────────────────────────────────────────────────────
if [[ -t 1 ]]; then IS_TTY=1; else IS_TTY=0; fi

if [[ $IS_TTY -eq 1 ]] && [[ -z "${NO_COLOR:-}" ]]; then
    C_RESET=$'\033[0m'; C_BOLD=$'\033[1m'; C_DIM=$'\033[2m'
    C_RED=$'\033[31m'; C_GREEN=$'\033[32m'; C_YELLOW=$'\033[33m'; C_BLUE=$'\033[34m'
else
    C_RESET=''; C_BOLD=''; C_DIM=''; C_RED=''; C_GREEN=''; C_YELLOW=''; C_BLUE=''
fi

log()  { printf '%s\n' "$*"; }
info() { printf '%s==>%s %s\n' "$C_BLUE$C_BOLD" "$C_RESET" "$*"; }
warn() { printf '%s[warn]%s %s\n' "$C_YELLOW" "$C_RESET" "$*"; }

# ── check registry ──────────────────────────────────────────────────────────
# id|group|workflow|description
CHECKS=(
    "workflow-yaml|meta|(all)|GitHub workflow YAML parses"
    "fmt|rust|rust.yml|cargo fmt --all -- --check"
    "pnpm-install|ts|ts.yml|pnpm install --frozen-lockfile"
    "pnpm-test|ts|ts.yml|pnpm test"
    "bundle|ts|canary.yml|Foundry adapter bundle builds"
    "boundary|rust|rust.yml|sync-engine dependency boundary (§15)"
    "clippy|rust|rust.yml|cargo clippy --workspace --all-targets -D warnings"
    "codegen|codegen|rust.yml|cargo xtask codegen leaves no drift (§3.6)"
    "cargo-test|rust|rust.yml|cargo test --workspace"
    "cargo-deny|audit|audit.yml|cargo deny check"
    "cargo-audit|audit|audit.yml|cargo audit"
    "pnpm-audit|audit|audit.yml|pnpm audit --audit-level high (informational)"
)

check_ids() { local c; for c in "${CHECKS[@]}"; do printf '%s\n' "${c%%|*}"; done; }

usage() {
    sed -n '2,28p' "$0" | sed 's/^#//; s/^ //'
    log ""
    log "Checks:"
    local c id group wf desc
    for c in "${CHECKS[@]}"; do
        IFS='|' read -r id group wf desc <<<"$c"
        printf '  %-14s %-8s %-12s %s\n' "$id" "[$group]" "$wf" "$desc"
    done
}

# ── argument parsing ────────────────────────────────────────────────────────
MODE="all"          # all | changed
RUN_AUDITS=1
DRY_RUN=0
EXPLICIT=()

while [[ $# -gt 0 ]]; do
    case "$1" in
        --changed)      MODE="changed" ;;
        --all)          MODE="all" ;;
        --fast)         RUN_AUDITS=0 ;;
        --dry-run)      DRY_RUN=1 ;;
        --list)         check_ids; exit 0 ;;
        -h|--help)      usage; exit 0 ;;
        -*)             echo "ci-local.sh: unknown flag $1" >&2; exit 2 ;;
        *)              EXPLICIT+=("$1") ;;
    esac
    shift
done

# ── which checks to run ─────────────────────────────────────────────────────
declare -A SELECTED=()
declare -A SKIP_REASON=()

select_all() { local id; while read -r id; do SELECTED["$id"]=1; done < <(check_ids); }

if [[ ${#EXPLICIT[@]} -gt 0 ]]; then
    for id in "${EXPLICIT[@]}"; do
        if check_ids | grep -qx "$id"; then
            SELECTED["$id"]=1
        else
            echo "ci-local.sh: unknown check '$id' (try --list)" >&2
            exit 2
        fi
    done
elif [[ "$MODE" == "changed" ]]; then
    # Staged paths decide which suites are worth running. Anything touching the
    # workflows themselves, or a lockfile, widens the scope back out to all.
    staged="$(git diff --cached --name-only --diff-filter=ACMR 2>/dev/null)"
    if [[ -z "$staged" ]]; then
        log "ci-local.sh: nothing staged — nothing to check."
        exit 0
    fi

    touches() { printf '%s\n' "$staged" | grep -qE "$1"; }

    rust_re='(\.rs$|(^|/)Cargo\.(toml|lock)$|^rust-toolchain|^\.cargo/|^deny\.toml$)'
    ts_re='(\.(ts|tsx|mts|cts|js|mjs|cjs)$|(^|/)package\.json$|^pnpm-lock\.yaml$|^pnpm-workspace\.yaml$|^tsconfig|^\.nvmrc$)'
    codegen_re='(^crates/schema-|^canonical\.schema\.json$|^packages/schema-|^xtask/)'
    audit_re='((^|/)Cargo\.lock$|^pnpm-lock\.yaml$|^pnpm-workspace\.yaml$|^deny\.toml$|^\.cargo/audit\.toml$)'

    SELECTED["workflow-yaml"]=1

    if touches '^\.github/workflows/'; then
        select_all
    else
        touches "$rust_re"    && for id in fmt clippy cargo-test boundary; do SELECTED["$id"]=1; done
        touches "$ts_re"      && for id in pnpm-install pnpm-test bundle; do SELECTED["$id"]=1; done
        touches "$codegen_re" && SELECTED["codegen"]=1
        touches "$audit_re"   && for id in cargo-deny cargo-audit pnpm-audit; do SELECTED["$id"]=1; done
    fi

    # cargo test loads the real adapter bundle, so it needs the bundle step even
    # when no TS file was touched.
    [[ -n "${SELECTED[cargo-test]:-}" ]] && SELECTED["bundle"]=1
else
    select_all
fi

[[ $RUN_AUDITS -eq 1 ]] || for id in cargo-deny cargo-audit pnpm-audit; do
    [[ -n "${SELECTED[$id]:-}" ]] && SKIP_REASON["$id"]="--fast"
done

IFS=',' read -ra _skips <<<"${SKIP_CHECKS:-}"
for id in "${_skips[@]}"; do
    [[ -n "$id" ]] && SKIP_REASON["$id"]="SKIP_CHECKS"
done

# ── capability probes ───────────────────────────────────────────────────────
have() { command -v "$1" >/dev/null 2>&1; }

# Git hooks run with a minimal, non-login PATH, so a Node installed through nvm,
# fnm, volta, asdf or Homebrew is visible to an interactive shell but not to the
# hook. Look in the usual places before concluding the toolchain is missing.
augment_path_for_node() {
    have pnpm && return 0

    local candidates=() d
    # nvm versions its installs; try the newest first.
    local nvm_node="${NVM_DIR:-$HOME/.nvm}/versions/node"
    if [[ -d "$nvm_node" ]]; then
        while IFS= read -r d; do
            candidates+=("$d/bin")
        done < <(find "$nvm_node" -mindepth 1 -maxdepth 1 -type d 2>/dev/null | sort -Vr)
    fi
    candidates+=(
        "$HOME/.volta/bin"
        "$HOME/.asdf/shims"
        "$HOME/Library/pnpm"        # pnpm home (macOS)
        "$HOME/.local/share/pnpm"   # pnpm home (Linux)
        "/opt/homebrew/bin"         # Homebrew (Apple silicon)
        "/usr/local/bin"            # Homebrew (Intel) / manual installs
        "/usr/bin"
    )

    for d in "${candidates[@]}"; do
        [[ -d "$d" ]] || continue
        case ":$PATH:" in *":$d:"*) continue ;; esac
        PATH="$PATH:$d"
    done
    export PATH

    have pnpm
}

# Linking the V8-based crates (deno_core) is memory hungry; on a small machine
# parallel linking gets OOM-killed, which looks like a spurious `cc` failure.
detect_cargo_jobs() {
    if [[ -n "${NEXUS_CI_LOCAL_CARGO_JOBS:-}" ]]; then
        printf '%s' "$NEXUS_CI_LOCAL_CARGO_JOBS"; return
    fi
    local mem_kb=0
    if [[ -r /proc/meminfo ]]; then
        mem_kb="$(awk '/^MemTotal:/{print $2}' /proc/meminfo)"
    elif have sysctl; then
        local bytes; bytes="$(sysctl -n hw.memsize 2>/dev/null || echo 0)"
        mem_kb=$(( bytes / 1024 ))
    fi
    # < 10 GiB total → serialise cargo to keep the linker alive.
    if [[ "$mem_kb" -gt 0 && "$mem_kb" -lt 10485760 ]]; then printf '1'; else printf '0'; fi
}

CARGO_JOBS="$(detect_cargo_jobs)"
CARGO_J=()
[[ "$CARGO_JOBS" != "0" ]] && CARGO_J=(-j "$CARGO_JOBS")

network_ok() {
    [[ -z "${NEXUS_CI_LOCAL_NO_NETWORK:-}" ]] || return 1
    have curl || return 1
    curl -sS -o /dev/null --max-time 6 https://index.crates.io/config.json 2>/dev/null
}

# ── runner ──────────────────────────────────────────────────────────────────
PASSED=(); FAILED=(); SKIPPED=()
LOG_DIR="$(mktemp -d)"
trap 'rm -rf "$LOG_DIR"' EXIT

# run_check <id> <description> <command...>
run_check() {
    local id="$1" desc="$2"; shift 2

    [[ -n "${SELECTED[$id]:-}" ]] || return 0

    if [[ -n "${SKIP_REASON[$id]:-}" ]]; then
        printf '  %sSKIP%s %-14s %s %s(%s)%s\n' \
            "$C_YELLOW" "$C_RESET" "$id" "$desc" "$C_DIM" "${SKIP_REASON[$id]}" "$C_RESET"
        SKIPPED+=("$id (${SKIP_REASON[$id]})")
        return 0
    fi

    local logf="$LOG_DIR/$id.log" start elapsed rc
    start=$SECONDS
    [[ $IS_TTY -eq 1 ]] && printf '  %s....%s %-14s %s' "$C_DIM" "$C_RESET" "$id" "$desc"

    "$@" >"$logf" 2>&1
    rc=$?
    elapsed=$(( SECONDS - start ))

    [[ $IS_TTY -eq 1 ]] && printf '\r\033[K'
    if [[ $rc -eq 0 ]]; then
        local note=''
        if grep -q '^note: ' "$logf" 2>/dev/null; then
            note=" $C_YELLOW($(sed -n 's/^note: //p' "$logf" | head -1))$C_RESET"
        fi
        printf '  %sPASS%s %-14s %s %s(%ss)%s%s\n' \
            "$C_GREEN" "$C_RESET" "$id" "$desc" "$C_DIM" "$elapsed" "$C_RESET" "$note"
        PASSED+=("$id")
    else
        printf '  %sFAIL%s %-14s %s %s(%ss)%s\n' \
            "$C_RED$C_BOLD" "$C_RESET" "$id" "$desc" "$C_DIM" "$elapsed" "$C_RESET"
        FAILED+=("$id")
        printf '%s' "$C_DIM"
        sed 's/^/       │ /' "$logf" | tail -n "${NEXUS_CI_LOCAL_LOG_LINES:-25}"
        printf '%s' "$C_RESET"
    fi
    return 0
}

# mark_skip <id> <reason>
mark_skip() { SKIP_REASON["$1"]="$2"; }

# ── check implementations ───────────────────────────────────────────────────

check_workflow_yaml() {
    local py; for py in python3 python; do have "$py" && break; done
    have "$py" || { echo "no python available to parse YAML"; return 127; }
    "$py" - <<'PY'
import glob, sys
try:
    import yaml
except ImportError:
    # PyYAML is not guaranteed; fall back to a structural sanity check so the
    # check still catches the common breakages (tabs, stray NUL, empty file).
    bad = False
    for f in sorted(glob.glob(".github/workflows/*.yml")) + sorted(glob.glob(".github/workflows/*.yaml")):
        text = open(f, encoding="utf-8").read()
        if not text.strip():
            print(f"{f}: empty"); bad = True
        if "\t" in text:
            print(f"{f}: contains a literal tab (invalid YAML indentation)"); bad = True
        if "jobs:" not in text:
            print(f"{f}: no `jobs:` key"); bad = True
    print("note: PyYAML missing — ran structural checks only")
    sys.exit(1 if bad else 0)

bad = False
for f in sorted(glob.glob(".github/workflows/*.yml")) + sorted(glob.glob(".github/workflows/*.yaml")):
    try:
        doc = yaml.safe_load(open(f, encoding="utf-8"))
    except Exception as e:
        print(f"{f}: YAML parse error: {e}"); bad = True; continue
    if not isinstance(doc, dict) or not doc.get("jobs"):
        print(f"{f}: no `jobs:` mapping"); bad = True; continue
    for name, job in doc["jobs"].items():
        if not isinstance(job, dict):
            print(f"{f}: job {name} is not a mapping"); bad = True; continue
        if "uses" in job:
            continue  # reusable workflow call
        if not job.get("runs-on"):
            print(f"{f}: job {name} has no runs-on"); bad = True
        for i, step in enumerate(job.get("steps") or []):
            if not isinstance(step, dict) or not ({"run", "uses"} & step.keys()):
                print(f"{f}: job {name} step {i} has neither run nor uses"); bad = True
sys.exit(1 if bad else 0)
PY
}

check_boundary() {
    # sync-engine must depend on schema-core and nothing else (§15).
    local crate
    for crate in store api adapter-host; do
        if cargo tree -p sync-engine --prefix none 2>/dev/null | grep -q "^$crate v"; then
            echo "sync-engine depends on $crate — §15 boundary violation"
            return 1
        fi
    done
    echo "OK: sync-engine dependency boundary intact"
}

check_codegen() {
    # CI runs `cargo xtask codegen` then `git diff --exit-code`. Here the working
    # tree is usually dirty with the change being committed, so compare only the
    # generated artefacts against their pre-codegen state.
    local generated=(
        canonical.schema.json
        packages/schema-types/src/index.ts
        packages/schema-zod/src/index.ts
    )
    local before after f rc
    before="$(git hash-object "${generated[@]}" 2>/dev/null)" || {
        echo "could not hash generated files"; return 1; }

    # codegen writes its outputs one at a time, so a mid-run failure (a missing
    # json2ts, say) leaves some regenerated and some not. Stash the originals
    # and put them back if it fails, so a failed check never mutates the tree.
    local backup; backup="$(mktemp -d)"
    for f in "${generated[@]}"; do
        mkdir -p "$backup/$(dirname "$f")"
        cp "$f" "$backup/$f" 2>/dev/null || true
    done

    cargo xtask codegen
    rc=$?
    if [[ $rc -ne 0 ]]; then
        for f in "${generated[@]}"; do
            [[ -f "$backup/$f" ]] && cp "$backup/$f" "$f"
        done
        rm -rf "$backup"
        echo "codegen failed; generated files restored to their previous contents"
        return 1
    fi
    rm -rf "$backup"

    after="$(git hash-object "${generated[@]}" 2>/dev/null)"
    if [[ "$before" != "$after" ]]; then
        echo "codegen changed generated output — Rust schema types were edited"
        echo "without regenerating. The files have now been regenerated; stage them:"
        for f in "${generated[@]}"; do echo "    git add $f"; done
        return 1
    fi
    echo "OK: generated output already up to date"
}

check_cargo_test() {
    # Mirror CI, where a missing adapter bundle must fail the suite rather than
    # let the bundle-dependent tests skip themselves and report a pass. Only
    # enforce it when a bundle actually exists, though: if the Node toolchain is
    # unavailable the bundle step could not have run, and failing here would
    # just be noise about something this machine cannot do.
    if [[ -f adapters/foundry/dist/bundle/bundle.js ]]; then
        NEXUS_REQUIRE_ADAPTER_BUNDLE=1 cargo test "${CARGO_J[@]}" --workspace
    else
        echo "note: no adapter bundle — bundle-dependent tests skipped themselves"
        cargo test "${CARGO_J[@]}" --workspace
    fi
}

check_pnpm_install() {
    # No TTY in a hook: CI=true makes pnpm purge node_modules non-interactively
    # when workspace settings changed, instead of aborting.
    CI=true pnpm install --frozen-lockfile
}

# ── preflight ───────────────────────────────────────────────────────────────
info "Local CI parity — mirrors .github/workflows/"

if [[ "$MODE" == "changed" ]]; then
    log "${C_DIM}scope: staged changes${C_RESET}"
else
    log "${C_DIM}scope: full workspace${C_RESET}"
fi

# Tool availability → SKIP (never a silent PASS).
augment_path_for_node || true

have cargo || for id in fmt clippy cargo-test boundary codegen; do mark_skip "$id" "cargo not installed"; done
# `cargo xtask codegen` shells out to `pnpm json2ts` and `pnpm
# json-schema-to-zod`, so it needs the Node toolchain as much as cargo does.
# Without it the generator dies halfway through rather than being absent.
have pnpm  || for id in pnpm-install pnpm-test bundle pnpm-audit codegen; do mark_skip "$id" "pnpm not found on PATH"; done
cargo deny  --version >/dev/null 2>&1 || mark_skip "cargo-deny"  "cargo-deny not installed (cargo install cargo-deny)"
cargo audit --version >/dev/null 2>&1 || mark_skip "cargo-audit" "cargo-audit not installed (cargo install cargo-audit)"

if [[ -n "${SELECTED[cargo-deny]:-}${SELECTED[cargo-audit]:-}${SELECTED[pnpm-audit]:-}" ]]; then
    if ! network_ok; then
        for id in cargo-deny cargo-audit pnpm-audit; do
            [[ -n "${SKIP_REASON[$id]:-}" ]] || mark_skip "$id" "no network"
        done
    fi
fi

if ! have pnpm; then
    warn "pnpm was not found on PATH, so every TypeScript check and the codegen"
    warn "staleness check will SKIP — that is a large hole in local CI parity."
    warn "Git hooks get a minimal PATH, so if pnpm works in your shell but not"
    warn "here, add its directory to PATH in the hook, or commit from a shell"
    warn "that has it. If this machine has no Node toolchain, run the checks"
    warn "where it does (e.g. your container) before pushing."
fi

if ! have docker || ! docker info >/dev/null 2>&1; then
    warn "Docker unavailable — the postgres_integration test will skip itself (as it does in CI without a service)."
fi

if [[ "$CARGO_JOBS" == "1" ]]; then
    log "${C_DIM}cargo -j 1 (low memory detected; linking deno_core/V8 needs headroom)${C_RESET}"
fi

# Warn when the working tree differs from the index: the checks below run
# against the working tree, so a pass is about the files on disk, not strictly
# about the commit's content.
if [[ "$MODE" == "changed" ]] && ! git diff --quiet 2>/dev/null; then
    warn "You have unstaged changes. Checks run against the working tree, so they"
    warn "do not test the staged snapshot in isolation."
fi

if [[ $DRY_RUN -eq 1 ]]; then
    info "Planned checks (dry run — nothing executed)"
    while read -r id; do
        [[ -n "${SELECTED[$id]:-}" ]] || continue
        if [[ -n "${SKIP_REASON[$id]:-}" ]]; then
            printf '  %sSKIP%s %-14s %s(%s)%s\n' \
                "$C_YELLOW" "$C_RESET" "$id" "$C_DIM" "${SKIP_REASON[$id]}" "$C_RESET"
        else
            printf '  %sRUN %s %-14s\n' "$C_GREEN" "$C_RESET" "$id"
        fi
    done < <(check_ids)
    exit 0
fi

log ""

# ── run ─────────────────────────────────────────────────────────────────────
run_check workflow-yaml "workflow YAML is valid"        check_workflow_yaml
run_check fmt           "cargo fmt"                     cargo fmt --all -- --check
run_check pnpm-install  "pnpm install --frozen-lockfile" check_pnpm_install
run_check pnpm-test     "pnpm test"                     pnpm test
run_check bundle        "foundry adapter bundle"        pnpm --filter @nexus-ttrpg/foundry-adapter bundle
run_check boundary      "sync-engine boundary (§15)"    check_boundary
run_check clippy        "cargo clippy -D warnings"      cargo clippy "${CARGO_J[@]}" --workspace --all-targets -- -D warnings
run_check codegen       "codegen staleness (§3.6)"      check_codegen

run_check cargo-test    "cargo test --workspace"        check_cargo_test

run_check cargo-deny    "cargo deny check"              cargo deny check
run_check cargo-audit   "cargo audit"                   cargo audit
run_check pnpm-audit    "pnpm audit (high)"             pnpm audit --audit-level high

# ── summary ─────────────────────────────────────────────────────────────────
log ""
info "Summary"
log "  ${C_GREEN}passed${C_RESET}:  ${#PASSED[@]}"
log "  ${C_RED}failed${C_RESET}:  ${#FAILED[@]}${FAILED[*]:+  (${FAILED[*]})}"
log "  ${C_YELLOW}skipped${C_RESET}: ${#SKIPPED[@]}"

if [[ ${#SKIPPED[@]} -gt 0 ]]; then
    for s in "${SKIPPED[@]}"; do log "            ${C_DIM}- $s${C_RESET}"; done
    log "  ${C_DIM}A skipped check is not a passing check — CI will still run it.${C_RESET}"
fi

if [[ ${#FAILED[@]} -gt 0 ]]; then
    log ""
    log "${C_RED}${C_BOLD}CI parity check failed.${C_RESET} Fix the above, or bypass with:"
    log "    git commit --no-verify"
    exit 1
fi

log ""
log "${C_GREEN}${C_BOLD}All selected CI checks passed.${C_RESET}"
exit 0
