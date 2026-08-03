# Goal G Amendment 10: Complete Entrypoint Bootstrap Recovery

Status: authorized for execution

Authorization date: 2026-08-03

Scope: preserve Amendment 9 evidence, authenticate retained v5, v6, and v7,
create fresh v8 solely from exact v3, repair the complete audited constructor,
runner, and Phase 0 replay bootstrap boundary, retain the closed no-scratch
review schema, and resume Goal G activation under the existing one-child
execution boundary

## Purpose

Amendment 9's two source reviews passed, exact `G9_AUTH` committed, and its
canonical 17-child launcher authenticated v3/v5/v6, copied exact v3 to fresh
v7, and post-verified the copy. The authorized constructor-bootstrap repair
was then applied. Before any other v7 edit or invocation, static mapping found
the same defect class in `run-attempt.sh`: its three identity probes, external
env/Bash storage helper, env/Python inspection target, and final clean re-exec
cannot satisfy the one-preflight-per-child boundary. Amendment 9 authorized a
behavior hunk only in the constructor, so it stopped. A separate read-only
mapper also violated the executor boundary by allowing a backticked token to
create an unintended child before its intended search. No candidate was
invoked and no patch, scratch, preview, official, Cargo, network, or trading
artifact was created.

This amendment preserves every stop and v5/v6/v7 byte-for-byte. v5, v6, and
v7 are comparison evidence only and are never mutated, invoked, promoted, or
used as bundle inputs. One closed launcher authenticates exact `G9_STOP`,
exact v3, exact retained v5/v6/v7, and every successor absence; copies exact
v3 to fresh v8 while remaining alive; and post-verifies the copy before
releasing its success record. v8 is the only new construction root.

A child-free static inventory covered all seven executable entrypoints before
this contract was frozen. Exactly three bootstrap behavior hunks are required:

1. reapply the exact Amendment 9 five-target constructor repair to fresh v8;
2. replace the runner's external env/Bash storage helper with one in-process
   BusyBox-compatible preflight, fence its three identity probes and its
   env/Python inspection target separately, and freshly fence its final clean
   re-exec;
3. replace the Phase 0 replay script's env/capture-helper-based storage check
   with the exact direct Bash `git`/`df`/`awk` preflight.

`validators.sh`, `inventory.preview.sh`, `source-reattest.preview.sh`, and
`summarize-baseline.preview.sh` launch no internal external child before their
guard definitions. Their outer `/usr/bin/env`-to-Bash shebang chains, and the
Phase 0 script's equivalent chain, are single same-PID invocation vectors
guarded by their callers. No behavior edit to those four script bodies is
authorized.

This amendment retains Amendment 9's eight-field no-scratch review schema and
supersedes only conflicting successor, status, construction/review/preview
root, activation-parent, constructor-bootstrap, and runner-bootstrap clauses
in Amendments 3 through 9. All other safety, workload, retained no-Cargo,
review, sealing, and Phase 0 requirements remain controlling.

From canonical bootstrap entry until the launcher returns success or failure,
the executor has exclusive mutation ownership of the repository, index, HEAD,
v3, retained v5, retained v6, retained v7, and every named v8, patch, reserved
scratch, preview, official, and runtime path. No concurrent session or process
may mutate any of them. After post-copy verification, the launcher separately
rechecks exact HEAD, exact tree, and clean tracked/untracked status before
releasing buffered success.

## Immutable boundary

The direct parent is:

```text
G9_STOP_commit=6e04e60fa8f2412c87d51d28136cc23546ad8805
G9_STOP_tree=583a5bdc9ee61a3276922c811d21eec092bff116
G9_STOP_parent=ed2948232dc433d5af48352206f7a7b8046f9278
G9_STOP_subject=docs: record goal g amendment 9 activation stop
G9_STOP_delta_path_count=1
G9_STOP_delta_paths=docs/polymarket-authenticated-execution-goal-g-handoff.md
G9_STOP_handoff_sha256=f3101d5e5887794454d4c503073ada18297c1be125e76a6460ff4e4b65ad6437
G9_STOP_handoff_bytes=165326
```

The successor chain is:

```text
R6 -> S4 -> T -> A5 -> G5_STOP -> G6_AUTH -> G6_STOP -> G7_AUTH -> G7_STOP -> G8_AUTH -> G8_STOP -> G9_AUTH -> G9_STOP -> G10_AUTH -> G3 -> P0
```

Historical Goal G-R retains its existing aliases. This authorization is
`G10_AUTH`; no commit may be amended, replaced, skipped, or assigned another
alias.

## Preserved statuses and artifacts

Before `G3`, the handoff contains exactly one of each:

```text
goal_g_amendment_3_status=activation-stopped-inactive
goal_g_amendment_5_status=activation-stopped-inactive
goal_g_amendment_6_status=activation-stopped-inactive
goal_g_amendment_7_status=activation-stopped-inactive
goal_g_amendment_8_status=activation-stopped-inactive
goal_g_amendment_9_status=activation-stopped-inactive
goal_g_amendment_10_status=authorized-inactive
```

Every Amendment 3, 5, 6, 7, 8, and 9 terminal block is immutable. Amendment 9
remains stopped even if Amendment 10 succeeds. Its source reviews, canonical
authentication pass, successful copy, constructor edit, and later boundary
failures retain their exact historical meanings.

Retained v5 is `/var/tmp/reap-g3-draft-v5`, device `66305`, inode `310596`,
mode `0700`, UID/GID `1000/1000`, link count `2`, and size `4096`. It has the
same ten direct regular children, component manifest, and forensic inventory
as exact v3:

```text
entry_count_including_root=11
regular_file_count=10
regular_bytes=1055725
component_manifest_rows=10
component_manifest_bytes=933
component_manifest_sha256=710ab62d5dbe846b21df74a4d78ee3f12d2a1883a22662d256bf751d411bc451
forensic_stream_bytes=1151
forensic_inventory_sha256=9ab5b0695e60cf47fc41e9d2e110b291c56b856d80cbfad36b9d41b6a5c7d233
```

Retained v6 is `/var/tmp/reap-g3-draft-v6`, device `66305`, inode `310607`,
mode `0700`, UID/GID `1000/1000`, link count `2`, and size `4096`. It has the
same exact child set, component manifest, and forensic inventory as v3 and
v5.

Retained v7 is `/var/tmp/reap-g3-draft-v7`, device `66305`, inode `310087`,
mode `0700`, UID/GID `1000/1000`, link count `2`, and size `4096`. Exactly one
child differs from v3: `construct-self-test.preview.sh` is 367844 bytes with
SHA-256 `942bc1afca185b9b0f848e667c51874cc9c650bb22d51a8fa8f262dc77161c43`.
Its other nine children remain exact v3 bytes. v7 has 1056757 regular bytes,
component-manifest SHA-256
`b81e90519bc8c74c777474867e98c486050e3276b92db00a74ee6c3c05d42804`,
and forensic-inventory stream length 1151 bytes with SHA-256
`182012c9932ef28a4981d441cc3a397a5c52c11b9aeac8f2e9079d16470a870d`.
The launcher reauthenticates v3, v5, v6, and v7 independently. Matching
aggregate hashes never substitute for per-file hashes and metadata.

## G10_AUTH commit

`G10_AUTH` is the direct child of exact `G9_STOP`, changes only:

- `docs/polymarket-authenticated-execution-goal-g-amendment-10.md`; and
- `docs/polymarket-authenticated-execution-goal-g-handoff.md`.

Its exact subject is:

```text
docs: authorize goal g amendment 10 complete entrypoint bootstrap recovery
```

The handoff appends one `goal_g_amendment_10_status=authorized-inactive` and
does not modify an earlier byte. The pre-commit contract does not contain the
future G10 commit, tree, or handoff hash. The user's 2026-08-03 instruction
authorizes this amendment's execution but does not authorize pushing its
future commits.

Two distinct read-only source reviewers in distinct sessions must pass before
the authorization commit. For review number `N` in `1,2`, the handoff binds
exactly:

```text
goal_g_amendment_10_source_review_N_result
goal_g_amendment_10_source_review_N_reviewer
goal_g_amendment_10_source_review_N_session
goal_g_amendment_10_source_review_N_contract_sha256
goal_g_amendment_10_source_review_N_bootstrap_sha256
goal_g_amendment_10_source_review_N_launcher_sha256
```

Both results are `pass`; identities and sessions are distinct; and all source
hashes equal the committed bytes.

## Exact bootstrap

The outer caller supplies the following exact UTF-8 bytes both as the
`/usr/bin/python3 -I -S -c` program and its first argument. Arguments two and
three are the absolute contract and handoff paths. The caller runs one exact
storage preflight and then `exec`s Python. No redirect, pipe, `tee`, command
substitution around the complete invocation, report, or additional predicate
is authorized.

