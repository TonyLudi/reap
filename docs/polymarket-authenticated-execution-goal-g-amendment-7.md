# Goal G Amendment 7: Closed Pre-Copy Recovery

Status: authorized for execution

Authorization date: 2026-08-02

Scope: one frozen current-state authentication, an indivisible v3-to-v5 copy,
provenance-only rebinding, and one fresh Goal G Amendment 3 activation lineage

## Purpose

Goal G Amendment 6 stopped honestly before v4 construction. Both required
forensic reviewers had independently passed, but the executor then added an
uncontracted component-row aggregate assertion to the immediate pre-copy
sequence. Its invented expected digest was wrong. The adjacent storage
preflight and copy were never reached, and every v4, review, preview, and
official path remained absent.

This amendment preserves that stop and removes the executor-controlled gap
that caused it. One exact, separately hashed `precopy-and-copy` launcher is
the complete new authentication authority. After its last content predicate
passes, it runs the retained Amendment 4 storage preflight and replaces itself
with the exact BusyBox copy process. Authentication success never returns to
the executor before copying begins.

For this recovery only, this amendment supersedes the conflicting successor,
status-transition, draft, review-root, preview-root, pre-copy authentication,
and activation-parent clauses in Amendments 3 through 6. Every other safety,
evidence, no-Cargo bootstrap, workload, sealing, and Phase 0 requirement
remains controlling.

## Immutable boundary and aliases

The new authorization is `G7_AUTH`. It must never be called `A7`; historical
Goal G-R already owns the `A6` family of aliases. The immutable parent is:

```text
G6_STOP_commit=f06e42623d9680dbe9c2012d6300a32ae17853c5
G6_STOP_tree=b44895964430bb25d0a6c2c0786cbfcf26c983ec
G6_STOP_parent=c20a95a3a45caa1cab66f878267469bff59481bf
G6_STOP_subject=docs: record goal g amendment 6 activation stop
G6_STOP_delta_path_count=1
G6_STOP_delta_paths=docs/polymarket-authenticated-execution-goal-g-handoff.md
G6_STOP_handoff_sha256=3b15c0cdf89bf017d681e5bc89cf581d50431db186b247f77a656dbf57102589
```

The complete successor chain is:

```text
R6 -> S4 -> T -> A5 -> G5_STOP -> G6_AUTH -> G6_STOP -> G7_AUTH -> G3 -> P0
```

No commit may be amended, rebased, reset, replaced, skipped, or assigned a
second alias.

## Preserved evidence and current-state gate

The Amendment 3, 5, and 6 terminal blocks are immutable historical evidence.
Before `G3`, their current statuses remain exactly:

```text
goal_g_amendment_3_status=activation-stopped-inactive
goal_g_amendment_5_status=activation-stopped-inactive
goal_g_amendment_6_status=activation-stopped-inactive
```

The two successful Amendment 6 forensic reviews remain immutable Amendment 6
historical evidence. Amendment 7 recognizes them as satisfied independent-
review prerequisites only for the exact v2/v3 identities they recorded. It
does not rerun, rename, relabel, or count them as Amendment 7 reviews, and it
does not claim Amendment 6 completed successfully.

After exact clean `G7_AUTH`, one new canonical Amendment 7 authentication
recomputes the complete closed retained-artifact identity set from current
bytes and binds exact `G6_STOP` and structurally exact `G7_AUTH`. This is
current-state reauthentication, not a retry of either Amendment 6 review.
Construction is authorized only when the historical review records and the
new canonical result agree exactly.

Exclusive mutation ownership is required from the `G7_AUTH` commit through
completion of the v3-to-v5 copy. Another session must not edit, stage, commit,
chmod, rename, link, remove, create, or mount over any repository, retained,
successor, preview, or official path in that interval.

## G7_AUTH commit contract

`G7_AUTH` is the direct child of exact `G6_STOP`, changes only:

- `docs/polymarket-authenticated-execution-goal-g-amendment-7.md`; and
- `docs/polymarket-authenticated-execution-goal-g-handoff.md`.

Its exact subject is:

```text
docs: authorize goal g amendment 7 closed pre-copy recovery
```

The handoff appends exactly one
`goal_g_amendment_7_status=authorized-inactive` field. It does not change any
earlier status or terminal byte. The pre-commit contract and handoff do not
contain the future `G7_AUTH` commit, tree, or handoff hash; those are captured
only after the commit and bound in v5 and `G3`.

Before committing, two distinct read-only reviewers in distinct sessions
must independently review the exact bootstrap and launcher source bytes,
their extraction and invocation schema, the closed assertion surface, child
boundary, exit classes, final preflight, exact copy argv, and this contract's
lineage and status rules. A review failure stops before authorization.

The handoff binds exactly six fields for each review number `N` in `1,2`:

```text
goal_g_amendment_7_source_review_N_result
goal_g_amendment_7_source_review_N_reviewer
goal_g_amendment_7_source_review_N_session
goal_g_amendment_7_source_review_N_contract_sha256
goal_g_amendment_7_source_review_N_bootstrap_sha256
goal_g_amendment_7_source_review_N_precopy_launcher_sha256
```

Both results must be `pass`; reviewer and session identities must be distinct;
and all three source identities must equal the exact committed identities.
The launcher authenticates this closed pair before recognizing the historical
Amendment 6 forensic evidence.

This amendment does not authorize a push.

## Frozen bootstrap

The canonical outer invocation first runs the exact Amendment 4 storage
preflight and then invokes `/usr/bin/python3 -I -S -c` with the following
exact UTF-8 bootstrap bytes both as the `-c` program and as its first argument.
The remaining arguments are the absolute contract and handoff paths in that
order. No other argument, environment-derived path, redirect, pipe, `tee`,
command substitution around the complete invocation, or report is allowed.

<!-- GOAL-G-A7-BOOTSTRAP-SOURCE-BEGIN -->
```python
import hashlib, os, sys

def die(message, code=65):
    try:
        os.write(2, ("goal-g-a7-bootstrap:" + message + "\n").encode("ascii"))
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
    if contract_path != "/home/ubuntu/code/reap/docs/polymarket-authenticated-execution-goal-g-amendment-7.md":
        die("contract-path", 64)
    if handoff_path != "/home/ubuntu/code/reap/docs/polymarket-authenticated-execution-goal-g-handoff.md":
        die("handoff-path", 64)
    contract = read_regular(contract_path)
    handoff = read_regular(handoff_path)
    if len(self_bytes) != decimal_field(handoff, "goal_g_amendment_7_bootstrap_bytes"):
        die("bootstrap-bytes")
    if hashlib.sha256(self_bytes).hexdigest() != field(handoff, "goal_g_amendment_7_bootstrap_sha256"):
        die("bootstrap-sha256")
    start = b"<!-- GOAL-G-A7-PRECOPY-AND-COPY-SOURCE-BEGIN -->\n```bash\n"
    end = b"\n```\n<!-- GOAL-G-A7-PRECOPY-AND-COPY-SOURCE-END -->"
    if contract.count(start) != 1 or contract.count(end) != 1:
        die("launcher-markers")
    source = contract.split(start, 1)[1].split(end, 1)[0]
    if b"\x00" in source:
        die("launcher-nul")
    if len(source) != decimal_field(handoff, "goal_g_amendment_7_precopy_launcher_bytes"):
        die("launcher-bytes")
    if hashlib.sha256(source).hexdigest() != field(handoff, "goal_g_amendment_7_precopy_launcher_sha256"):
        die("launcher-sha256")
    try:
        decoded = source.decode("utf-8")
    except UnicodeDecodeError:
        die("launcher-utf8")
    os.execve(
        "/bin/bash",
        ["/bin/bash", "--noprofile", "--norc", "-c", decoded,
         "goal-g-a7-precopy-and-copy"],
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
<!-- GOAL-G-A7-BOOTSTRAP-SOURCE-END -->

The frozen bootstrap identities are:

```text
bootstrap_bytes=3623
bootstrap_sha256=7fb4bb36a4a5a666c60d89c62037184b853daf077a7c0a84971163b88166d633
```

## Closed pre-copy-and-copy launcher

The launcher below is the complete authentication authority. It executes only
its literal ordered assertion IDs. No caller, executor, reviewer, wrapper,
shell option, helper, or diagnostic may add, remove, repeat, reorder,
reinterpret, or aggregate a predicate. Any unlisted predicate, digest, child,
transformation, assertion, or diagnostic is terminal
`canonical-sequence-deviation`, even if it would pass.

The only permitted digest classes are the frozen Git/document, bootstrap,
launcher, retained file-content, component-manifest, forensic-inventory,
fixed-vector, BusyBox, and argv digests named in this contract. Computing,
comparing, or emitting either aggregate recorded as the Amendment 6 failure,
or any new row-set, assertion-output, status, absence, transcript, or
component-row aggregate, is prohibited.

The prohibited Amendment 6 failure aggregates are exactly:

```text
48bd3e112607431d7b442103921d94c0e65ca0812a15a51d593e7e8f28e34200
be8ecd49614fd0a14d3f30f05e7380e55f077c1c912175daa70451ecd3301abc
```

Every external child is preceded by the exact retained storage preflight. The
preflight's own `git`, `df`, and `awk` children are its only recursion
exception. The verifier is child-free. Its stdout and stderr remain attached
directly to the controlling session. A nonzero result is terminal. A copy
failure preserves partial v5 exactly and is terminal.

<!-- GOAL-G-A7-PRECOPY-AND-COPY-SOURCE-BEGIN -->
```bash
set -Eeuo pipefail

readonly REPO=/home/ubuntu/code/reap
readonly CONTRACT=$REPO/docs/polymarket-authenticated-execution-goal-g-amendment-7.md
readonly HANDOFF=$REPO/docs/polymarket-authenticated-execution-goal-g-handoff.md
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
  printf 'goal-g-a7-precopy:failure:%s:%s\n' "$code" "$assertion" >&2 || :
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

capture_child() {
  local destination=$1
  local assertion=$2
  shift 2
  storage_preflight || fail 66 "$assertion-storage-preflight"
  local output status
  set +e
  output=$("$@" 2>&1)
  status=$?
  set -e
  (( status == 0 )) || fail 66 "$assertion-child"
  printf -v "$destination" '%s' "$output"
}

readonly -a SHELL_ASSERTION_IDS=(
  invocation-schema
  fixed-environment
  repository-root
  repository-branch
  repository-clean
  g7-commit
  g7-tree
  g7-parent
  g7-subject
  g7-two-path-delta
  g6-stop-object
  g6-auth-object
  first-parent-lineage
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
[[ $# -eq 0 ]] || fail 64 invocation-schema
[[ $0 == goal-g-a7-precopy-and-copy ]] || fail 64 invocation-name

begin_shell_assertion fixed-environment
[[ ${PATH-} == /usr/bin:/bin ]] || fail 64 environment-path
[[ ${LC_ALL-} == C && ${LANG-} == C && ${TZ-} == UTC ]] || fail 64 environment-locale
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

begin_shell_assertion g7-commit
capture_child g7_commit g7-commit /usr/bin/git rev-parse HEAD
[[ $g7_commit =~ ^[0-9a-f]{40}$ ]] || fail 67 g7-commit

begin_shell_assertion g7-tree
capture_child g7_tree g7-tree /usr/bin/git rev-parse 'HEAD^{tree}'
[[ $g7_tree =~ ^[0-9a-f]{40}$ ]] || fail 67 g7-tree

begin_shell_assertion g7-parent
capture_child g7_parent g7-parent /usr/bin/git rev-parse 'HEAD^'
[[ $g7_parent == "$G6_STOP" ]] || fail 67 g7-parent

begin_shell_assertion g7-subject
capture_child g7_subject g7-subject /usr/bin/git show -s --format=%s HEAD
[[ $g7_subject == 'docs: authorize goal g amendment 7 closed pre-copy recovery' ]] || fail 67 g7-subject

begin_shell_assertion g7-two-path-delta
capture_child g7_delta g7-two-path-delta /usr/bin/git diff-tree --no-commit-id --name-only -r HEAD
[[ $g7_delta == $'docs/polymarket-authenticated-execution-goal-g-amendment-7.md\ndocs/polymarket-authenticated-execution-goal-g-handoff.md' ]] || fail 67 g7-two-path-delta

begin_shell_assertion g6-stop-object
capture_child g6_stop_object g6-stop-object /usr/bin/git show -s --format='%H%x09%T%x09%P%x09%s' "$G6_STOP"
[[ $g6_stop_object == $'f06e42623d9680dbe9c2012d6300a32ae17853c5\tb44895964430bb25d0a6c2c0786cbfcf26c983ec\tc20a95a3a45caa1cab66f878267469bff59481bf\tdocs: record goal g amendment 6 activation stop' ]] || fail 67 g6-stop-object

begin_shell_assertion g6-auth-object
capture_child g6_auth_object g6-auth-object /usr/bin/git show -s --format='%H%x09%T%x09%P%x09%s' "$G6_AUTH"
[[ $g6_auth_object == $'c20a95a3a45caa1cab66f878267469bff59481bf\t9b19215d6560858adb3fb0427fe92a6e3e928d92\tdab6a252ffe25bb390da12a0459125cbeeacb7de\tdocs: authorize goal g amendment 6 forensic inventory recovery' ]] || fail 67 g6-auth-object

begin_shell_assertion first-parent-lineage
capture_child first_parent_lineage first-parent-lineage /usr/bin/git rev-list --first-parent --max-count=8 HEAD
[[ $first_parent_lineage == "$g7_commit"$'\n'"$G6_STOP"$'\n'"$G6_AUTH"$'\n'"$G5_STOP"$'\n'"$A5"$'\n'"$T"$'\n'"$S4"$'\n'"$R6" ]] || fail 67 first-parent-lineage

begin_shell_assertion shell-assertion-sequence-complete
(( shell_assertion_index == ${#SHELL_ASSERTION_IDS[@]} )) || fail 73 assertion-underrun

export GOAL_G_A7_REPO=$REPO
export GOAL_G_A7_CONTRACT=$CONTRACT
export GOAL_G_A7_HANDOFF=$HANDOFF
export GOAL_G_A7_G7_COMMIT=$g7_commit
export GOAL_G_A7_G7_TREE=$g7_tree
export GOAL_G_A7_G7_PARENT=$g7_parent
export GOAL_G_A7_G7_SUBJECT=$g7_subject

storage_preflight || fail 66 verifier-storage-preflight
set +e
/usr/bin/python3 -I -S <<'PY'
import hashlib
import os
import stat
import sys

EXIT_REPOSITORY = 67
EXIT_ENTRY = 68
EXIT_COMPONENT = 69
EXIT_FORENSIC = 70
EXIT_ABSENCE = 71
EXIT_CONCURRENT = 72
EXIT_SEQUENCE = 73

ASSERTION_IDS = (
    "handoff-schema-and-status",
    "contract-and-authorization-cross-binding",
    "preauthorization-source-review-evidence",
    "historical-a6-review-evidence",
    "fixed-forensic-vectors",
    "retained-v2-tree",
    "retained-a5-control-tree",
    "retained-a5-patch",
    "retained-failed-preview",
    "closed-required-absence-set",
    "busybox-and-argv-vectors",
    "final-v3-copy-source-tree",
)
assertion_index = 0

def abort(code, assertion, detail):
    message = "goal-g-a7-precopy:failure:%d:%s:%s\n" % (code, assertion, detail)
    try:
        os.write(2, message.encode("ascii", "backslashreplace"))
    except Exception:
        pass
    raise SystemExit(code)

def begin(assertion):
    global assertion_index
    if assertion_index >= len(ASSERTION_IDS):
        abort(EXIT_SEQUENCE, assertion, "assertion-overrun")
    if ASSERTION_IDS[assertion_index] != assertion:
        abort(EXIT_SEQUENCE, assertion, "assertion-reorder")
    assertion_index += 1

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
            abort(EXIT_ENTRY, assertion, "not-regular")
        fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
        opened = os.fstat(fd)
        digest = hashlib.sha256()
        chunks = []
        while True:
            chunk = os.read(fd, 1048576)
            if not chunk:
                break
            digest.update(chunk)
            chunks.append(chunk)
        after_fd = os.fstat(fd)
        os.close(fd)
        after = os.lstat(path)
    except SystemExit:
        raise
    except Exception:
        abort(EXIT_CONCURRENT, assertion, "stable-read-runtime")
    if metadata(before) != metadata(opened) or metadata(opened) != metadata(after_fd) or metadata(after_fd) != metadata(after):
        abort(EXIT_CONCURRENT, assertion, "stable-read-cut")
    return b"".join(chunks), digest.hexdigest(), before

def sha256_bytes(value):
    return hashlib.sha256(value).hexdigest()

def handoff_fields(data):
    result = {}
    for line in data.splitlines():
        if b"=" not in line:
            continue
        key, value = line.split(b"=", 1)
        try:
            name = key.decode("ascii")
            decoded = value.decode("ascii")
        except UnicodeDecodeError:
            continue
        result.setdefault(name, []).append(decoded)
    return result

def unique(fields, name, assertion):
    values = fields.get(name, [])
    if len(values) != 1:
        abort(EXIT_REPOSITORY, assertion, "field-" + name)
    return values[0]

def expected_files(version):
    unchanged = {
        "commands.tsv": ("89d0e03b192d03ba34d8680616f0c5484010cb06ec3cc59813b66a8c4b0abb7f", 5509, 0o664),
        "inventory.preview.sh": ("d102c9ddc68cf0eb7fad72308bd86fa986dca52e2dbc0c8346e98a11fe9cf84c", 53408, 0o664),
        "run-phase0-replay.preview.sh": ("f4b7a52322a0568b19b1e515cb3ec998e827ccbd0ac25abcce0ddd11eddbb2a7", 100443, 0o664),
        "source-reattest.preview.sh": ("ff1a11823e39b73682c0b77a614f356c17a17907b29855e7d2c7dbeca9bfbd76", 22544, 0o664),
        "summarize-baseline.preview.sh": ("8c4a006f1eea1c077322bb2baaec195fc2cc8bac52d4ca7fe3d03b6772799f2d", 82593, 0o664),
    }
    if version == "v2":
        changed = {
            "SELF-TEST-DESIGN.md": ("83ed16b84d8d2f9ef2865eecca2d8fc431636da776c32a800e575ebf2fb20c7d", 43316, 0o664),
            "SELF-TEST-SCHEMA.md": ("5e5d90b7b568e53e5f3366717108071ff7b3473bdab567beb16ebdf02845d5f0", 22821, 0o664),
            "construct-self-test.preview.sh": ("2fe07168369ca726f17328b3d9142522ab2540d057b5d95dd9586a6ded952ee6", 362479, 0o664),
            "run-attempt.sh": ("fc5253b789f7ada0e7ba4e016d4ce59551ac03235376c4a9d5e2b3246df93411", 211644, 0o664),
            "validators.sh": ("4d254d326676ef685d36cb666f8475e3e15d0cb24c4c7ac24c55525e54e0c121", 133650, 0o700),
        }
    else:
        constructor_hash = "22677a1ebcdb6fa9bb59b885db6ef0133d62f9d28056e4bc4b632cfa4fde73db" if version == "control" else "7f16928835d296353d6cc94501bd3cabd6f7febc7da044606673d7ee287c9bba"
        constructor_bytes = 366813 if version == "control" else 366812
        changed = {
            "SELF-TEST-DESIGN.md": ("4f739c6f49d90418ba1e1576bf2f4015f1da9a4b9b8eed9ffa3de9414d21c5a4", 44806, 0o664),
            "SELF-TEST-SCHEMA.md": ("a4d8e7ae085bd2517678e0762690c813d2e69232d463e3df83ec9956faf27ecd", 24089, 0o664),
            "construct-self-test.preview.sh": (constructor_hash, constructor_bytes, 0o664),
            "run-attempt.sh": ("86a79706b6aa8253b7d8fb298c5016535aab33a2cd91f4c842b3c2d06c72ddcd", 217156, 0o664),
            "validators.sh": ("897f3bb05418397d8d17944dea70501a1bb2adbbf65c73acc06035726eab678b", 138365, 0o700),
        }
    changed.update(unchanged)
    return changed

TREE_EXPECTATIONS = {
    "v2": {
        "path": "/var/tmp/reap-g3-draft-v2", "root": (66305, 305347, 0o700, 1000, 1000, 2, 4096),
        "files": expected_files("v2"), "regular_bytes": 1038407,
        "component": "82fa2de7bc468a5a60fa3f795f336d621515557a5ee21b9828b09d1d526cf4a8",
        "forensic": "062c306df0e3a5b331be79df841dc98eefeed1a9d1a5b899968bae662d59f0cb",
    },
    "control": {
        "path": "/var/tmp/reap-g3-draft-v3-provenance-control", "root": (66305, 310092, 0o700, 1000, 1000, 2, 4096),
        "files": expected_files("control"), "regular_bytes": 1055726,
        "component": "50f7de09cb5f19de4a9f1375a4a4a5a1acf40b4f831e65004a0651664df3db61",
        "forensic": "2f05254afe092859bcae96711f993cfd88165820896b0287441f2251206b9d51",
    },
    "v3": {
        "path": "/var/tmp/reap-g3-draft-v3", "root": (66305, 310585, 0o700, 1000, 1000, 2, 4096),
        "files": expected_files("v3"), "regular_bytes": 1055725,
        "component": "710ab62d5dbe846b21df74a4d78ee3f12d2a1883a22662d256bf751d411bc451",
        "forensic": "9ab5b0695e60cf47fc41e9d2e110b291c56b856d80cbfad36b9d41b6a5c7d233",
    },
}

def direct_tree(name, assertion):
    expected = TREE_EXPECTATIONS[name]
    root = expected["path"]
    try:
        root_before = os.lstat(root)
        names = os.listdir(os.fsencode(root))
    except Exception:
        abort(EXIT_ENTRY, assertion, "root-read")
    if not stat.S_ISDIR(root_before.st_mode):
        abort(EXIT_ENTRY, assertion, "root-not-directory")
    root_actual = (root_before.st_dev, root_before.st_ino, root_before.st_mode & 0o7777,
                   root_before.st_uid, root_before.st_gid, root_before.st_nlink,
                   root_before.st_size)
    equal(root_actual, expected["root"], EXIT_ENTRY, assertion, "root-metadata")
    expected_names = sorted(item.encode("ascii") for item in expected["files"])
    equal(sorted(names), expected_names, EXIT_ENTRY, assertion, "child-set")
    component = bytearray()
    forensic_records = []
    file_snapshots = {}
    total = 0
    root_record = (b".\0d\0%04o\0%d\0%d\0%d\0%d\0-\n" %
                   (root_before.st_mode & 0o7777, root_before.st_uid,
                    root_before.st_gid, root_before.st_nlink, root_before.st_size))
    forensic_records.append((b".", root_record))
    for raw_name in expected_names:
        text_name = raw_name.decode("ascii")
        data, digest, entry = stable_file(os.path.join(os.fsencode(root), raw_name), assertion)
        wanted_hash, wanted_size, wanted_mode = expected["files"][text_name]
        equal(digest, wanted_hash, EXIT_ENTRY, assertion, "file-hash-" + text_name)
        actual = (entry.st_mode & 0o7777, entry.st_uid, entry.st_gid,
                  entry.st_nlink, entry.st_size)
        equal(actual, (wanted_mode, 1000, 1000, 1, wanted_size),
              EXIT_ENTRY, assertion, "file-metadata-" + text_name)
        total += len(data)
        component.extend((digest + "\t" + str(len(data)) + "\t" + text_name + "\n").encode("ascii"))
        record = (raw_name + b"\0f\0" + ("%04o" % (entry.st_mode & 0o7777)).encode("ascii") +
                  b"\0" + str(entry.st_uid).encode("ascii") + b"\0" +
                  str(entry.st_gid).encode("ascii") + b"\0" +
                  str(entry.st_nlink).encode("ascii") + b"\0" +
                  str(entry.st_size).encode("ascii") + b"\0" +
                  digest.encode("ascii") + b"\n")
        forensic_records.append((raw_name, record))
        file_snapshots[raw_name] = metadata(entry)
    root_after = os.lstat(root)
    equal(metadata(root_after), metadata(root_before), EXIT_CONCURRENT, assertion, "root-stability")
    for raw_name in expected_names:
        equal(metadata(os.lstat(os.path.join(os.fsencode(root), raw_name))), file_snapshots[raw_name],
              EXIT_CONCURRENT, assertion, "file-stability")
    forensic = b"".join(record for _, record in sorted(forensic_records))
    equal(total, expected["regular_bytes"], EXIT_ENTRY, assertion, "regular-bytes")
    equal((len(component.splitlines()), len(component)), (10, 933), EXIT_COMPONENT, assertion, "component-shape")
    equal(sha256_bytes(component), expected["component"], EXIT_COMPONENT, assertion, "component-sha256")
    equal((len(forensic_records), len(forensic)), (11, 1151), EXIT_FORENSIC, assertion, "forensic-shape")
    equal(sha256_bytes(forensic), expected["forensic"], EXIT_FORENSIC, assertion, "forensic-sha256")
    return root_record, forensic_records

def recursive_preview(assertion):
    root = b"/home/ubuntu/code/reap/target/tmp/goal-g-amendment-3-preview-v1"
    try:
        root_before = os.lstat(root)
    except Exception:
        abort(EXIT_ENTRY, assertion, "preview-root")
    if not stat.S_ISDIR(root_before.st_mode):
        abort(EXIT_ENTRY, assertion, "preview-root-not-directory")
    equal((root_before.st_dev, root_before.st_ino, root_before.st_mode & 0o7777,
           root_before.st_uid, root_before.st_gid),
          (66305, 808763, 0o700, 1000, 1000), EXIT_ENTRY, assertion,
          "preview-root-metadata")
    rels = [b"."]
    def walk(base, relative):
        try:
            children = sorted(os.listdir(base))
        except Exception:
            abort(EXIT_ENTRY, assertion, "preview-list")
        for child in children:
            path = os.path.join(base, child)
            rel = child if not relative else relative + b"/" + child
            entry = os.lstat(path)
            if stat.S_ISLNK(entry.st_mode):
                abort(EXIT_ENTRY, assertion, "preview-link")
            if not stat.S_ISDIR(entry.st_mode) and not stat.S_ISREG(entry.st_mode):
                abort(EXIT_ENTRY, assertion, "preview-type")
            rels.append(rel)
            if stat.S_ISDIR(entry.st_mode):
                walk(path, rel)
    walk(root, b"")
    inventory = bytearray()
    manifest = bytearray()
    snapshots = {}
    dirs = files = regular_bytes = 0
    dir_modes = []
    file_modes = []
    for rel in sorted(rels):
        path = root if rel == b"." else os.path.join(root, *rel.split(b"/"))
        entry = os.lstat(path)
        snapshots[rel] = metadata(entry)
        if stat.S_ISDIR(entry.st_mode):
            kind = b"d"
            payload = b"-"
            dirs += 1
            dir_modes.append(entry.st_mode & 0o7777)
        elif stat.S_ISREG(entry.st_mode):
            kind = b"f"
            data, digest, entry = stable_file(path, assertion)
            snapshots[rel] = metadata(entry)
            payload = digest.encode("ascii")
            files += 1
            regular_bytes += len(data)
            file_modes.append(entry.st_mode & 0o7777)
            manifest.extend(payload + b"  " + rel + b"\n")
        else:
            abort(EXIT_ENTRY, assertion, "preview-type-late")
        inventory.extend(rel + b"\0" + kind + b"\0" +
                         ("%04o" % (entry.st_mode & 0o7777)).encode("ascii") + b"\0" +
                         str(entry.st_uid).encode("ascii") + b"\0" +
                         str(entry.st_gid).encode("ascii") + b"\0" +
                         str(entry.st_nlink).encode("ascii") + b"\0" +
                         str(entry.st_size).encode("ascii") + b"\0" + payload + b"\n")
    for rel in sorted(rels):
        path = root if rel == b"." else os.path.join(root, *rel.split(b"/"))
        equal(metadata(os.lstat(path)), snapshots[rel], EXIT_CONCURRENT, assertion,
              "preview-stability")
    equal((len(rels), dirs, files, regular_bytes), (21, 13, 8, 615138),
          EXIT_ENTRY, assertion, "preview-counts")
    equal((dir_modes.count(0o700), file_modes.count(0o700), file_modes.count(0o600)),
          (13, 6, 2), EXIT_ENTRY, assertion, "preview-modes")
    equal(sha256_bytes(inventory),
          "82ac222e4932320ad14ce7ef7800bd8e39a373deaf6ce8205a9ab9ccbfd11747",
          EXIT_FORENSIC, assertion, "preview-forensic")
    equal(sha256_bytes(manifest),
          "a86c192658af2e4edef79c70ae4f89e842ac9f57ba278f1b8c0ff835defe2df9",
          EXIT_COMPONENT, assertion, "preview-manifest")
    report = os.path.join(root, b"self-test/fixtures/reports/10-combined-valid.log")
    report_data, report_hash, _ = stable_file(report, assertion)
    equal((len(report_data), report_data.count(b"\n"), report_hash),
          (5347, 27, "bbea695789a6c13ef3095f55622c0c9cf9108a1965f5010485f01628369a3d67"),
          EXIT_ENTRY, assertion, "preview-report")

def ensure_absent(path, assertion):
    if not path.startswith("/") or os.path.normpath(path) != path:
        abort(EXIT_ABSENCE, assertion, "noncanonical-path")
    current = "/"
    parts = [part for part in path.split("/") if part]
    for index, part in enumerate(parts):
        current = os.path.join(current, part)
        try:
            entry = os.lstat(current)
        except FileNotFoundError:
            return
        except Exception:
            abort(EXIT_ABSENCE, assertion, "absence-runtime")
        if stat.S_ISLNK(entry.st_mode):
            abort(EXIT_ABSENCE, assertion, "linked-ancestor")
        if index == len(parts) - 1:
            abort(EXIT_ABSENCE, assertion, "path-present")
        if not stat.S_ISDIR(entry.st_mode):
            abort(EXIT_ABSENCE, assertion, "non-directory-ancestor")
    abort(EXIT_ABSENCE, assertion, "path-present")

try:
    repo = os.environ["GOAL_G_A7_REPO"]
    contract_path = os.environ["GOAL_G_A7_CONTRACT"]
    handoff_path = os.environ["GOAL_G_A7_HANDOFF"]
    g7_commit = os.environ["GOAL_G_A7_G7_COMMIT"]
    g7_tree = os.environ["GOAL_G_A7_G7_TREE"]
    g7_parent = os.environ["GOAL_G_A7_G7_PARENT"]
    g7_subject = os.environ["GOAL_G_A7_G7_SUBJECT"]

    begin("handoff-schema-and-status")
    contract_data, contract_hash, _ = stable_file(contract_path, "handoff-schema-and-status")
    handoff_data, handoff_hash, _ = stable_file(handoff_path, "handoff-schema-and-status")
    fields = handoff_fields(handoff_data)
    for name, value in (
        ("goal_g_amendment_3_status", "activation-stopped-inactive"),
        ("goal_g_amendment_5_status", "activation-stopped-inactive"),
        ("goal_g_amendment_6_status", "activation-stopped-inactive"),
        ("goal_g_amendment_7_status", "authorized-inactive"),
    ):
        equal(unique(fields, name, "handoff-schema-and-status"), value,
              EXIT_REPOSITORY, "handoff-schema-and-status", name)

    begin("contract-and-authorization-cross-binding")
    expected_cross = {
        "goal_g_amendment_7_schema": "goal-g-amendment-7-v1",
        "goal_g_amendment_7_parent_commit": "f06e42623d9680dbe9c2012d6300a32ae17853c5",
        "goal_g_amendment_7_parent_tree": "b44895964430bb25d0a6c2c0786cbfcf26c983ec",
        "goal_g_amendment_7_parent_parent": "c20a95a3a45caa1cab66f878267469bff59481bf",
        "goal_g_amendment_7_parent_subject": "docs: record goal g amendment 6 activation stop",
        "goal_g_amendment_7_parent_handoff_sha256": "3b15c0cdf89bf017d681e5bc89cf581d50431db186b247f77a656dbf57102589",
        "goal_g_amendment_7_contract_path": "docs/polymarket-authenticated-execution-goal-g-amendment-7.md",
        "goal_g_amendment_7_contract_sha256": contract_hash,
        "goal_g_amendment_7_authorization_alias": "G7_AUTH",
        "goal_g_amendment_7_authorization_subject": "docs: authorize goal g amendment 7 closed pre-copy recovery",
        "goal_g_amendment_7_authorization_path_count": "2",
        "goal_g_amendment_7_authorization_paths": "docs/polymarket-authenticated-execution-goal-g-amendment-7.md,docs/polymarket-authenticated-execution-goal-g-handoff.md",
        "goal_g_amendment_7_lineage": "R6->S4->T->A5->G5_STOP->G6_AUTH->G6_STOP->G7_AUTH->G3->P0",
    }
    for name, value in expected_cross.items():
        equal(unique(fields, name, "contract-and-authorization-cross-binding"), value,
              EXIT_REPOSITORY, "contract-and-authorization-cross-binding", name)
    equal((g7_parent, g7_subject),
          ("f06e42623d9680dbe9c2012d6300a32ae17853c5",
           "docs: authorize goal g amendment 7 closed pre-copy recovery"),
          EXIT_REPOSITORY, "contract-and-authorization-cross-binding", "g7-runtime")

    begin("preauthorization-source-review-evidence")
    source_review_identities = (
        ("1", "g5-contract-design-review-1", "g5-contract-design-a7-20260802"),
        ("2", "g5-repo-boundary-review-2", "g5-repo-boundary-a7-20260802"),
    )
    bootstrap_hash = unique(fields, "goal_g_amendment_7_bootstrap_sha256",
                            "preauthorization-source-review-evidence")
    launcher_hash = unique(fields, "goal_g_amendment_7_precopy_launcher_sha256",
                           "preauthorization-source-review-evidence")
    for number, reviewer, session in source_review_identities:
        prefix = "goal_g_amendment_7_source_review_" + number + "_"
        expected_review = {
            prefix + "result": "pass",
            prefix + "reviewer": reviewer,
            prefix + "session": session,
            prefix + "contract_sha256": contract_hash,
            prefix + "bootstrap_sha256": bootstrap_hash,
            prefix + "precopy_launcher_sha256": launcher_hash,
        }
        for name, value in expected_review.items():
            equal(unique(fields, name, "preauthorization-source-review-evidence"), value,
                  EXIT_REPOSITORY, "preauthorization-source-review-evidence", name)

    begin("historical-a6-review-evidence")
    review_values = {
        "goal_g_amendment_6_activation_stop_forensic_review_1_result": "pass",
        "goal_g_amendment_6_activation_stop_forensic_review_1_reviewer": "root-g6-forensic-review-1",
        "goal_g_amendment_6_activation_stop_forensic_review_1_session": "root-g6-forensic-review-1-20260802",
        "goal_g_amendment_6_activation_stop_forensic_review_1_implementation_sha256": "d63920d4332a8b305cb1b5d893a48e1933608a615ae397a72e7b3bc1befb4331",
        "goal_g_amendment_6_activation_stop_forensic_review_2_result": "pass",
        "goal_g_amendment_6_activation_stop_forensic_review_2_reviewer": "root-g6-forensic-review-2",
        "goal_g_amendment_6_activation_stop_forensic_review_2_session": "root-g6-forensic-review-2-20260802",
        "goal_g_amendment_6_activation_stop_forensic_review_2_implementation_sha256": "e4b884cbf4d85be2b36593092bae5dc35c50f1192f30ddba78c5c4e9b39f2fe2",
    }
    for number in ("1", "2"):
        review_values["goal_g_amendment_6_activation_stop_forensic_review_" + number + "_v2_inventory_sha256"] = "062c306df0e3a5b331be79df841dc98eefeed1a9d1a5b899968bae662d59f0cb"
        review_values["goal_g_amendment_6_activation_stop_forensic_review_" + number + "_v3_inventory_sha256"] = "9ab5b0695e60cf47fc41e9d2e110b291c56b856d80cbfad36b9d41b6a5c7d233"
        review_values["goal_g_amendment_6_activation_stop_forensic_review_" + number + "_two_record_vector_sha256"] = "63ed0e2d6f3f43abc06cce1dd215d166131f25132b645ec6c027b50d1629c9c0"
    for name, value in review_values.items():
        equal(unique(fields, name, "historical-a6-review-evidence"), value,
              EXIT_REPOSITORY, "historical-a6-review-evidence", name)

    begin("fixed-forensic-vectors")
    vector = (b".\x00d\x000700\x001000\x001000\x002\x004096\x00-\n"
              b"a\x00f\x000644\x001000\x001000\x001\x003\x00"
              b"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad\n")
    equal((len(vector), sha256_bytes(vector)),
          (116, "63ed0e2d6f3f43abc06cce1dd215d166131f25132b645ec6c027b50d1629c9c0"),
          EXIT_FORENSIC, "fixed-forensic-vectors", "two-record")

    begin("retained-v2-tree")
    v2_root_record, v2_records = direct_tree("v2", "retained-v2-tree")

    begin("retained-a5-control-tree")
    direct_tree("control", "retained-a5-control-tree")

    begin("retained-a5-patch")
    patch_data, patch_hash, patch_stat = stable_file(
        "/var/tmp/reap-g3-draft-v3-provenance.patch", "retained-a5-patch")
    equal((len(patch_data), patch_hash, patch_stat.st_mode & 0o7777,
           patch_stat.st_uid, patch_stat.st_gid, patch_stat.st_nlink,
           patch_data.count(b"diff --git ")),
          (56207, "fc340abca04400d0aff3fce73dcf5a309bdfaec838fc22bfd82aca5a46f55daf",
           0o664, 1000, 1000, 1, 5), EXIT_ENTRY, "retained-a5-patch",
          "patch-identity")

    begin("retained-failed-preview")
    recursive_preview("retained-failed-preview")

    begin("closed-required-absence-set")
    absent_paths = (
        "/var/tmp/reap-g3-draft-v3-review-1-scratch",
        "/var/tmp/reap-g3-draft-v3-review-2-scratch",
        repo + "/target/tmp/goal-g-amendment-3-preview-v2",
        "/var/tmp/reap-g3-draft-v4",
        "/var/tmp/reap-g3-draft-v4-provenance.patch",
        "/var/tmp/reap-g3-draft-v4-review-1-scratch",
        "/var/tmp/reap-g3-draft-v4-review-2-scratch",
        repo + "/target/tmp/goal-g-amendment-3-preview-v3",
        "/var/tmp/reap-g3-draft-v5",
        "/var/tmp/reap-g3-draft-v5-provenance.patch",
        "/var/tmp/reap-g3-draft-v5-review-1-scratch",
        "/var/tmp/reap-g3-draft-v5-review-2-scratch",
        repo + "/target/tmp/goal-g-amendment-3-preview-v4",
        repo + "/target/tmp/goal-g-amendment-3-recorder-bundle",
        repo + "/target/tmp/goal-g-phase0-amendment-3",
        repo + "/target/tmp/goal-g-amendment-3-runtime",
    )
    for absent_path in absent_paths:
        ensure_absent(absent_path, "closed-required-absence-set")

    begin("busybox-and-argv-vectors")
    _, busybox_hash, _ = stable_file("/bin/busybox", "busybox-and-argv-vectors")
    equal(busybox_hash, "c2f279d1d5640a0f327890d41cad594c0f059f3fed3f96dd72fdcc4f5e18ec02",
          EXIT_ENTRY, "busybox-and-argv-vectors", "busybox")
    copy_argv = ("/bin/busybox", "cp", "-a", "--",
                 "/var/tmp/reap-g3-draft-v3", "/var/tmp/reap-g3-draft-v5")
    copy_stream = b"".join(item.encode("ascii") + b"\0" for item in copy_argv)
    equal((len(copy_argv), len(copy_stream), sha256_bytes(copy_stream)),
          (6, 74, "18d707d79567219d0ca519b9e8de54a56d595682de8b1b2f739792c76f15806d"),
          EXIT_ENTRY, "busybox-and-argv-vectors", "copy-argv")
    preview_argv = ("/bin/busybox", "sh",
                    "/var/tmp/reap-g3-draft-v5/construct-self-test.preview.sh",
                    "preview", repo + "/target/tmp/goal-g-amendment-3-preview-v4")
    preview_stream = b"".join(item.encode("ascii") + b"\0" for item in preview_argv)
    equal((len(preview_argv), len(preview_stream), sha256_bytes(preview_stream)),
          (5, 145, "461c5989b5ccc0b2a4931051a7f215ad2fc7088f3945876048dcb7b860837e73"),
          EXIT_ENTRY, "busybox-and-argv-vectors", "preview-argv")

    begin("final-v3-copy-source-tree")
    v3_root_record, v3_records = direct_tree("v3", "final-v3-copy-source-tree")
    equal((len(v3_root_record), sha256_bytes(v3_root_record)),
          (28, "5c5f2aa15f151a1c1fd8285ee13c42e968e17889c99ad85c06e544080824ba81"),
          EXIT_FORENSIC, "final-v3-copy-source-tree", "root-record")
    command_records = [record for rel, record in v3_records if rel == b"commands.tsv"]
    equal(len(command_records), 1, EXIT_FORENSIC, "final-v3-copy-source-tree",
          "commands-record-count")
    equal((len(command_records[0]), sha256_bytes(command_records[0])),
          (102, "3ca42fa79530d356d42a05c2324d7ea09132e0d8ae5882e9285e7cff5abd3bea"),
          EXIT_FORENSIC, "final-v3-copy-source-tree", "commands-record")

    equal(assertion_index, len(ASSERTION_IDS), EXIT_SEQUENCE,
          "assertion-sequence", "assertion-underrun")
    for label, value in (
        ("goal_g_amendment_7_authentication", "pass"),
        ("goal_g_amendment_7_g7_auth_commit", g7_commit),
        ("goal_g_amendment_7_g7_auth_tree", g7_tree),
        ("goal_g_amendment_7_g7_auth_parent", g7_parent),
        ("goal_g_amendment_7_g7_auth_subject", g7_subject),
        ("goal_g_amendment_7_g7_auth_contract_sha256", contract_hash),
        ("goal_g_amendment_7_g7_auth_handoff_sha256", handoff_hash),
        ("goal_g_amendment_7_v3_forensic_inventory_sha256",
         "9ab5b0695e60cf47fc41e9d2e110b291c56b856d80cbfad36b9d41b6a5c7d233"),
    ):
        os.write(1, (label + "=" + value + "\n").encode("ascii"))
except SystemExit:
    raise
except Exception:
    abort(EXIT_CONCURRENT, "internal-runtime", "unexpected-exception")
PY
verifier_status=$?
set -e
case $verifier_status in
  0) ;;
  67|68|69|70|71|72|73) exit "$verifier_status" ;;
  *) fail 66 verifier-child-runtime ;;
esac

storage_preflight || fail 66 final-copy-storage-preflight
exec /bin/busybox cp -a -- /var/tmp/reap-g3-draft-v3 /var/tmp/reap-g3-draft-v5 || fail 66 copy-exec
```
<!-- GOAL-G-A7-PRECOPY-AND-COPY-SOURCE-END -->

The frozen launcher identities are:

```text
precopy_launcher_bytes=31518
precopy_launcher_sha256=d42320de72049a32d84710ae0b2944ee6fcb656b8910bd44e67f987a3ad73934
```

The closed exit classes are:

```text
64 invocation-schema
65 bootstrap-or-launcher-source-mismatch
66 child-runtime-or-storage-preflight-failure
67 repository-lineage-status-or-contract-mismatch
68 retained-entry-identity-or-content-mismatch
69 component-manifest-mismatch
70 forensic-inventory-or-fixed-vector-mismatch
71 required-absence-or-ancestor-mismatch
72 concurrent-change-or-internal-runtime-failure
73 canonical-sequence-deviation
```

The exact final copy argv and identity are:

```text
/bin/busybox
cp
-a
--
/var/tmp/reap-g3-draft-v3
/var/tmp/reap-g3-draft-v5
argv_count=6
argv_nul_bytes=74
argv_nul_sha256=18d707d79567219d0ca519b9e8de54a56d595682de8b1b2f739792c76f15806d
busybox_sha256=c2f279d1d5640a0f327890d41cad594c0f059f3fed3f96dd72fdcc4f5e18ec02
```

## Fresh v5 namespace and construction boundary

The new paths are exactly:

```text
v5_root=/var/tmp/reap-g3-draft-v5
v5_patch=/var/tmp/reap-g3-draft-v5-provenance.patch
review_1_scratch=/var/tmp/reap-g3-draft-v5-review-1-scratch
review_2_scratch=/var/tmp/reap-g3-draft-v5-review-2-scratch
preview_root=target/tmp/goal-g-amendment-3-preview-v4
```

No v4 or preview-v3 name may be reused. Exact authenticated v3 is the sole
v5 source and control. No separate v5 control is authorized.

Exactly these five v5 files may differ from v3:

```text
SELF-TEST-DESIGN.md
SELF-TEST-SCHEMA.md
construct-self-test.preview.sh
run-attempt.sh
validators.sh
```

Exactly these five remain byte-identical:

```text
commands.tsv
inventory.preview.sh
run-phase0-replay.preview.sh
source-reattest.preview.sh
summarize-baseline.preview.sh
```

Amendment 6's exact function-level edit allowlist and all functional anchors
remain controlling, with `v4` read as `v5` and the lineage additions below.
In particular, the corrected matcher, 3025-byte combined-fixture function,
179-row normalized redirection manifest, 116 fixture cases, and 1240 fixture
subcases remain unchanged. Every changed hunk is provenance-only.

The cumulative repository-fact and `phase0.meta` fields are exactly:

```text
g5_stop_commit g5_stop_tree g5_stop_parent g5_stop_subject g5_stop_handoff_sha256
g6_auth_commit g6_auth_tree g6_auth_parent g6_auth_subject g6_auth_contract_sha256 g6_auth_handoff_sha256
g6_stop_commit g6_stop_tree g6_stop_parent g6_stop_subject g6_stop_handoff_sha256
g7_auth_commit g7_auth_tree g7_auth_parent g7_auth_subject g7_auth_contract_sha256 g7_auth_handoff_sha256
```

All existing `s4_*`, `t_*`, and `a5_*` fields remain exact.
`candidate_parent` becomes exact `G7_AUTH`. No `a6_*` synonym is authorized.
Finalize runner, validators, design, and schema first; finalize constructor
last, avoiding a constructor self-hash.

The v5 patch is a standard Git full-index text patch directly from exact v3
to exact v5. It has exactly five sections and no rename, copy, mode, binary,
or extra-file marker. It is audit material only.

## Reviews, preview, official construction, and activation

Two distinct static reviewers in distinct sessions each create its exact
scratch root at most once, independently reproduce the direct v3-to-v5 patch,
all component and forensic identities, the complete allowed-edit proof, and
all functional anchors. Passed scratch may be removed only after its final
inventory is captured; failed scratch is preserved. Both must pass before the
preview.

The retained no-Cargo bootstrap check from Amendment 3 remains mandatory and
must pass before the one allowed preview invocation. The exact preview argv is:

```text
/bin/busybox
sh
/var/tmp/reap-g3-draft-v5/construct-self-test.preview.sh
preview
/home/ubuntu/code/reap/target/tmp/goal-g-amendment-3-preview-v4
argv_count=5
argv_nul_bytes=145
argv_nul_sha256=461c5989b5ccc0b2a4931051a7f215ad2fc7088f3945876048dcb7b860837e73
```

The preview root may be created once. Two distinct post-preview reviewers
must pass before fresh official construction. The unchanged official roots
remain:

```text
target/tmp/goal-g-amendment-3-recorder-bundle
target/tmp/goal-g-phase0-amendment-3
target/tmp/goal-g-amendment-3-runtime
```

Two distinct official reviewers must pass before sealing. All original
Amendment 3 safety-false evidence and Amendment 6 review/evidence schemas
remain controlling, with v5 and Amendment 7 identities added under exact
`goal_g_amendment_7_` prefixes. No optional evidence field is authorized.

If all gates pass, `G3` is the direct child of exact `G7_AUTH`, changes only
the Goal G handoff, and uses exact subject:

```text
docs: activate goal g amendment 3
```

It changes only these current statuses:

```text
goal_g_amendment_3_status: activation-stopped-inactive -> active-phase0
goal_g_amendment_7_status: authorized-inactive -> activation-complete-phase0-active
```

Amendment 5 and Amendment 6 remain `activation-stopped-inactive`. `P0` remains
a direct child of `G3`, changes only the handoff, and uses exact subject:

```text
docs: qualify goal g amendment 3 phase 0
```

Only after valid `G3` may the original Phase 0 Cargo campaigns become
available. Only after valid `P0` may later original Goal G phases resume.

## Failure semantics

Any nonzero bootstrap, launcher, copy, construction, review, preview,
official, sealing, or activation result stops this lineage. Preserve every
created byte in its honest state. Do not retry, repair in place, reuse another
root, or continue to `G3`.

Whenever storage permits, a stop commit is the direct child of `G7_AUTH`,
modifies only the Goal G handoff, and uses exact subject:

```text
docs: record goal g amendment 7 activation stop
```

Relative to the authorization handoff it replaces exactly one
`goal_g_amendment_7_status=authorized-inactive` with
`goal_g_amendment_7_status=activation-stopped-inactive`, appends one terminal
block, and leaves every earlier byte unchanged. If storage preflight prevents
that edit or commit, make no further mutation. A later attempt requires a new
reviewed, user-authorized amendment.

## Storage, safety, and non-claims

The exact Amendment 4 2-GiB storage preflight remains mandatory immediately
before every external child, write, redirect, executable-bit change, tracked
edit, staging operation, and commit. Its own `git`, `df`, and `awk` children
are the only recursion exception.

From `G7_AUTH` through valid `G3`, Cargo, rustc, rustdoc, rustfmt, test and
benchmark binaries, public fetches, network children, credentials,
authenticated requests, Polygon RPC, and production order entry are
prohibited.

```text
production_order_entry_authorized=false
real_credentials_loaded=false
authenticated_external_request_sent=false
real_polygon_rpc_request_sent=false
real_order_submitted=false
historical_goal_g_attempt_relabelled=false
historical_goal_g_r_equivalence_claimed=false
amendment_6_retry_or_completion_claimed=false
v4_or_preview_v3_reuse_authorized=false
push_authorized=false
```