<!-- GOAL-G-A10-BOOTSTRAP-SOURCE-BEGIN -->
```python
import hashlib, os, sys

def die(message, code=65):
    try:
        os.write(2, ("goal-g-a10-bootstrap:" + message + "\n").encode("ascii"))
    except Exception:
        pass
    raise SystemExit(code)

def read_regular(path):
    before = os.lstat(path)
    if not os.path.isfile(path) or os.path.islink(path):
        die("input-type")
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
    try:
        opened = os.fstat(fd)
        chunks = []
        while True:
            chunk = os.read(fd, 1048576)
            if not chunk:
                break
            chunks.append(chunk)
        after_fd = os.fstat(fd)
    finally:
        os.close(fd)
    after = os.lstat(path)
    key = lambda value: (value.st_dev, value.st_ino, value.st_mode,
                         value.st_uid, value.st_gid, value.st_nlink,
                         value.st_size, value.st_mtime_ns, value.st_ctime_ns)
    if key(before) != key(opened) or key(opened) != key(after_fd) or key(after_fd) != key(after):
        die("concurrent-input-change")
    return b"".join(chunks)

def field(handoff, name):
    prefix = (name + "=").encode("ascii")
    values = [line[len(prefix):] for line in handoff.splitlines() if line.startswith(prefix)]
    if len(values) != 1:
        die("handoff-field-" + name)
    try:
        return values[0].decode("ascii")
    except UnicodeDecodeError:
        die("handoff-field-encoding")

def decimal_field(handoff, name):
    value = field(handoff, name)
    canonical = value == "0" or (value and value[0] in "123456789" and
                                  all(character in "0123456789" for character in value))
    if not canonical:
        die("handoff-decimal-" + name)
    return int(value)

def main():
    if len(sys.argv) != 4:
        die("invocation-schema", 64)
    self_bytes = sys.argv[1].encode("utf-8")
    contract_path = sys.argv[2]
    handoff_path = sys.argv[3]
    if contract_path != "/home/ubuntu/code/reap/docs/polymarket-authenticated-execution-goal-g-amendment-10.md":
        die("contract-path", 64)
    if handoff_path != "/home/ubuntu/code/reap/docs/polymarket-authenticated-execution-goal-g-handoff.md":
        die("handoff-path", 64)
    contract = read_regular(contract_path)
    handoff = read_regular(handoff_path)
    if len(self_bytes) != decimal_field(handoff, "goal_g_amendment_10_bootstrap_bytes"):
        die("bootstrap-bytes")
    if hashlib.sha256(self_bytes).hexdigest() != field(handoff, "goal_g_amendment_10_bootstrap_sha256"):
        die("bootstrap-sha256")
    start = b"<!-- GOAL-G-A10-LAUNCHER-SOURCE-BEGIN -->\n```bash\n"
    end = b"\n```\n<!-- GOAL-G-A10-LAUNCHER-SOURCE-END -->"
    if contract.count(start) != 1 or contract.count(end) != 1:
        die("launcher-markers")
    source = contract.split(start, 1)[1].split(end, 1)[0]
    if b"\x00" in source:
        die("launcher-nul")
    if len(source) != decimal_field(handoff, "goal_g_amendment_10_launcher_bytes"):
        die("launcher-bytes")
    if hashlib.sha256(source).hexdigest() != field(handoff, "goal_g_amendment_10_launcher_sha256"):
        die("launcher-sha256")
    try:
        decoded = source.decode("utf-8")
    except UnicodeDecodeError:
        die("launcher-utf8")
    os.execve(
        "/bin/bash",
        ["/bin/bash", "--noprofile", "--norc", "-c", decoded,
         "goal-g-a10-authenticate-and-copy"],
        {"PATH": "/usr/bin:/bin", "LC_ALL": "C", "LANG": "C", "TZ": "UTC",
         "GIT_OPTIONAL_LOCKS": "0", "GIT_NO_REPLACE_OBJECTS": "1"},
    )

try:
    main()
except SystemExit:
    raise
except Exception:
    die("internal-runtime")
```
<!-- GOAL-G-A10-BOOTSTRAP-SOURCE-END -->

```text
bootstrap_bytes=3605
bootstrap_sha256=0c1b0948d40822043c18a0c4654e2087244cd8c0cf7cb54d9c0f23bc577e4ad4
```

## Exact authenticate-and-copy launcher

The launcher is the complete pre-v8 authority. It executes each listed
assertion exactly once. No caller or diagnostic may add, omit, reorder,
repeat, reinterpret, or aggregate a predicate. The only permitted digests are
the named source, Git/document, retained-file, component-manifest,
forensic-inventory, fixed-vector, BusyBox, and argv digests.

Every child has its own immediately adjacent storage preflight. The embedded
verifiers are child-free. The launcher remains alive across the copy: after
the pre-copy verifier passes it runs a freshly preflighted copy, then a
separately preflighted post-copy verifier, and only then emits success.
Neither authentication success nor copy success returns to the executor
before post-copy verification.

<!-- GOAL-G-A10-LAUNCHER-SOURCE-BEGIN -->
```bash
set -Eeuo pipefail

readonly REPO=/home/ubuntu/code/reap
readonly CONTRACT=$REPO/docs/polymarket-authenticated-execution-goal-g-amendment-10.md
readonly HANDOFF=$REPO/docs/polymarket-authenticated-execution-goal-g-handoff.md
readonly G9_STOP=6e04e60fa8f2412c87d51d28136cc23546ad8805
readonly G9_AUTH=ed2948232dc433d5af48352206f7a7b8046f9278
readonly G8_STOP=e4312e6ce93d411c69a0602dca0014fc41a1467e
readonly G8_AUTH=4fa757f9ff2c6d4e748b30a87f664c2710f57848
readonly G7_STOP=49210315169fa7ec3e3c02b4e70a745105bf9476
readonly G7_AUTH=32f449d3ff3db3043f3547105b9f7e1965289080
readonly G6_STOP=f06e42623d9680dbe9c2012d6300a32ae17853c5
readonly G6_AUTH=c20a95a3a45caa1cab66f878267469bff59481bf
readonly G5_STOP=dab6a252ffe25bb390da12a0459125cbeeacb7de
readonly A5=ba3b666d95d8097f60f8fc33a12b9844115edca8
readonly T=ed7d34ea504cae9d7dbb4524f6f6ebf494f5648d
readonly S4=706c4bd763647054264cdf3cb52d2355e0aa1b75
readonly R6=fc1ceba88fc91bc5c55d34fb639a4b575e584844

fail() {
  local code=$1
  local assertion=$2
  printf 'goal-g-a10:failure:%s:%s\n' "$code" "$assertion" >&2 || :
  exit "$code"
}

storage_preflight() {
  (
    set -euo pipefail
    root=$(git rev-parse --show-toplevel)
    available_bytes=$(df --output=avail -B1 "$root" |
      awk 'NR == 2 {print $1}')
    [[ $available_bytes =~ ^[0-9]+$ ]]
    (( available_bytes >= 2147483648 ))
  )
}

readonly -a CHILD_IDS=(
  repository-root repository-branch repository-clean g10-commit g10-tree
  g10-parent g10-subject g10-two-path-delta g9-stop-object g9-auth-object
  first-parent-lineage pre-copy-verifier v3-to-v8-copy post-copy-verifier
  final-g10-commit final-g10-tree final-repository-clean
)
child_index=0
begin_child() {
  local expected=$1
  (( child_index < ${#CHILD_IDS[@]} )) || fail 73 child-overrun
  [[ ${CHILD_IDS[$child_index]} == "$expected" ]] || fail 73 child-reorder
  child_index=$((child_index + 1))
}

capture_child() {
  local destination=$1
  local assertion=$2
  shift 2
  begin_child "$assertion"
  local output status
  set +e
  output=$(
    storage_preflight || exit 125
    exec "$@" 2>&1
  )
  status=$?
  set -e
  (( status != 125 )) || fail 66 "$assertion-storage-preflight"
  (( status == 0 )) || fail 66 "$assertion-child"
  printf -v "$destination" '%s' "$output"
}

readonly -a SHELL_ASSERTION_IDS=(
  invocation-schema fixed-environment repository-root repository-branch
  repository-clean g10-commit g10-tree g10-parent g10-subject g10-two-path-delta
  g9-stop-object g9-auth-object first-parent-lineage
  shell-assertion-sequence-complete
)
shell_assertion_index=0
begin_shell_assertion() {
  local expected=$1
  (( shell_assertion_index < ${#SHELL_ASSERTION_IDS[@]} )) || fail 73 assertion-overrun
  [[ ${SHELL_ASSERTION_IDS[$shell_assertion_index]} == "$expected" ]] || fail 73 assertion-reorder
  shell_assertion_index=$((shell_assertion_index + 1))
}

begin_shell_assertion invocation-schema
[[ $# -eq 0 && $0 == goal-g-a10-authenticate-and-copy ]] || fail 64 invocation-schema

begin_shell_assertion fixed-environment
[[ ${PATH-} == /usr/bin:/bin && ${LC_ALL-} == C && ${LANG-} == C && ${TZ-} == UTC ]] || fail 64 environment-core
[[ ${GIT_OPTIONAL_LOCKS-} == 0 && ${GIT_NO_REPLACE_OBJECTS-} == 1 ]] || fail 64 environment-git
[[ -z ${CARGO_HOME+x} && -z ${RUSTUP_HOME+x} && -z ${HTTP_PROXY+x} && -z ${HTTPS_PROXY+x} ]] || fail 64 environment-extra

cd "$REPO" || fail 66 repository-cd

begin_shell_assertion repository-root
capture_child repository_root repository-root /usr/bin/git rev-parse --show-toplevel
[[ $repository_root == "$REPO" ]] || fail 67 repository-root

begin_shell_assertion repository-branch
capture_child repository_branch repository-branch /usr/bin/git symbolic-ref --quiet --short HEAD
[[ $repository_branch == master ]] || fail 67 repository-branch

begin_shell_assertion repository-clean
capture_child repository_status repository-clean /usr/bin/git status --porcelain=v1 --untracked-files=all
[[ -z $repository_status ]] || fail 67 repository-clean

begin_shell_assertion g10-commit
capture_child g10_commit g10-commit /usr/bin/git rev-parse HEAD
[[ $g10_commit =~ ^[0-9a-f]{40}$ ]] || fail 67 g10-commit

begin_shell_assertion g10-tree
capture_child g10_tree g10-tree /usr/bin/git rev-parse 'HEAD^{tree}'
[[ $g10_tree =~ ^[0-9a-f]{40}$ ]] || fail 67 g10-tree

begin_shell_assertion g10-parent
capture_child g10_parent g10-parent /usr/bin/git rev-parse 'HEAD^'
[[ $g10_parent == "$G9_STOP" ]] || fail 67 g10-parent

begin_shell_assertion g10-subject
capture_child g10_subject g10-subject /usr/bin/git show -s --format=%s HEAD
[[ $g10_subject == 'docs: authorize goal g amendment 10 complete entrypoint bootstrap recovery' ]] || fail 67 g10-subject

begin_shell_assertion g10-two-path-delta
capture_child g10_delta g10-two-path-delta /usr/bin/git diff-tree --no-commit-id --name-only -r HEAD
[[ $g10_delta == $'docs/polymarket-authenticated-execution-goal-g-amendment-10.md\ndocs/polymarket-authenticated-execution-goal-g-handoff.md' ]] || fail 67 g10-two-path-delta

begin_shell_assertion g9-stop-object
capture_child g9_stop_object g9-stop-object /usr/bin/git show -s --format='%H%x09%T%x09%P%x09%s' "$G9_STOP"
[[ $g9_stop_object == $'6e04e60fa8f2412c87d51d28136cc23546ad8805\t583a5bdc9ee61a3276922c811d21eec092bff116\ted2948232dc433d5af48352206f7a7b8046f9278\tdocs: record goal g amendment 9 activation stop' ]] || fail 67 g9-stop-object

begin_shell_assertion g9-auth-object
capture_child g9_auth_object g9-auth-object /usr/bin/git show -s --format='%H%x09%T%x09%P%x09%s' "$G9_AUTH"
[[ $g9_auth_object == $'ed2948232dc433d5af48352206f7a7b8046f9278\t5c6ef7a138bfc7f9ffca1b5e93233a356bbedc9a\te4312e6ce93d411c69a0602dca0014fc41a1467e\tdocs: authorize goal g amendment 9 constructor bootstrap and review schema recovery' ]] || fail 67 g9-auth-object

begin_shell_assertion first-parent-lineage
capture_child lineage first-parent-lineage /usr/bin/git rev-list --first-parent --max-count=14 HEAD
[[ $lineage == "$g10_commit"$'\n'"$G9_STOP"$'\n'"$G9_AUTH"$'\n'"$G8_STOP"$'\n'"$G8_AUTH"$'\n'"$G7_STOP"$'\n'"$G7_AUTH"$'\n'"$G6_STOP"$'\n'"$G6_AUTH"$'\n'"$G5_STOP"$'\n'"$A5"$'\n'"$T"$'\n'"$S4"$'\n'"$R6" ]] || fail 67 first-parent-lineage

begin_shell_assertion shell-assertion-sequence-complete
(( shell_assertion_index == ${#SHELL_ASSERTION_IDS[@]} )) || fail 73 assertion-underrun

export GOAL_G_A10_REPO=$REPO
export GOAL_G_A10_CONTRACT=$CONTRACT
export GOAL_G_A10_HANDOFF=$HANDOFF
export GOAL_G_A10_G10_COMMIT=$g10_commit
export GOAL_G_A10_G10_TREE=$g10_tree
export GOAL_G_A10_G10_PARENT=$g10_parent
export GOAL_G_A10_G10_SUBJECT=$g10_subject

begin_child pre-copy-verifier
set +e
pre_output=$(
  storage_preflight || exit 125
  exec /usr/bin/python3 -I -S <<'PY'
import hashlib, os, stat

ASSERTION_IDS = (
    "handoff-status-and-cross-binding",
    "source-review-evidence",
    "a9-terminal-evidence",
    "fixed-forensic-vector",
    "retained-v5-v6-v7-trees",
    "required-absence-set",
    "busybox-and-argv",
    "final-v3-source-tree",
)
index = 0

def abort(code, assertion, detail):
    try:
        os.write(2, ("goal-g-a10:failure:%d:%s:%s\n" %
                     (code, assertion, detail)).encode("ascii", "backslashreplace"))
    except Exception:
        pass
    raise SystemExit(code)

def begin(assertion):
    global index
    if index >= len(ASSERTION_IDS) or ASSERTION_IDS[index] != assertion:
        abort(73, assertion, "assertion-sequence")
    index += 1

def equal(actual, expected, code, assertion, detail):
    if actual != expected:
        abort(code, assertion, detail)

def metadata(value):
    return (value.st_dev, value.st_ino, value.st_mode, value.st_uid,
            value.st_gid, value.st_nlink, value.st_size, value.st_mtime_ns,
            value.st_ctime_ns)

def stable_file(path, assertion):
    try:
        before = os.lstat(path)
        if not stat.S_ISREG(before.st_mode):
            abort(68, assertion, "not-regular")
        fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
        opened = os.fstat(fd)
        digest = hashlib.sha256()
        chunks = []
        while True:
            chunk = os.read(fd, 1048576)
            if not chunk:
                break
            chunks.append(chunk)
            digest.update(chunk)
        after_fd = os.fstat(fd)
        os.close(fd)
        after = os.lstat(path)
    except SystemExit:
        raise
    except Exception:
        abort(72, assertion, "stable-read-runtime")
    if metadata(before) != metadata(opened) or metadata(opened) != metadata(after_fd) or metadata(after_fd) != metadata(after):
        abort(72, assertion, "stable-read-cut")
    return b"".join(chunks), digest.hexdigest(), before

def fields(data):
    result = {}
    for line in data.splitlines():
        if b"=" not in line:
            continue
        key, value = line.split(b"=", 1)
        if not key.startswith(b"goal_g_"):
            continue
        try:
            decoded_key = key.decode("ascii")
            decoded_value = value.decode("ascii")
        except UnicodeDecodeError:
            abort(67, "handoff-status-and-cross-binding", "field-encoding")
        result.setdefault(decoded_key, []).append(decoded_value)
    return result

def unique(values, name, assertion):
    found = values.get(name, [])
    if len(found) != 1:
        abort(67, assertion, "field-" + name)
    return found[0]

V3_FILES = {
    "SELF-TEST-DESIGN.md": ("4f739c6f49d90418ba1e1576bf2f4015f1da9a4b9b8eed9ffa3de9414d21c5a4", 44806, 0o664),
    "SELF-TEST-SCHEMA.md": ("a4d8e7ae085bd2517678e0762690c813d2e69232d463e3df83ec9956faf27ecd", 24089, 0o664),
    "commands.tsv": ("89d0e03b192d03ba34d8680616f0c5484010cb06ec3cc59813b66a8c4b0abb7f", 5509, 0o664),
    "construct-self-test.preview.sh": ("7f16928835d296353d6cc94501bd3cabd6f7febc7da044606673d7ee287c9bba", 366812, 0o664),
    "inventory.preview.sh": ("d102c9ddc68cf0eb7fad72308bd86fa986dca52e2dbc0c8346e98a11fe9cf84c", 53408, 0o664),
    "run-attempt.sh": ("86a79706b6aa8253b7d8fb298c5016535aab33a2cd91f4c842b3c2d06c72ddcd", 217156, 0o664),
    "run-phase0-replay.preview.sh": ("f4b7a52322a0568b19b1e515cb3ec998e827ccbd0ac25abcce0ddd11eddbb2a7", 100443, 0o664),
    "source-reattest.preview.sh": ("ff1a11823e39b73682c0b77a614f356c17a17907b29855e7d2c7dbeca9bfbd76", 22544, 0o664),
    "summarize-baseline.preview.sh": ("8c4a006f1eea1c077322bb2baaec195fc2cc8bac52d4ca7fe3d03b6772799f2d", 82593, 0o664),
    "validators.sh": ("897f3bb05418397d8d17944dea70501a1bb2adbbf65c73acc06035726eab678b", 138365, 0o700),
}

V7_FILES = dict(V3_FILES)
V7_FILES["construct-self-test.preview.sh"] = (
    "942bc1afca185b9b0f848e667c51874cc9c650bb22d51a8fa8f262dc77161c43",
    367844,
    0o664,
)

def direct_tree(path, inode, assertion, files=V3_FILES,
                expected_total=1055725,
                expected_component_hash="710ab62d5dbe846b21df74a4d78ee3f12d2a1883a22662d256bf751d411bc451",
                expected_inventory_bytes=1151,
                expected_inventory_hash="9ab5b0695e60cf47fc41e9d2e110b291c56b856d80cbfad36b9d41b6a5c7d233"):
    root = os.fsencode(path)
    before = os.lstat(root)
    if not stat.S_ISDIR(before.st_mode):
        abort(68, assertion, "root-type")
    equal((before.st_dev, before.st_ino, before.st_mode & 0o7777,
           before.st_uid, before.st_gid, before.st_nlink, before.st_size),
          (66305, inode, 0o700, 1000, 1000, 2, 4096), 68, assertion,
          "root-metadata")
    names = sorted(os.listdir(root))
    wanted = sorted(name.encode("ascii") for name in files)
    equal(names, wanted, 68, assertion, "child-set")
    component = bytearray()
    records = [(b".", b".\x00d\x000700\x001000\x001000\x002\x004096\x00-\n")]
    total = 0
    snapshots = {}
    for raw in wanted:
        name = raw.decode("ascii")
        data, digest, entry = stable_file(os.path.join(root, raw), assertion)
        expected_hash, expected_bytes, expected_mode = files[name]
        equal((digest, len(data), entry.st_mode & 0o7777, entry.st_uid,
               entry.st_gid, entry.st_nlink),
              (expected_hash, expected_bytes, expected_mode, 1000, 1000, 1),
              68, assertion, "file-" + name)
        component.extend((digest + "\t" + str(len(data)) + "\t" + name + "\n").encode("ascii"))
        record = (raw + b"\0f\0" + ("%04o" % (entry.st_mode & 0o7777)).encode() +
                  b"\0" + str(entry.st_uid).encode() + b"\0" + str(entry.st_gid).encode() +
                  b"\0" + str(entry.st_nlink).encode() + b"\0" + str(entry.st_size).encode() +
                  b"\0" + digest.encode() + b"\n")
        records.append((raw, record)); total += len(data); snapshots[raw] = metadata(entry)
    after = os.lstat(root)
    equal(metadata(after), metadata(before), 72, assertion, "root-stability")
    for raw in wanted:
        equal(metadata(os.lstat(os.path.join(root, raw))), snapshots[raw],
              72, assertion, "file-stability")
    inventory = b"".join(record for _, record in sorted(records))
    equal((total, len(component.splitlines()), len(component), hashlib.sha256(component).hexdigest()),
          (expected_total, 10, 933, expected_component_hash),
          69, assertion, "component")
    equal((len(records), len(inventory), hashlib.sha256(inventory).hexdigest()),
          (11, expected_inventory_bytes, expected_inventory_hash),
          70, assertion, "forensic")
    return records

def ensure_absent(path, assertion):
    if not path.startswith("/") or os.path.normpath(path) != path:
        abort(71, assertion, "noncanonical")
    current = "/"
    parts = [part for part in path.split("/") if part]
    for position, part in enumerate(parts):
        current = os.path.join(current, part)
        try:
            entry = os.lstat(current)
        except FileNotFoundError:
            return
        except Exception:
            abort(71, assertion, "absence-runtime")
        if stat.S_ISLNK(entry.st_mode):
            abort(71, assertion, "linked-ancestor")
        if position == len(parts) - 1:
            abort(71, assertion, "path-present")
        if not stat.S_ISDIR(entry.st_mode):
            abort(71, assertion, "ancestor-type")
    abort(71, assertion, "path-present")

try:
    repo = os.environ["GOAL_G_A10_REPO"]
    contract_path = os.environ["GOAL_G_A10_CONTRACT"]
    handoff_path = os.environ["GOAL_G_A10_HANDOFF"]
    g10_commit = os.environ["GOAL_G_A10_G10_COMMIT"]
    g10_tree = os.environ["GOAL_G_A10_G10_TREE"]
    g10_parent = os.environ["GOAL_G_A10_G10_PARENT"]
    g10_subject = os.environ["GOAL_G_A10_G10_SUBJECT"]

    begin("handoff-status-and-cross-binding")
    _, contract_hash, _ = stable_file(contract_path, "handoff-status-and-cross-binding")
    handoff_data, handoff_hash, _ = stable_file(handoff_path, "handoff-status-and-cross-binding")
    parent_handoff_bytes = 165326
    equal(len(handoff_data) > parent_handoff_bytes, True, 67,
          "handoff-status-and-cross-binding", "appended-length")
    equal(hashlib.sha256(handoff_data[:parent_handoff_bytes]).hexdigest(),
          "f3101d5e5887794454d4c503073ada18297c1be125e76a6460ff4e4b65ad6437",
          67, "handoff-status-and-cross-binding", "parent-prefix")
    suffix = handoff_data[parent_handoff_bytes:]
    suffix_start = b"\n## User-Authorized Amendment 10 \xe2\x80\x94 2026-08-03\n"
    equal((suffix.startswith(suffix_start), suffix.endswith(b"\n```\n"),
           suffix.count(b"\n```text\n"), suffix.count(b"\n```\n")),
          (True, True, 1, 1), 67, "handoff-status-and-cross-binding",
          "closed-amendment-suffix")
    values = fields(handoff_data)
    for name, expected in (
        ("goal_g_amendment_3_status", "activation-stopped-inactive"),
        ("goal_g_amendment_5_status", "activation-stopped-inactive"),
        ("goal_g_amendment_6_status", "activation-stopped-inactive"),
        ("goal_g_amendment_7_status", "activation-stopped-inactive"),
        ("goal_g_amendment_8_status", "activation-stopped-inactive"),
        ("goal_g_amendment_9_status", "activation-stopped-inactive"),
        ("goal_g_amendment_10_status", "authorized-inactive"),
        ("goal_g_amendment_10_contract_sha256", contract_hash),
        ("goal_g_amendment_10_parent_commit", "6e04e60fa8f2412c87d51d28136cc23546ad8805"),
        ("goal_g_amendment_10_parent_handoff_sha256", "f3101d5e5887794454d4c503073ada18297c1be125e76a6460ff4e4b65ad6437"),
        ("goal_g_amendment_10_parent_handoff_bytes", "165326"),
        ("goal_g_amendment_10_authorization_subject", "docs: authorize goal g amendment 10 complete entrypoint bootstrap recovery"),
        ("goal_g_amendment_10_lineage", "R6->S4->T->A5->G5_STOP->G6_AUTH->G6_STOP->G7_AUTH->G7_STOP->G8_AUTH->G8_STOP->G9_AUTH->G9_STOP->G10_AUTH->G3->P0"),
    ):
        equal(unique(values, name, "handoff-status-and-cross-binding"), expected,
              67, "handoff-status-and-cross-binding", name)
    equal((g10_parent, g10_subject),
          ("6e04e60fa8f2412c87d51d28136cc23546ad8805",
           "docs: authorize goal g amendment 10 complete entrypoint bootstrap recovery"),
          67, "handoff-status-and-cross-binding", "g10-runtime")

    begin("source-review-evidence")
    bootstrap_hash = unique(values, "goal_g_amendment_10_bootstrap_sha256", "source-review-evidence")
    launcher_hash = unique(values, "goal_g_amendment_10_launcher_sha256", "source-review-evidence")
    allowed_review_keys = set()
    for number, reviewer, session in (
        ("1", "g10-contract-review-1", "g10-contract-review-1-20260803"),
        ("2", "g10-boundary-review-2", "g10-boundary-review-2-20260803"),
    ):
        prefix = "goal_g_amendment_10_source_review_" + number + "_"
        expected = {
            prefix + "result": "pass", prefix + "reviewer": reviewer,
            prefix + "session": session, prefix + "contract_sha256": contract_hash,
            prefix + "bootstrap_sha256": bootstrap_hash,
            prefix + "launcher_sha256": launcher_hash,
        }
        allowed_review_keys.update(expected)
        actual_keys = {name for name in values if name.startswith(prefix)}
        equal(actual_keys, set(expected), 67, "source-review-evidence",
              prefix + "exact-key-set")
        for name, wanted in expected.items():
            equal(unique(values, name, "source-review-evidence"), wanted,
                  67, "source-review-evidence", name)
    actual_review_keys = {
        name for name in values
        if name.startswith("goal_g_amendment_10_source_review_")
    }
    equal(actual_review_keys, allowed_review_keys, 67, "source-review-evidence",
          "exact-review-namespace")

    begin("a9-terminal-evidence")
    for name, expected in (
        ("goal_g_amendment_9_activation_stop_status", "stopped"),
        ("goal_g_amendment_9_activation_stop_parent_commit", "ed2948232dc433d5af48352206f7a7b8046f9278"),
        ("goal_g_amendment_9_activation_stop_parent_handoff_sha256", "9e767561029a1bb458b85d12eb5780af97dc43a73cd81b9e827c20771bde069e"),
        ("goal_g_amendment_9_activation_stop_primary_failure_class", "authorized-edit-surface-cannot-satisfy-runner-bootstrap-boundary"),
        ("goal_g_amendment_9_activation_stop_secondary_failure_class", "unintended-command-substitution-child-and-preflight-reuse"),
        ("goal_g_amendment_9_activation_stop_canonical_authentication_result", "pass"),
        ("goal_g_amendment_9_activation_stop_copy_exit", "0"),
        ("goal_g_amendment_9_activation_stop_v7_state", "retained-non-authoritative-partially-constructed-bootstrap-only-not-invoked"),
    ):
        equal(unique(values, name, "a9-terminal-evidence"), expected,
              67, "a9-terminal-evidence", name)

    begin("fixed-forensic-vector")
    vector = (b".\x00d\x000700\x001000\x001000\x002\x004096\x00-\n"
              b"a\x00f\x000644\x001000\x001000\x001\x003\x00"
              b"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad\n")
    equal((len(vector), hashlib.sha256(vector).hexdigest()),
          (116, "63ed0e2d6f3f43abc06cce1dd215d166131f25132b645ec6c027b50d1629c9c0"),
          70, "fixed-forensic-vector", "vector")

    begin("retained-v5-v6-v7-trees")
    direct_tree("/var/tmp/reap-g3-draft-v5", 310596, "retained-v5-v6-v7-trees")
    direct_tree("/var/tmp/reap-g3-draft-v6", 310607, "retained-v5-v6-v7-trees")
    direct_tree(
        "/var/tmp/reap-g3-draft-v7", 310087, "retained-v5-v6-v7-trees",
        V7_FILES, 1056757,
        "b81e90519bc8c74c777474867e98c486050e3276b92db00a74ee6c3c05d42804",
        1151,
        "182012c9932ef28a4981d441cc3a397a5c52c11b9aeac8f2e9079d16470a870d",
    )

    begin("required-absence-set")
    for path in (
        "/var/tmp/reap-g3-draft-v4",
        "/var/tmp/reap-g3-draft-v4-provenance.patch",
        "/var/tmp/reap-g3-draft-v4-review-1-scratch",
        "/var/tmp/reap-g3-draft-v4-review-2-scratch",
        repo + "/target/tmp/goal-g-amendment-3-preview-v3",
        "/var/tmp/reap-g3-draft-v5-provenance.patch",
        "/var/tmp/reap-g3-draft-v5-review-1-scratch",
        "/var/tmp/reap-g3-draft-v5-review-2-scratch",
        repo + "/target/tmp/goal-g-amendment-3-preview-v4",
        "/var/tmp/reap-g3-draft-v6-provenance.patch",
        "/var/tmp/reap-g3-draft-v6-review-1-scratch",
        "/var/tmp/reap-g3-draft-v6-review-2-scratch",
        repo + "/target/tmp/goal-g-amendment-3-preview-v5",
        "/var/tmp/reap-g3-draft-v7-provenance.patch",
        "/var/tmp/reap-g3-draft-v7-review-1-scratch",
        "/var/tmp/reap-g3-draft-v7-review-2-scratch",
        repo + "/target/tmp/goal-g-amendment-3-preview-v6",
        "/var/tmp/reap-g3-draft-v8",
        "/var/tmp/reap-g3-draft-v8-provenance.patch",
        "/var/tmp/reap-g3-draft-v8-review-1-scratch",
        "/var/tmp/reap-g3-draft-v8-review-2-scratch",
        repo + "/target/tmp/goal-g-amendment-3-preview-v7",
        repo + "/target/tmp/goal-g-amendment-3-recorder-bundle",
        repo + "/target/tmp/goal-g-phase0-amendment-3",
        repo + "/target/tmp/goal-g-amendment-3-runtime",
    ):
        ensure_absent(path, "required-absence-set")

    begin("busybox-and-argv")
    _, busybox_hash, _ = stable_file("/bin/busybox", "busybox-and-argv")
    equal(busybox_hash, "c2f279d1d5640a0f327890d41cad594c0f059f3fed3f96dd72fdcc4f5e18ec02",
          68, "busybox-and-argv", "busybox")
    argv = ("/bin/busybox", "cp", "-a", "--",
            "/var/tmp/reap-g3-draft-v3", "/var/tmp/reap-g3-draft-v8")
    stream = b"".join(value.encode() + b"\0" for value in argv)
    equal((len(argv), len(stream), hashlib.sha256(stream).hexdigest()),
          (6, 74, "902a72f92276f193879fa128fed96ec905fcd6327789df03115b62fb3abca1e6"),
          68, "busybox-and-argv", "copy-argv")

    begin("final-v3-source-tree")
    records = direct_tree("/var/tmp/reap-g3-draft-v3", 310585, "final-v3-source-tree")
    root_record = [record for rel, record in records if rel == b"."][0]
    equal((len(root_record), hashlib.sha256(root_record).hexdigest()),
          (28, "5c5f2aa15f151a1c1fd8285ee13c42e968e17889c99ad85c06e544080824ba81"),
          70, "final-v3-source-tree", "root-record")

    equal(index, len(ASSERTION_IDS), 73, "assertion-sequence", "underrun")
    for name, value in (
        ("goal_g_a10_pre_contract_sha256", contract_hash),
        ("goal_g_a10_pre_handoff_sha256", handoff_hash),
    ):
        os.write(1, (name + "=" + value + "\n").encode("ascii"))
except SystemExit:
    raise
except Exception:
    abort(72, "internal-runtime", "unexpected-exception")
PY
)
verifier_status=$?
set -e
case $verifier_status in
  0) ;;
  125) fail 66 verifier-storage-preflight ;;
  67|68|69|70|71|72|73) exit "$verifier_status" ;;
  *) fail 66 verifier-child-runtime ;;
esac

[[ $pre_output == *$'\n'* && $pre_output != *$'\n'*$'\n'* ]] || fail 67 verifier-output-schema
pre_contract_line=${pre_output%%$'\n'*}
pre_handoff_line=${pre_output#*$'\n'}
pre_contract_sha256=${pre_contract_line#goal_g_a10_pre_contract_sha256=}
pre_handoff_sha256=${pre_handoff_line#goal_g_a10_pre_handoff_sha256=}
[[ $pre_contract_line == goal_g_a10_pre_contract_sha256="$pre_contract_sha256" && $pre_contract_sha256 =~ ^[0-9a-f]{64}$ ]] || fail 67 verifier-contract-output
[[ $pre_handoff_line == goal_g_a10_pre_handoff_sha256="$pre_handoff_sha256" && $pre_handoff_sha256 =~ ^[0-9a-f]{64}$ ]] || fail 67 verifier-handoff-output
readonly pre_contract_sha256 pre_handoff_sha256
export GOAL_G_A10_PRE_CONTRACT_SHA256=$pre_contract_sha256
export GOAL_G_A10_PRE_HANDOFF_SHA256=$pre_handoff_sha256

begin_child v3-to-v8-copy
set +e
(
  storage_preflight || exit 125
  exec /bin/busybox cp -a -- /var/tmp/reap-g3-draft-v3 /var/tmp/reap-g3-draft-v8
)
copy_status=$?
set -e
(( copy_status != 125 )) || fail 66 final-copy-storage-preflight
(( copy_status == 0 )) || fail 66 copy-child

begin_child post-copy-verifier
set +e
post_output=$(
  storage_preflight || exit 125
  exec /usr/bin/python3 -I -S <<'PY'
import hashlib, os, stat

def abort(code, detail):
    try:
        os.write(2, ("goal-g-a10:post-copy-failure:%d:%s\n" %
                     (code, detail)).encode("ascii", "backslashreplace"))
    except Exception:
        pass
    raise SystemExit(code)

def equal(actual, expected, code, detail):
    if actual != expected:
        abort(code, detail)

def metadata(value):
    return (value.st_dev, value.st_ino, value.st_mode, value.st_uid,
            value.st_gid, value.st_nlink, value.st_size, value.st_mtime_ns,
            value.st_ctime_ns)

def stable_file(path):
    try:
        before = os.lstat(path)
        if not stat.S_ISREG(before.st_mode):
            abort(68, "not-regular")
        fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
        opened = os.fstat(fd)
        digest = hashlib.sha256()
        size = 0
        while True:
            chunk = os.read(fd, 1048576)
            if not chunk:
                break
            digest.update(chunk)
            size += len(chunk)
        after_fd = os.fstat(fd)
        os.close(fd)
        after = os.lstat(path)
    except SystemExit:
        raise
    except Exception:
        abort(72, "stable-read-runtime")
    if metadata(before) != metadata(opened) or metadata(opened) != metadata(after_fd) or metadata(after_fd) != metadata(after):
        abort(72, "stable-read-cut")
    return digest.hexdigest(), size, before

V3_FILES = {
    "SELF-TEST-DESIGN.md": ("4f739c6f49d90418ba1e1576bf2f4015f1da9a4b9b8eed9ffa3de9414d21c5a4", 44806, 0o664),
    "SELF-TEST-SCHEMA.md": ("a4d8e7ae085bd2517678e0762690c813d2e69232d463e3df83ec9956faf27ecd", 24089, 0o664),
    "commands.tsv": ("89d0e03b192d03ba34d8680616f0c5484010cb06ec3cc59813b66a8c4b0abb7f", 5509, 0o664),
    "construct-self-test.preview.sh": ("7f16928835d296353d6cc94501bd3cabd6f7febc7da044606673d7ee287c9bba", 366812, 0o664),
    "inventory.preview.sh": ("d102c9ddc68cf0eb7fad72308bd86fa986dca52e2dbc0c8346e98a11fe9cf84c", 53408, 0o664),
    "run-attempt.sh": ("86a79706b6aa8253b7d8fb298c5016535aab33a2cd91f4c842b3c2d06c72ddcd", 217156, 0o664),
    "run-phase0-replay.preview.sh": ("f4b7a52322a0568b19b1e515cb3ec998e827ccbd0ac25abcce0ddd11eddbb2a7", 100443, 0o664),
    "source-reattest.preview.sh": ("ff1a11823e39b73682c0b77a614f356c17a17907b29855e7d2c7dbeca9bfbd76", 22544, 0o664),
    "summarize-baseline.preview.sh": ("8c4a006f1eea1c077322bb2baaec195fc2cc8bac52d4ca7fe3d03b6772799f2d", 82593, 0o664),
    "validators.sh": ("897f3bb05418397d8d17944dea70501a1bb2adbbf65c73acc06035726eab678b", 138365, 0o700),
}

V7_FILES = dict(V3_FILES)
V7_FILES["construct-self-test.preview.sh"] = (
    "942bc1afca185b9b0f848e667c51874cc9c650bb22d51a8fa8f262dc77161c43",
    367844,
    0o664,
)

def direct_tree(path, expected_inode, allow_fresh_inode=False,
                files=V3_FILES, expected_total=1055725,
                expected_component_hash="710ab62d5dbe846b21df74a4d78ee3f12d2a1883a22662d256bf751d411bc451",
                expected_inventory_bytes=1151,
                expected_inventory_hash="9ab5b0695e60cf47fc41e9d2e110b291c56b856d80cbfad36b9d41b6a5c7d233"):
    root = os.fsencode(path)
    before = os.lstat(root)
    if not stat.S_ISDIR(before.st_mode):
        abort(68, "root-type")
    if allow_fresh_inode:
        if before.st_dev != 66305 or before.st_ino in (310585, 310596, 310607, 310087):
            abort(68, "fresh-root-identity")
    else:
        equal((before.st_dev, before.st_ino), (66305, expected_inode),
              68, "root-identity")
    equal((before.st_mode & 0o7777, before.st_uid, before.st_gid,
           before.st_nlink, before.st_size),
          (0o700, 1000, 1000, 2, 4096), 68, "root-metadata")
    names = sorted(os.listdir(root))
    wanted = sorted(name.encode("ascii") for name in files)
    equal(names, wanted, 68, "child-set")
    component = bytearray()
    records = [(b".", b".\x00d\x000700\x001000\x001000\x002\x004096\x00-\n")]
    total = 0
    snapshots = {}
    for raw in wanted:
        name = raw.decode("ascii")
        digest, size, entry = stable_file(os.path.join(root, raw))
        expected_hash, expected_size, expected_mode = files[name]
        equal((digest, size, entry.st_mode & 0o7777, entry.st_uid,
               entry.st_gid, entry.st_nlink),
              (expected_hash, expected_size, expected_mode, 1000, 1000, 1),
              68, "file-" + name)
        component.extend((digest + "\t" + str(size) + "\t" + name + "\n").encode())
        record = (raw + b"\x00f\x00" +
                  ("%04o" % (entry.st_mode & 0o7777)).encode() + b"\x00" +
                  str(entry.st_uid).encode() + b"\x00" +
                  str(entry.st_gid).encode() + b"\x00" +
                  str(entry.st_nlink).encode() + b"\x00" +
                  str(entry.st_size).encode() + b"\x00" +
                  digest.encode() + b"\n")
        records.append((raw, record))
        total += size
        snapshots[raw] = metadata(entry)
    equal(metadata(os.lstat(root)), metadata(before), 72, "root-stability")
    for raw in wanted:
        equal(metadata(os.lstat(os.path.join(root, raw))), snapshots[raw],
              72, "file-stability")
    inventory = b"".join(record for _, record in sorted(records))
    equal((total, len(component.splitlines()), len(component),
           hashlib.sha256(component).hexdigest()),
          (expected_total, 10, 933, expected_component_hash),
          69, "component")
    equal((len(records), len(inventory), hashlib.sha256(inventory).hexdigest()),
          (11, expected_inventory_bytes, expected_inventory_hash),
          70, "forensic")
    return before.st_dev, before.st_ino

def stable_document(path):
    digest, size, _ = stable_file(path)
    return digest, size

try:
    v3 = direct_tree("/var/tmp/reap-g3-draft-v3", 310585)
    v5 = direct_tree("/var/tmp/reap-g3-draft-v5", 310596)
    v6 = direct_tree("/var/tmp/reap-g3-draft-v6", 310607)
    v7 = direct_tree(
        "/var/tmp/reap-g3-draft-v7", 310087, False, V7_FILES, 1056757,
        "b81e90519bc8c74c777474867e98c486050e3276b92db00a74ee6c3c05d42804",
        1151,
        "182012c9932ef28a4981d441cc3a397a5c52c11b9aeac8f2e9079d16470a870d",
    )
    v8 = direct_tree("/var/tmp/reap-g3-draft-v8", None, True)
    if v8 in (v3, v5, v6, v7):
        abort(68, "fresh-root-alias")
    for path in (
        "/var/tmp/reap-g3-draft-v6-provenance.patch",
        "/var/tmp/reap-g3-draft-v6-review-1-scratch",
        "/var/tmp/reap-g3-draft-v6-review-2-scratch",
        os.environ["GOAL_G_A10_REPO"] + "/target/tmp/goal-g-amendment-3-preview-v5",
        "/var/tmp/reap-g3-draft-v7-provenance.patch",
        "/var/tmp/reap-g3-draft-v7-review-1-scratch",
        "/var/tmp/reap-g3-draft-v7-review-2-scratch",
        os.environ["GOAL_G_A10_REPO"] + "/target/tmp/goal-g-amendment-3-preview-v6",
        "/var/tmp/reap-g3-draft-v8-provenance.patch",
        "/var/tmp/reap-g3-draft-v8-review-1-scratch",
        "/var/tmp/reap-g3-draft-v8-review-2-scratch",
        os.environ["GOAL_G_A10_REPO"] + "/target/tmp/goal-g-amendment-3-preview-v7",
        os.environ["GOAL_G_A10_REPO"] + "/target/tmp/goal-g-amendment-3-recorder-bundle",
        os.environ["GOAL_G_A10_REPO"] + "/target/tmp/goal-g-phase0-amendment-3",
        os.environ["GOAL_G_A10_REPO"] + "/target/tmp/goal-g-amendment-3-runtime",
    ):
        if os.path.lexists(path):
            abort(71, "post-copy-auxiliary-present")
    contract_hash, _ = stable_document(os.environ["GOAL_G_A10_CONTRACT"])
    handoff_hash, _ = stable_document(os.environ["GOAL_G_A10_HANDOFF"])
    equal(contract_hash, os.environ["GOAL_G_A10_PRE_CONTRACT_SHA256"],
          72, "contract-pre-post-hash")
    equal(handoff_hash, os.environ["GOAL_G_A10_PRE_HANDOFF_SHA256"],
          72, "handoff-pre-post-hash")
    for name, value in (
        ("goal_g_amendment_10_result", "authenticate-copy-postverify-pass"),
        ("goal_g_amendment_10_g10_auth_commit", os.environ["GOAL_G_A10_G10_COMMIT"]),
        ("goal_g_amendment_10_g10_auth_tree", os.environ["GOAL_G_A10_G10_TREE"]),
        ("goal_g_amendment_10_g10_auth_parent", os.environ["GOAL_G_A10_G10_PARENT"]),
        ("goal_g_amendment_10_g10_auth_subject", os.environ["GOAL_G_A10_G10_SUBJECT"]),
        ("goal_g_amendment_10_g10_auth_contract_sha256", contract_hash),
        ("goal_g_amendment_10_g10_auth_handoff_sha256", handoff_hash),
        ("goal_g_amendment_10_v8_root_dev", str(v8[0])),
        ("goal_g_amendment_10_v8_root_inode", str(v8[1])),
        ("goal_g_amendment_10_v8_forensic_inventory_sha256",
         "9ab5b0695e60cf47fc41e9d2e110b291c56b856d80cbfad36b9d41b6a5c7d233"),
    ):
        os.write(1, (name + "=" + value + "\n").encode("ascii"))
except SystemExit:
    raise
except Exception:
    abort(72, "unexpected-exception")
PY
)
post_status=$?
set -e
case $post_status in
  0) ;;
  125) fail 66 post-copy-verifier-storage-preflight ;;
  67|68|69|70|71|72|73) exit "$post_status" ;;
  *) fail 66 post-copy-verifier-child-runtime ;;
esac

capture_child final_g10_commit final-g10-commit /usr/bin/git rev-parse HEAD
[[ $final_g10_commit == "$g10_commit" ]] || fail 67 final-g10-commit

capture_child final_g10_tree final-g10-tree /usr/bin/git rev-parse 'HEAD^{tree}'
[[ $final_g10_tree == "$g10_tree" ]] || fail 67 final-g10-tree

capture_child final_repository_status final-repository-clean /usr/bin/git status --porcelain=v1 --untracked-files=all
[[ -z $final_repository_status ]] || fail 67 final-repository-clean

(( child_index == ${#CHILD_IDS[@]} )) || fail 73 child-underrun
printf '%s\n' "$post_output"
```
<!-- GOAL-G-A10-LAUNCHER-SOURCE-END -->

```text
launcher_bytes=34433
launcher_sha256=9034e3bbd18654d949a881dd8bb668272c0f680a8c6854fb38f45275a898c355
launcher_external_child_count=17
launcher_storage_preflight_count=17
launcher_post_copy_success_gap=false
```

The exit classes are:

```text
64 invocation-schema
65 bootstrap-or-launcher-source-mismatch
66 child-runtime-or-storage-preflight-failure
67 repository-lineage-status-contract-or-evidence-mismatch
68 retained-entry-identity-or-content-mismatch
69 component-manifest-mismatch
70 forensic-inventory-or-fixed-vector-mismatch
71 required-absence-or-ancestor-mismatch
72 concurrent-change-or-internal-runtime-failure
73 canonical-sequence-deviation
```

The exact copy vector is:

```text
/bin/busybox
cp
-a
--
/var/tmp/reap-g3-draft-v3
/var/tmp/reap-g3-draft-v8
argv_count=6
argv_nul_bytes=74
argv_nul_sha256=902a72f92276f193879fa128fed96ec905fcd6327789df03115b62fb3abca1e6
```

## One-child execution boundary

After successful copy and through valid `G3`, an external child may run only
in an envelope containing the exact storage preflight followed by exactly one
external child. No second child, pipeline peer, command substitution child,
or later command may reuse that preflight. Shell builtins may classify the one
child's status but may not launch another child.

Tracked or artifact edits use a separate exact preflight immediately before
the one edit operation. `apply_patch` is one edit operation. Staging and
committing each use their own preflight and one Git child. A reviewed script
may spawn children only when its own frozen implementation performs the exact
preflight immediately before each such child.

Any boundary violation stops before the next operation, even if read-only and
successful.

## Fresh v8 boundary

The new paths are:

```text
v8_root=/var/tmp/reap-g3-draft-v8
v8_patch=/var/tmp/reap-g3-draft-v8-provenance.patch
review_1_scratch=/var/tmp/reap-g3-draft-v8-review-1-scratch
review_2_scratch=/var/tmp/reap-g3-draft-v8-review-2-scratch
preview_root=target/tmp/goal-g-amendment-3-preview-v7
```

v3 is the sole source/control. v5, v6, and v7 remain immutable evidence.
Exactly these six v8 files may differ from v3:

```text
SELF-TEST-DESIGN.md
SELF-TEST-SCHEMA.md
construct-self-test.preview.sh
run-attempt.sh
run-phase0-replay.preview.sh
validators.sh
```

The other four remain byte-identical. Amendment 6's exact provenance edit
allowlist and all matcher, fixture, redirection-manifest, case, subcase, and
body-hash anchors remain controlling with `v4` read as `v8`, except for the
three closed bootstrap/preflight behavior hunks and eight-field review schema
closed below.

### Exact constructor-bootstrap repair

The first nonblank bytes after `#!/bin/busybox sh` freeze the bootstrap
environment and then define exactly one bootstrap preflight:

```bash
PATH=/usr/bin:/bin
LC_ALL=C
LANG=C
TZ=UTC
export PATH LC_ALL LANG TZ

reap_g3_bootstrap_storage_preflight() {
  (
    set -euo pipefail
    root=$(git rev-parse --show-toplevel)
    available_bytes=$(df --output=avail -B1 "$root" |
      awk 'NR == 2 {print $1}')
    case "$available_bytes" in
      ''|*[!0-9]*) exit 1 ;;
    esac
    [ "$available_bytes" -ge 2147483648 ]
  )
}
```

The `git`, `df`, and `awk` children inside that exact function are the only
bootstrap recursion exceptions. This BusyBox-compatible spelling has the same
repository-root, decimal-only, and 2-GiB predicates as the exact Bash
preflight. A preauthorization probe established that pinned BusyBox rejects
Bash `(( ... ))` arithmetic with exit `127`, while the frozen `case` plus
`[ ... -ge ... ]` spelling exits `0`; Bash-only syntax is therefore forbidden
in this bootstrap function. The function is not exported. It executes once
immediately before each of exactly these five target vectors:

```text
/bin/busybox stat -Lc %u:%h:%a:%s:%d:%i /bin/busybox
/bin/busybox stat -Lc %d:%i /proc/SELF/exe
/bin/busybox sha256sum /bin/busybox
/bin/busybox env -i PATH=/usr/bin:/bin LC_ALL=C LANG=C TZ=UTC /bin/bash --noprofile --norc -c ROOT_PREFLIGHT_PROGRAM
/bin/busybox env -i LC_ALL=C LANG=C TZ=UTC REAP_G3_CONSTRUCTOR_CLEAN_LAUNCH=1 REAP_G3_BOOTSTRAP_ROOT=ROOT /bin/bash --noprofile --norc SELF ARGS
```

For each of the first four command-substitution targets, the bootstrap
preflight and then `exec` of the target occur inside the command substitution.
For each of the first three identity probes, the parent shell classifies a
preflight exit `125` as `125` and every other nonzero target status as the
retained `124`. For the fourth root-bootstrap target, both preflight and
target failure retain the existing `125`; it is never mapped to `124`. For
the fifth target, the preflight is immediately followed by the existing top-
level `exec`; preflight failure remains `125` and impossible exec fallthrough
remains `126`. An `env` process and the Bash image that replaces it through
same-PID `execve` are one target vector, not two reusable child envelopes.

The nested root-discovery `git`, `df`, and `awk` remain the exact storage-
preflight recursion exceptions. The clean environment, BusyBox identity and
hash checks, root result, stdout, argument order, interpreter identity,
no-Cargo behavior, and `124`/`125`/`126` exit classification remain exact.
The later runtime `storage_preflight` definition and every external-command
wrapper remain byte-identical to v3. The bootstrap function occurs once; its
five calls occur exactly five times; and no other pre-runtime-definition
external target is present.

In `construct-self-test.preview.sh`, the executable edit surface is therefore
only the exact bootstrap hunk above plus Amendment 6's existing top-level
lineage/hash constants, `verify_preactivation_repository`, fixture 22a,
`EXPECTED_MANIFEST`, and lineage/status/evidence emission.

### Exact runner-bootstrap repair

`run-attempt.sh` receives the same fixed `PATH`, locale, timezone, export, and
BusyBox-compatible `reap_g3_bootstrap_storage_preflight` function shown above
before its first external target. Its v3 lines 2 through 157 are the only
runner behavior range this amendment supersedes:

```text
v3_prefix_bytes=5783
v3_prefix_sha256=4cd31fc4015b3260823c49d8c1c3a31c56ebdc6d95f70b034fe68cb09a7f1c66
```

The repair removes the old external env/Bash preflight helper. The new local
function is in-process except for its exact `git`, `df`, and `awk` recursion
exceptions. Exactly six calls guard exactly six non-exempt bootstrap target
vectors:

```text
1 /bin/busybox stat -Lc %u:%h:%a:%s:%d:%i /bin/busybox
2 /bin/busybox stat -Lc %d:%i /proc/SELF/exe
3 /bin/busybox sha256sum /bin/busybox
4 /bin/busybox env -i PATH=/usr/bin:/bin LC_ALL=C LANG=C TZ=UTC /bin/bash --noprofile --norc -c ROOT_PREFLIGHT_PROGRAM
5 /bin/busybox env -i PATH=/usr/bin:/bin LC_ALL=C LANG=C TZ=UTC /usr/bin/python3 -I -S -c PARENT_ENVIRONMENT_PROGRAM PARENT_PID PARENT_START
6 /bin/busybox env -i LC_ALL=C LANG=C TZ=UTC REAP_G3_CLEAN_LAUNCH=1 REAP_G3_BOOTSTRAP_ROOT=ROOT REAP_G3_REJECTED_ENV_NAMES=NAMES REAP_G3_REJECTED_FUNCTION_NAMES_HEX=HEX /bin/bash --noprofile --norc SELF ARGS
```

Targets 1 through 5 execute inside their respective command-substitution
shells. Each preflight is immediately followed by `exec` for targets 1
through 4. Target 5 deliberately does not use an exec-last-command shortcut:
the command-substitution shell must remain alive while Python authenticates
its `/proc` parent, captures the target status, and exits with that status.
The `/proc/self/stat` read between targets 4 and 5 is a shell redirection and
builtin read, not an external child, and receives no redundant preflight.
Target 6 is the final top-level same-PID env-to-Bash re-exec.

For identity targets 1 through 3, preflight failure is `125` and target
failure is `124`. For root target 4, either failure is the retained `125`.
For Python target 5, preflight failure is `125` and target, capture, parent,
or environment-validation failure is `126`. For target 6, preflight failure
is `125` and impossible exec fallthrough is `126`. Existing stdout, stderr,
argument order, clean environments, BusyBox/interpreter/hash checks, rejected
variable/function evidence, authenticated-parent behavior, and all later
Bash runtime bytes remain exact. The runtime `storage_preflight` and wrapper
families are not changed.

### Exact Phase 0 replay preflight repair

Only v3 lines 103 through 123 of `run-phase0-replay.preview.sh` are a behavior
edit surface:

```text
v3_range_bytes=820
v3_range_sha256=b01a62275c420792a7bb927262a99cdfa0c85ca166e4e8b04d70bb00bd08c782
```

The old `storage_preflight` routed its first query through
`capture_output env GIT_OPTIONAL_LOCKS=0 git -C ...`; that env-to-git vector is
not an allowed recursion exception. Replace the entire old
`storage_preflight` plus now-unused `storage_available_bytes` helper with the
exact Bash preflight:

```bash
storage_preflight() {
  (
    set -euo pipefail
    root=$(git rev-parse --show-toplevel)
    available_bytes=$(df --output=avail -B1 "$root" |
      awk 'NR == 2 {print $1}')
    [[ $available_bytes =~ ^[0-9]+$ ]]
    (( available_bytes >= 2147483648 ))
  )
}
```

Its direct `git`, `df`, and `awk` children are the exact recursion exceptions.
All guarded-child families, argument vectors, status handling, process
handshakes, stdout/stderr redirections, and every later Phase 0 byte remain
v3-identical except for Amendment 6's separately authorized provenance edits.

The five `/usr/bin/env`-to-Bash shebang transitions are caller-owned outer
invocation vectors; each same-PID interpreter chain consumes one caller
preflight and no in-script preflight. Runtime env-to-Bash, env-to-Python,
env-to-validator, env-to-setsid-to-Bash, and nested env-to-target chains are
also single same-PID target vectors guarded by their already frozen caller or
runtime wrapper. File-descriptor-only `exec` forms and heredoc fixture text
are not current-script external targets.

The v3 constructor lines 2 through 47 are independently frozen at 1538 bytes
with SHA-256
`b0e99d7b0299db7cb3bfa3a0ca34d1f7f9b08506e73a50b22d5d01a961bad98a`.
The v3-to-v7 constructor diff is exactly three hunks, 76 diff lines, 2484
bytes, and SHA-256
`d36620f444f5e1853a1666c2a6a37fb99719824f27c253b240617d86ba6cd4e8`.
Those bytes are reconstructed from v3; v7 is never a source.

In `run-attempt.sh`, `run-phase0-replay.preview.sh`, and `validators.sh`,
Amendment 6's existing provenance allowlist remains exact outside the two new
closed behavior ranges. Design and schema may describe only the authorized
provenance, three bootstrap-boundary, and no-scratch schema changes. Every
other byte in the six changed files remains byte-identical to v3.

The corrected matcher line plus LF remains exact SHA-256
`107cbbb11918f7bf6144f32a718ca10b6eabb328100721dc42dfbef0248393e1`
with one occurrence. `construct_combined_fixtures` remains 3025 bytes with
SHA-256 `7c1f62087f71572805426f0209c536e8c10310596292ac32e709974f05c8fa70`.
The shell-aware validator-redirection source manifest remains 179 seven-
column rows; after normalizing only source-line column 2 and preflight-line
column 6, it remains 17554 bytes with SHA-256
`b2734fc048d6e536cd2c4fdabe6975f5da77cee1b061a28e4eac97d4e51ef924`.
Only mechanically shifted decimal line numbers in those columns may differ.
Fixture cases remain 116 and high-cardinality subcases remain 1240.

Repository facts and `phase0.meta` retain existing `s4_*`, `t_*`, and `a5_*`
fields and add exactly these 55 fields:

```text
g5_stop_commit g5_stop_tree g5_stop_parent g5_stop_subject g5_stop_handoff_sha256
g6_auth_commit g6_auth_tree g6_auth_parent g6_auth_subject g6_auth_contract_sha256 g6_auth_handoff_sha256
g6_stop_commit g6_stop_tree g6_stop_parent g6_stop_subject g6_stop_handoff_sha256
g7_auth_commit g7_auth_tree g7_auth_parent g7_auth_subject g7_auth_contract_sha256 g7_auth_handoff_sha256
g7_stop_commit g7_stop_tree g7_stop_parent g7_stop_subject g7_stop_handoff_sha256
g8_auth_commit g8_auth_tree g8_auth_parent g8_auth_subject g8_auth_contract_sha256 g8_auth_handoff_sha256
g8_stop_commit g8_stop_tree g8_stop_parent g8_stop_subject g8_stop_handoff_sha256
g9_auth_commit g9_auth_tree g9_auth_parent g9_auth_subject g9_auth_contract_sha256 g9_auth_handoff_sha256
g9_stop_commit g9_stop_tree g9_stop_parent g9_stop_subject g9_stop_handoff_sha256
g10_auth_commit g10_auth_tree g10_auth_parent g10_auth_subject g10_auth_contract_sha256 g10_auth_handoff_sha256
```

`candidate_parent` is exact `G10_AUTH`. No `a6_*` synonym is authorized.
Runner, Phase 0 replay, validators, design, and schema are finalized before
constructor so no constructor self-hash is introduced. The patch is a
six-section Git full-index text patch directly from v3 to v8.

### Exact no-scratch static-review schema

Only for v8 static reviews, this amendment supersedes Amendment 6's eight-
field v4 review schema, its `removed-after-pass` value, and its create,
inventory, and remove lifecycle. For review number `N` in `1,2`, `G3`
contains exactly these eight fields:

```text
goal_g_amendment_10_v8_review_N_result
goal_g_amendment_10_v8_review_N_reviewer
goal_g_amendment_10_v8_review_N_session
goal_g_amendment_10_v8_review_N_implementation_sha256
goal_g_amendment_10_v8_review_N_scratch_state
goal_g_amendment_10_v8_review_N_v3_forensic_inventory_sha256
goal_g_amendment_10_v8_review_N_v8_forensic_inventory_sha256
goal_g_amendment_10_v8_review_N_patch_sha256
```

Both results are `pass`; reviewers, sessions, and implementation hashes are
nonempty and pairwise distinct; each implementation hash binds one exact,
separately reviewed child-free program; `scratch_state` is exactly
`absent-not-created`; both v3 hashes, both v8 hashes, and both patch hashes
agree with the frozen inputs. Any `scratch_final_inventory_sha256` field,
Amendment 6/8 synonym, missing, duplicate, malformed, or extra Amendment 10
v8-review field is forbidden. `verify_bundle`, activation validation,
design/schema provenance, and evidence emission consume exactly this schema.

Each independent reviewer runs as one freshly preflighted Python child whose
implementation launches no descendant. It proves its exact reserved scratch
path absent before and after the review, without creating that path. In
memory, it independently reproduces the complete direct v3-to-v8 patch,
allowed-edit proof, component manifest, forensic inventory, exact constructor
and runner bootstrap function/call/target counts, the exact Phase 0 preflight,
unchanged runtime wrappers, and every functional anchor. Any failure stops and
preserves all existing bytes.

## Preview, official construction, and activation

The retained no-Cargo bootstrap check remains mandatory before the single
preview invocation. The exact preview vector is:

```text
/bin/busybox
sh
/var/tmp/reap-g3-draft-v8/construct-self-test.preview.sh
preview
/home/ubuntu/code/reap/target/tmp/goal-g-amendment-3-preview-v7
argv_count=5
argv_nul_bytes=145
argv_nul_sha256=b4b54159748201b1274387c518abe99ba8fdc37e0b0c827e96c5bd2649001b2e
```

The v8 constructor reconstructs the exact A9-reviewed bootstrap directly from
v3 and retains the existing runtime per-child wrapper. The outer invocation
also has its own one-child envelope. Two distinct post-preview reviewers pass
before fresh official construction; two distinct official reviewers pass
before sealing. The official bundle, evidence, and runtime roots retain their
existing names.

The exact fresh official-construction vector is:

```text
/bin/busybox
sh
/var/tmp/reap-g3-draft-v8/construct-self-test.preview.sh
official-construct
argv_count=4
argv_nul_bytes=92
argv_nul_sha256=3b9ad97fbb4423566738befe0fa49f27bb6c67f4a92999f0563ae642d2287f8d
```

Official sealing retains Amendment 6's complete fourteen review-binding
arguments; its final complete argv digest is frozen and independently checked
before the one authorized sealing invocation.

If every gate passes, `G3` is the direct child of exact `G10_AUTH`, changes
only the handoff, and uses subject:

```text
docs: activate goal g amendment 3
```

Only these statuses change:

```text
goal_g_amendment_3_status: activation-stopped-inactive -> active-phase0
goal_g_amendment_10_status: authorized-inactive -> activation-complete-phase0-active
```

Amendments 5, 6, 7, 8, and 9 remain stopped. `P0` remains the direct child of
`G3`, changes only the handoff, and uses
`docs: qualify goal g amendment 3 phase 0`. Only after valid `G3` may Cargo
become available.

## Failure and safety

Any nonzero gate, copy, construction, patch, review, preview, official,
sealing, activation, or one-child-boundary violation stops. Preserve every
created byte. Do not retry or repair in place.

When storage permits, a stop commit is the direct child of `G10_AUTH`, changes
only the handoff, replaces the one Amendment 10 status with
`activation-stopped-inactive`, appends one terminal block, and uses subject:

```text
docs: record goal g amendment 10 activation stop
```

The exact 2-GiB preflight remains mandatory before every child, edit, write,
stage, and commit. Before valid `G3`, Cargo, rustc, rustdoc, rustfmt, tests,
benchmarks, public fetches, network children, credentials, authenticated
requests, Polygon RPC, and production order entry are prohibited.

```text
production_order_entry_authorized=false
real_credentials_loaded=false
authenticated_external_request_sent=false
real_polygon_rpc_request_sent=false
real_order_submitted=false
historical_goal_g_attempt_relabelled=false
historical_goal_g_r_equivalence_claimed=false
v5_mutation_or_promotion_authorized=false
v6_mutation_or_promotion_authorized=false
v7_mutation_or_promotion_authorized=false
push_authorized=false
```
