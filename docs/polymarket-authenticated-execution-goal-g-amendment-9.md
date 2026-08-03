# Goal G Amendment 9: Constructor Bootstrap and No-Scratch Review Recovery

Status: authorized for execution

Authorization date: 2026-08-03

Scope: preserve Amendment 8 evidence, authenticate retained v5 and v6, create
a fresh v7 from exact v3, repair only the constructor bootstrap child boundary
and the impossible static-review scratch schema, and resume Goal G activation
under the existing one-child execution boundary

## Purpose

Amendment 8's reviewed canonical launcher passed all 17 child envelopes,
authenticated the repository and retained evidence, copied exact v3 to fresh
v6, and post-verified the copy. Before any v6 edit or invocation, static audit
found five bootstrap children in `construct-self-test.preview.sh` before its
runtime `storage_preflight` definition. Amendment 8 did not authorize editing
that bootstrap region. It also inherited Amendment 6's requirement to create,
inventory, and remove review scratch while simultaneously requiring
child-free reviewers and permanently absent scratch roots. No v6 byte was
edited or invoked, and no patch, scratch, preview, or official artifact was
created.

This amendment preserves both stops and v5/v6 byte-for-byte. v5 and v6 are
comparison evidence only and are never mutated, invoked, promoted, or used as
bundle inputs. One new closed launcher authenticates exact `G8_STOP`, exact
v3, retained v5, retained v6, and every successor absence; copies exact v3 to
fresh v7 while remaining alive; and post-verifies the copy before releasing
its success record. v7 is the only new construction root.

After the copy, this amendment permits exactly two non-provenance changes:

1. add one bootstrap-local exact storage-preflight function and invoke it
   immediately before each of the five bootstrap children; and
2. replace the inherited created-and-removed scratch schema with an eight-field
   no-scratch schema whose reserved paths remain absent before and after each
   child-free static review.

It supersedes only the conflicting successor, status, construction-root,
review-root, preview-root, activation-parent, constructor-bootstrap child
boundary, and static-review scratch lifecycle/schema clauses in Amendments 3
through 8. All other safety, workload, retained no-Cargo bootstrap, review,
sealing, and Phase 0 requirements remain controlling.

From canonical bootstrap entry until the launcher returns success or failure,
the executor has exclusive mutation ownership of the repository, index, HEAD,
v3, retained v5, retained v6, and every named v7, patch, reserved scratch,
preview, official, and runtime path. No concurrent session or process may
mutate any of them. After post-copy verification, the launcher separately
rechecks exact HEAD, exact tree, and clean tracked/untracked status before
releasing buffered success.

## Immutable boundary

The direct parent is:

```text
G8_STOP_commit=e4312e6ce93d411c69a0602dca0014fc41a1467e
G8_STOP_tree=d594daa309f31b4533afdbf0d0cf71ed54c2bc80
G8_STOP_parent=4fa757f9ff2c6d4e748b30a87f664c2710f57848
G8_STOP_subject=docs: record goal g amendment 8 activation stop
G8_STOP_delta_path_count=1
G8_STOP_delta_paths=docs/polymarket-authenticated-execution-goal-g-handoff.md
G8_STOP_handoff_sha256=fd0afaad2cea179b61865a9e04904b192f116f05069f057be3279f689ccf5201
G8_STOP_handoff_bytes=144742
```

The successor chain is:

```text
R6 -> S4 -> T -> A5 -> G5_STOP -> G6_AUTH -> G6_STOP -> G7_AUTH -> G7_STOP -> G8_AUTH -> G8_STOP -> G9_AUTH -> G3 -> P0
```

Historical Goal G-R retains its existing aliases. This authorization is
`G9_AUTH`; no commit may be amended, replaced, skipped, or assigned another
alias.

## Preserved statuses and artifacts

Before `G3`, the handoff contains exactly one of each:

```text
goal_g_amendment_3_status=activation-stopped-inactive
goal_g_amendment_5_status=activation-stopped-inactive
goal_g_amendment_6_status=activation-stopped-inactive
goal_g_amendment_7_status=activation-stopped-inactive
goal_g_amendment_8_status=activation-stopped-inactive
goal_g_amendment_9_status=authorized-inactive
```

Every Amendment 3, 5, 6, 7, and 8 terminal block is immutable. Amendment 8
remains stopped even if Amendment 9 succeeds. Its source reviews, canonical
authentication pass, successful copy, and pre-construction audit retain their
exact historical meanings.

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
v5. The launcher reauthenticates v3, v5, and v6 independently. Matching
aggregate hashes never substitute for per-file hashes and metadata.

## G9_AUTH commit

`G9_AUTH` is the direct child of exact `G8_STOP`, changes only:

- `docs/polymarket-authenticated-execution-goal-g-amendment-9.md`; and
- `docs/polymarket-authenticated-execution-goal-g-handoff.md`.

Its exact subject is:

```text
docs: authorize goal g amendment 9 constructor bootstrap and review schema recovery
```

The handoff appends one `goal_g_amendment_9_status=authorized-inactive` and
does not modify an earlier byte. The pre-commit contract does not contain the
future G9 commit, tree, or handoff hash. The user's 2026-08-03 instruction
authorizes this amendment's execution but does not authorize pushing its
future commits.

Two distinct read-only source reviewers in distinct sessions must pass before
the authorization commit. For review number `N` in `1,2`, the handoff binds
exactly:

```text
goal_g_amendment_9_source_review_N_result
goal_g_amendment_9_source_review_N_reviewer
goal_g_amendment_9_source_review_N_session
goal_g_amendment_9_source_review_N_contract_sha256
goal_g_amendment_9_source_review_N_bootstrap_sha256
goal_g_amendment_9_source_review_N_launcher_sha256
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

<!-- GOAL-G-A9-BOOTSTRAP-SOURCE-BEGIN -->
```python
import hashlib, os, sys

def die(message, code=65):
    try:
        os.write(2, ("goal-g-a9-bootstrap:" + message + "\n").encode("ascii"))
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
    if contract_path != "/home/ubuntu/code/reap/docs/polymarket-authenticated-execution-goal-g-amendment-9.md":
        die("contract-path", 64)
    if handoff_path != "/home/ubuntu/code/reap/docs/polymarket-authenticated-execution-goal-g-handoff.md":
        die("handoff-path", 64)
    contract = read_regular(contract_path)
    handoff = read_regular(handoff_path)
    if len(self_bytes) != decimal_field(handoff, "goal_g_amendment_9_bootstrap_bytes"):
        die("bootstrap-bytes")
    if hashlib.sha256(self_bytes).hexdigest() != field(handoff, "goal_g_amendment_9_bootstrap_sha256"):
        die("bootstrap-sha256")
    start = b"<!-- GOAL-G-A9-LAUNCHER-SOURCE-BEGIN -->\n```bash\n"
    end = b"\n```\n<!-- GOAL-G-A9-LAUNCHER-SOURCE-END -->"
    if contract.count(start) != 1 or contract.count(end) != 1:
        die("launcher-markers")
    source = contract.split(start, 1)[1].split(end, 1)[0]
    if b"\x00" in source:
        die("launcher-nul")
    if len(source) != decimal_field(handoff, "goal_g_amendment_9_launcher_bytes"):
        die("launcher-bytes")
    if hashlib.sha256(source).hexdigest() != field(handoff, "goal_g_amendment_9_launcher_sha256"):
        die("launcher-sha256")
    try:
        decoded = source.decode("utf-8")
    except UnicodeDecodeError:
        die("launcher-utf8")
    os.execve(
        "/bin/bash",
        ["/bin/bash", "--noprofile", "--norc", "-c", decoded,
         "goal-g-a9-authenticate-and-copy"],
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
<!-- GOAL-G-A9-BOOTSTRAP-SOURCE-END -->

```text
bootstrap_bytes=3596
bootstrap_sha256=a2f7ac2bba3e00d67450d4e80be1ebe9b3f108113e0bcae39f000260336596ef
```

## Exact authenticate-and-copy launcher

The launcher is the complete pre-v7 authority. It executes each listed
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

<!-- GOAL-G-A9-LAUNCHER-SOURCE-BEGIN -->
```bash
set -Eeuo pipefail

readonly REPO=/home/ubuntu/code/reap
readonly CONTRACT=$REPO/docs/polymarket-authenticated-execution-goal-g-amendment-9.md
readonly HANDOFF=$REPO/docs/polymarket-authenticated-execution-goal-g-handoff.md
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
  printf 'goal-g-a9:failure:%s:%s\n' "$code" "$assertion" >&2 || :
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
  repository-root repository-branch repository-clean g9-commit g9-tree
  g9-parent g9-subject g9-two-path-delta g8-stop-object g8-auth-object
  first-parent-lineage pre-copy-verifier v3-to-v7-copy post-copy-verifier
  final-g9-commit final-g9-tree final-repository-clean
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
  repository-clean g9-commit g9-tree g9-parent g9-subject g9-two-path-delta
  g8-stop-object g8-auth-object first-parent-lineage
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
[[ $# -eq 0 && $0 == goal-g-a9-authenticate-and-copy ]] || fail 64 invocation-schema

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

begin_shell_assertion g9-commit
capture_child g9_commit g9-commit /usr/bin/git rev-parse HEAD
[[ $g9_commit =~ ^[0-9a-f]{40}$ ]] || fail 67 g9-commit

begin_shell_assertion g9-tree
capture_child g9_tree g9-tree /usr/bin/git rev-parse 'HEAD^{tree}'
[[ $g9_tree =~ ^[0-9a-f]{40}$ ]] || fail 67 g9-tree

begin_shell_assertion g9-parent
capture_child g9_parent g9-parent /usr/bin/git rev-parse 'HEAD^'
[[ $g9_parent == "$G8_STOP" ]] || fail 67 g9-parent

begin_shell_assertion g9-subject
capture_child g9_subject g9-subject /usr/bin/git show -s --format=%s HEAD
[[ $g9_subject == 'docs: authorize goal g amendment 9 constructor bootstrap and review schema recovery' ]] || fail 67 g9-subject

begin_shell_assertion g9-two-path-delta
capture_child g9_delta g9-two-path-delta /usr/bin/git diff-tree --no-commit-id --name-only -r HEAD
[[ $g9_delta == $'docs/polymarket-authenticated-execution-goal-g-amendment-9.md\ndocs/polymarket-authenticated-execution-goal-g-handoff.md' ]] || fail 67 g9-two-path-delta

begin_shell_assertion g8-stop-object
capture_child g8_stop_object g8-stop-object /usr/bin/git show -s --format='%H%x09%T%x09%P%x09%s' "$G8_STOP"
[[ $g8_stop_object == $'e4312e6ce93d411c69a0602dca0014fc41a1467e\td594daa309f31b4533afdbf0d0cf71ed54c2bc80\t4fa757f9ff2c6d4e748b30a87f664c2710f57848\tdocs: record goal g amendment 8 activation stop' ]] || fail 67 g8-stop-object

begin_shell_assertion g8-auth-object
capture_child g8_auth_object g8-auth-object /usr/bin/git show -s --format='%H%x09%T%x09%P%x09%s' "$G8_AUTH"
[[ $g8_auth_object == $'4fa757f9ff2c6d4e748b30a87f664c2710f57848\t063e991a51923655bdc8cd63fc7850543bad56a6\t49210315169fa7ec3e3c02b4e70a745105bf9476\tdocs: authorize goal g amendment 8 per-child preflight recovery' ]] || fail 67 g8-auth-object

begin_shell_assertion first-parent-lineage
capture_child lineage first-parent-lineage /usr/bin/git rev-list --first-parent --max-count=12 HEAD
[[ $lineage == "$g9_commit"$'\n'"$G8_STOP"$'\n'"$G8_AUTH"$'\n'"$G7_STOP"$'\n'"$G7_AUTH"$'\n'"$G6_STOP"$'\n'"$G6_AUTH"$'\n'"$G5_STOP"$'\n'"$A5"$'\n'"$T"$'\n'"$S4"$'\n'"$R6" ]] || fail 67 first-parent-lineage

begin_shell_assertion shell-assertion-sequence-complete
(( shell_assertion_index == ${#SHELL_ASSERTION_IDS[@]} )) || fail 73 assertion-underrun

export GOAL_G_A9_REPO=$REPO
export GOAL_G_A9_CONTRACT=$CONTRACT
export GOAL_G_A9_HANDOFF=$HANDOFF
export GOAL_G_A9_G9_COMMIT=$g9_commit
export GOAL_G_A9_G9_TREE=$g9_tree
export GOAL_G_A9_G9_PARENT=$g9_parent
export GOAL_G_A9_G9_SUBJECT=$g9_subject

begin_child pre-copy-verifier
set +e
pre_output=$(
  storage_preflight || exit 125
  exec /usr/bin/python3 -I -S <<'PY'
import hashlib, os, stat

ASSERTION_IDS = (
    "handoff-status-and-cross-binding",
    "source-review-evidence",
    "a8-terminal-evidence",
    "fixed-forensic-vector",
    "retained-v5-v6-trees",
    "required-absence-set",
    "busybox-and-argv",
    "final-v3-source-tree",
)
index = 0

def abort(code, assertion, detail):
    try:
        os.write(2, ("goal-g-a9:failure:%d:%s:%s\n" %
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

FILES = {
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

def direct_tree(path, inode, assertion):
    root = os.fsencode(path)
    before = os.lstat(root)
    if not stat.S_ISDIR(before.st_mode):
        abort(68, assertion, "root-type")
    equal((before.st_dev, before.st_ino, before.st_mode & 0o7777,
           before.st_uid, before.st_gid, before.st_nlink, before.st_size),
          (66305, inode, 0o700, 1000, 1000, 2, 4096), 68, assertion,
          "root-metadata")
    names = sorted(os.listdir(root))
    wanted = sorted(name.encode("ascii") for name in FILES)
    equal(names, wanted, 68, assertion, "child-set")
    component = bytearray()
    records = [(b".", b".\x00d\x000700\x001000\x001000\x002\x004096\x00-\n")]
    total = 0
    snapshots = {}
    for raw in wanted:
        name = raw.decode("ascii")
        data, digest, entry = stable_file(os.path.join(root, raw), assertion)
        expected_hash, expected_bytes, expected_mode = FILES[name]
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
          (1055725, 10, 933, "710ab62d5dbe846b21df74a4d78ee3f12d2a1883a22662d256bf751d411bc451"),
          69, assertion, "component")
    equal((len(records), len(inventory), hashlib.sha256(inventory).hexdigest()),
          (11, 1151, "9ab5b0695e60cf47fc41e9d2e110b291c56b856d80cbfad36b9d41b6a5c7d233"),
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
    repo = os.environ["GOAL_G_A9_REPO"]
    contract_path = os.environ["GOAL_G_A9_CONTRACT"]
    handoff_path = os.environ["GOAL_G_A9_HANDOFF"]
    g9_commit = os.environ["GOAL_G_A9_G9_COMMIT"]
    g9_tree = os.environ["GOAL_G_A9_G9_TREE"]
    g9_parent = os.environ["GOAL_G_A9_G9_PARENT"]
    g9_subject = os.environ["GOAL_G_A9_G9_SUBJECT"]

    begin("handoff-status-and-cross-binding")
    _, contract_hash, _ = stable_file(contract_path, "handoff-status-and-cross-binding")
    handoff_data, handoff_hash, _ = stable_file(handoff_path, "handoff-status-and-cross-binding")
    parent_handoff_bytes = 144742
    equal(len(handoff_data) > parent_handoff_bytes, True, 67,
          "handoff-status-and-cross-binding", "appended-length")
    equal(hashlib.sha256(handoff_data[:parent_handoff_bytes]).hexdigest(),
          "fd0afaad2cea179b61865a9e04904b192f116f05069f057be3279f689ccf5201",
          67, "handoff-status-and-cross-binding", "parent-prefix")
    suffix = handoff_data[parent_handoff_bytes:]
    suffix_start = b"\n## User-Authorized Amendment 9 \xe2\x80\x94 2026-08-03\n"
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
        ("goal_g_amendment_9_status", "authorized-inactive"),
        ("goal_g_amendment_9_contract_sha256", contract_hash),
        ("goal_g_amendment_9_parent_commit", "e4312e6ce93d411c69a0602dca0014fc41a1467e"),
        ("goal_g_amendment_9_parent_handoff_sha256", "fd0afaad2cea179b61865a9e04904b192f116f05069f057be3279f689ccf5201"),
        ("goal_g_amendment_9_parent_handoff_bytes", "144742"),
        ("goal_g_amendment_9_authorization_subject", "docs: authorize goal g amendment 9 constructor bootstrap and review schema recovery"),
        ("goal_g_amendment_9_lineage", "R6->S4->T->A5->G5_STOP->G6_AUTH->G6_STOP->G7_AUTH->G7_STOP->G8_AUTH->G8_STOP->G9_AUTH->G3->P0"),
    ):
        equal(unique(values, name, "handoff-status-and-cross-binding"), expected,
              67, "handoff-status-and-cross-binding", name)
    equal((g9_parent, g9_subject),
          ("e4312e6ce93d411c69a0602dca0014fc41a1467e",
           "docs: authorize goal g amendment 9 constructor bootstrap and review schema recovery"),
          67, "handoff-status-and-cross-binding", "g9-runtime")

    begin("source-review-evidence")
    bootstrap_hash = unique(values, "goal_g_amendment_9_bootstrap_sha256", "source-review-evidence")
    launcher_hash = unique(values, "goal_g_amendment_9_launcher_sha256", "source-review-evidence")
    allowed_review_keys = set()
    for number, reviewer, session in (
        ("1", "g9-contract-review-1", "g9-contract-review-1-20260803"),
        ("2", "g9-boundary-review-2", "g9-boundary-review-2-20260803"),
    ):
        prefix = "goal_g_amendment_9_source_review_" + number + "_"
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
        if name.startswith("goal_g_amendment_9_source_review_")
    }
    equal(actual_review_keys, allowed_review_keys, 67, "source-review-evidence",
          "exact-review-namespace")

    begin("a8-terminal-evidence")
    for name, expected in (
        ("goal_g_amendment_8_activation_stop_status", "stopped"),
        ("goal_g_amendment_8_activation_stop_parent_commit", "4fa757f9ff2c6d4e748b30a87f664c2710f57848"),
        ("goal_g_amendment_8_activation_stop_parent_handoff_sha256", "f3f1f3d6c47e30f85e27c739478876f846a7d8e25ee399eb349f941e46d4c345"),
        ("goal_g_amendment_8_activation_stop_failure_class", "authorized-edit-surface-cannot-satisfy-execution-boundary"),
        ("goal_g_amendment_8_activation_stop_canonical_authentication_result", "pass"),
        ("goal_g_amendment_8_activation_stop_copy_exit", "0"),
        ("goal_g_amendment_8_activation_stop_v6_state", "retained-non-authoritative-exact-copy-not-edited-not-invoked"),
        ("goal_g_amendment_8_activation_stop_v6_forensic_inventory_sha256", "9ab5b0695e60cf47fc41e9d2e110b291c56b856d80cbfad36b9d41b6a5c7d233"),
    ):
        equal(unique(values, name, "a8-terminal-evidence"), expected,
              67, "a8-terminal-evidence", name)

    begin("fixed-forensic-vector")
    vector = (b".\x00d\x000700\x001000\x001000\x002\x004096\x00-\n"
              b"a\x00f\x000644\x001000\x001000\x001\x003\x00"
              b"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad\n")
    equal((len(vector), hashlib.sha256(vector).hexdigest()),
          (116, "63ed0e2d6f3f43abc06cce1dd215d166131f25132b645ec6c027b50d1629c9c0"),
          70, "fixed-forensic-vector", "vector")

    begin("retained-v5-v6-trees")
    direct_tree("/var/tmp/reap-g3-draft-v5", 310596, "retained-v5-v6-trees")
    direct_tree("/var/tmp/reap-g3-draft-v6", 310607, "retained-v5-v6-trees")

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
        "/var/tmp/reap-g3-draft-v7",
        "/var/tmp/reap-g3-draft-v7-provenance.patch",
        "/var/tmp/reap-g3-draft-v7-review-1-scratch",
        "/var/tmp/reap-g3-draft-v7-review-2-scratch",
        repo + "/target/tmp/goal-g-amendment-3-preview-v6",
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
            "/var/tmp/reap-g3-draft-v3", "/var/tmp/reap-g3-draft-v7")
    stream = b"".join(value.encode() + b"\0" for value in argv)
    equal((len(argv), len(stream), hashlib.sha256(stream).hexdigest()),
          (6, 74, "4cb778859b673a5f5a3d2f03b6325f2f2decad82d2dbf4039fbb41a9040141e4"),
          68, "busybox-and-argv", "copy-argv")

    begin("final-v3-source-tree")
    records = direct_tree("/var/tmp/reap-g3-draft-v3", 310585, "final-v3-source-tree")
    root_record = [record for rel, record in records if rel == b"."][0]
    equal((len(root_record), hashlib.sha256(root_record).hexdigest()),
          (28, "5c5f2aa15f151a1c1fd8285ee13c42e968e17889c99ad85c06e544080824ba81"),
          70, "final-v3-source-tree", "root-record")

    equal(index, len(ASSERTION_IDS), 73, "assertion-sequence", "underrun")
    for name, value in (
        ("goal_g_a9_pre_contract_sha256", contract_hash),
        ("goal_g_a9_pre_handoff_sha256", handoff_hash),
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
pre_contract_sha256=${pre_contract_line#goal_g_a9_pre_contract_sha256=}
pre_handoff_sha256=${pre_handoff_line#goal_g_a9_pre_handoff_sha256=}
[[ $pre_contract_line == goal_g_a9_pre_contract_sha256="$pre_contract_sha256" && $pre_contract_sha256 =~ ^[0-9a-f]{64}$ ]] || fail 67 verifier-contract-output
[[ $pre_handoff_line == goal_g_a9_pre_handoff_sha256="$pre_handoff_sha256" && $pre_handoff_sha256 =~ ^[0-9a-f]{64}$ ]] || fail 67 verifier-handoff-output
readonly pre_contract_sha256 pre_handoff_sha256
export GOAL_G_A9_PRE_CONTRACT_SHA256=$pre_contract_sha256
export GOAL_G_A9_PRE_HANDOFF_SHA256=$pre_handoff_sha256

begin_child v3-to-v7-copy
set +e
(
  storage_preflight || exit 125
  exec /bin/busybox cp -a -- /var/tmp/reap-g3-draft-v3 /var/tmp/reap-g3-draft-v7
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
        os.write(2, ("goal-g-a9:post-copy-failure:%d:%s\n" %
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

FILES = {
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

def direct_tree(path, expected_inode, allow_fresh_inode=False):
    root = os.fsencode(path)
    before = os.lstat(root)
    if not stat.S_ISDIR(before.st_mode):
        abort(68, "root-type")
    if allow_fresh_inode:
        if before.st_dev != 66305 or before.st_ino in (310585, 310596, 310607):
            abort(68, "fresh-root-identity")
    else:
        equal((before.st_dev, before.st_ino), (66305, expected_inode),
              68, "root-identity")
    equal((before.st_mode & 0o7777, before.st_uid, before.st_gid,
           before.st_nlink, before.st_size),
          (0o700, 1000, 1000, 2, 4096), 68, "root-metadata")
    names = sorted(os.listdir(root))
    wanted = sorted(name.encode("ascii") for name in FILES)
    equal(names, wanted, 68, "child-set")
    component = bytearray()
    records = [(b".", b".\x00d\x000700\x001000\x001000\x002\x004096\x00-\n")]
    total = 0
    snapshots = {}
    for raw in wanted:
        name = raw.decode("ascii")
        digest, size, entry = stable_file(os.path.join(root, raw))
        expected_hash, expected_size, expected_mode = FILES[name]
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
          (1055725, 10, 933,
           "710ab62d5dbe846b21df74a4d78ee3f12d2a1883a22662d256bf751d411bc451"),
          69, "component")
    equal((len(records), len(inventory), hashlib.sha256(inventory).hexdigest()),
          (11, 1151,
           "9ab5b0695e60cf47fc41e9d2e110b291c56b856d80cbfad36b9d41b6a5c7d233"),
          70, "forensic")
    return before.st_dev, before.st_ino

def stable_document(path):
    digest, size, _ = stable_file(path)
    return digest, size

try:
    v3 = direct_tree("/var/tmp/reap-g3-draft-v3", 310585)
    v5 = direct_tree("/var/tmp/reap-g3-draft-v5", 310596)
    v6 = direct_tree("/var/tmp/reap-g3-draft-v6", 310607)
    v7 = direct_tree("/var/tmp/reap-g3-draft-v7", None, True)
    if v7 in (v3, v5, v6):
        abort(68, "fresh-root-alias")
    for path in (
        "/var/tmp/reap-g3-draft-v6-provenance.patch",
        "/var/tmp/reap-g3-draft-v6-review-1-scratch",
        "/var/tmp/reap-g3-draft-v6-review-2-scratch",
        os.environ["GOAL_G_A9_REPO"] + "/target/tmp/goal-g-amendment-3-preview-v5",
        "/var/tmp/reap-g3-draft-v7-provenance.patch",
        "/var/tmp/reap-g3-draft-v7-review-1-scratch",
        "/var/tmp/reap-g3-draft-v7-review-2-scratch",
        os.environ["GOAL_G_A9_REPO"] + "/target/tmp/goal-g-amendment-3-preview-v6",
        os.environ["GOAL_G_A9_REPO"] + "/target/tmp/goal-g-amendment-3-recorder-bundle",
        os.environ["GOAL_G_A9_REPO"] + "/target/tmp/goal-g-phase0-amendment-3",
        os.environ["GOAL_G_A9_REPO"] + "/target/tmp/goal-g-amendment-3-runtime",
    ):
        if os.path.lexists(path):
            abort(71, "post-copy-auxiliary-present")
    contract_hash, _ = stable_document(os.environ["GOAL_G_A9_CONTRACT"])
    handoff_hash, _ = stable_document(os.environ["GOAL_G_A9_HANDOFF"])
    equal(contract_hash, os.environ["GOAL_G_A9_PRE_CONTRACT_SHA256"],
          72, "contract-pre-post-hash")
    equal(handoff_hash, os.environ["GOAL_G_A9_PRE_HANDOFF_SHA256"],
          72, "handoff-pre-post-hash")
    for name, value in (
        ("goal_g_amendment_9_result", "authenticate-copy-postverify-pass"),
        ("goal_g_amendment_9_g9_auth_commit", os.environ["GOAL_G_A9_G9_COMMIT"]),
        ("goal_g_amendment_9_g9_auth_tree", os.environ["GOAL_G_A9_G9_TREE"]),
        ("goal_g_amendment_9_g9_auth_parent", os.environ["GOAL_G_A9_G9_PARENT"]),
        ("goal_g_amendment_9_g9_auth_subject", os.environ["GOAL_G_A9_G9_SUBJECT"]),
        ("goal_g_amendment_9_g9_auth_contract_sha256", contract_hash),
        ("goal_g_amendment_9_g9_auth_handoff_sha256", handoff_hash),
        ("goal_g_amendment_9_v7_root_dev", str(v7[0])),
        ("goal_g_amendment_9_v7_root_inode", str(v7[1])),
        ("goal_g_amendment_9_v7_forensic_inventory_sha256",
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

capture_child final_g9_commit final-g9-commit /usr/bin/git rev-parse HEAD
[[ $final_g9_commit == "$g9_commit" ]] || fail 67 final-g9-commit

capture_child final_g9_tree final-g9-tree /usr/bin/git rev-parse 'HEAD^{tree}'
[[ $final_g9_tree == "$g9_tree" ]] || fail 67 final-g9-tree

capture_child final_repository_status final-repository-clean /usr/bin/git status --porcelain=v1 --untracked-files=all
[[ -z $final_repository_status ]] || fail 67 final-repository-clean

(( child_index == ${#CHILD_IDS[@]} )) || fail 73 child-underrun
printf '%s\n' "$post_output"
```
<!-- GOAL-G-A9-LAUNCHER-SOURCE-END -->

```text
launcher_bytes=32151
launcher_sha256=2a1b153f92a49eae546d152471325d8e804c172575a37aa444ccefe3f5197887
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
/var/tmp/reap-g3-draft-v7
argv_count=6
argv_nul_bytes=74
argv_nul_sha256=4cb778859b673a5f5a3d2f03b6325f2f2decad82d2dbf4039fbb41a9040141e4
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

## Fresh v7 boundary

The new paths are:

```text
v7_root=/var/tmp/reap-g3-draft-v7
v7_patch=/var/tmp/reap-g3-draft-v7-provenance.patch
review_1_scratch=/var/tmp/reap-g3-draft-v7-review-1-scratch
review_2_scratch=/var/tmp/reap-g3-draft-v7-review-2-scratch
preview_root=target/tmp/goal-g-amendment-3-preview-v6
```

v3 is the sole source/control. v5 and v6 remain immutable evidence. Exactly
these five v7 files may differ from v3:

```text
SELF-TEST-DESIGN.md
SELF-TEST-SCHEMA.md
construct-self-test.preview.sh
run-attempt.sh
validators.sh
```

The other five remain byte-identical. Amendment 6's exact provenance edit
allowlist and all matcher, fixture, redirection-manifest, case, subcase, and
body-hash anchors remain controlling with `v4` read as `v7`, except for the
one constructor-bootstrap behavior hunk and eight-field review schema closed
below.

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
`EXPECTED_MANIFEST`, and lineage/status/evidence emission. In
`run-attempt.sh` and `validators.sh`, Amendment 6's existing provenance
allowlist remains exact. Design and schema may describe only the authorized
provenance, bootstrap-boundary, and no-scratch schema changes. Every other
byte in the five changed files remains byte-identical to v3.

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
fields and add exactly these 44 fields:

```text
g5_stop_commit g5_stop_tree g5_stop_parent g5_stop_subject g5_stop_handoff_sha256
g6_auth_commit g6_auth_tree g6_auth_parent g6_auth_subject g6_auth_contract_sha256 g6_auth_handoff_sha256
g6_stop_commit g6_stop_tree g6_stop_parent g6_stop_subject g6_stop_handoff_sha256
g7_auth_commit g7_auth_tree g7_auth_parent g7_auth_subject g7_auth_contract_sha256 g7_auth_handoff_sha256
g7_stop_commit g7_stop_tree g7_stop_parent g7_stop_subject g7_stop_handoff_sha256
g8_auth_commit g8_auth_tree g8_auth_parent g8_auth_subject g8_auth_contract_sha256 g8_auth_handoff_sha256
g8_stop_commit g8_stop_tree g8_stop_parent g8_stop_subject g8_stop_handoff_sha256
g9_auth_commit g9_auth_tree g9_auth_parent g9_auth_subject g9_auth_contract_sha256 g9_auth_handoff_sha256
```

`candidate_parent` is exact `G9_AUTH`. No `a6_*` synonym is authorized.
Runner, validators, design, and schema are finalized before constructor so no
constructor self-hash is introduced. The patch is a five-section Git
full-index text patch directly from v3 to v7.

### Exact no-scratch static-review schema

Only for v7 static reviews, this amendment supersedes Amendment 6's eight-
field v4 review schema, its `removed-after-pass` value, and its create,
inventory, and remove lifecycle. For review number `N` in `1,2`, `G3`
contains exactly these eight fields:

```text
goal_g_amendment_9_v7_review_N_result
goal_g_amendment_9_v7_review_N_reviewer
goal_g_amendment_9_v7_review_N_session
goal_g_amendment_9_v7_review_N_implementation_sha256
goal_g_amendment_9_v7_review_N_scratch_state
goal_g_amendment_9_v7_review_N_v3_forensic_inventory_sha256
goal_g_amendment_9_v7_review_N_v7_forensic_inventory_sha256
goal_g_amendment_9_v7_review_N_patch_sha256
```

Both results are `pass`; reviewers, sessions, and implementation hashes are
nonempty and pairwise distinct; each implementation hash binds one exact,
separately reviewed child-free program; `scratch_state` is exactly
`absent-not-created`; both v3 hashes, both v7 hashes, and both patch hashes
agree with the frozen inputs. Any `scratch_final_inventory_sha256` field,
Amendment 6/8 synonym, missing, duplicate, malformed, or extra Amendment 9
v7-review field is forbidden. `verify_bundle`, activation validation,
design/schema provenance, and evidence emission consume exactly this schema.

Each independent reviewer runs as one freshly preflighted Python child whose
implementation launches no descendant. It proves its exact reserved scratch
path absent before and after the review, without creating that path. In
memory, it independently reproduces the complete direct v3-to-v7 patch,
allowed-edit proof, component manifest, forensic inventory, exact bootstrap
function/call/target counts, unchanged runtime wrappers, and every functional
anchor. Any failure stops and preserves all existing bytes.

## Preview, official construction, and activation

The retained no-Cargo bootstrap check remains mandatory before the single
preview invocation. The exact preview vector is:

```text
/bin/busybox
sh
/var/tmp/reap-g3-draft-v7/construct-self-test.preview.sh
preview
/home/ubuntu/code/reap/target/tmp/goal-g-amendment-3-preview-v6
argv_count=5
argv_nul_bytes=145
argv_nul_sha256=99fc7cb73e9d6f592c44342ceb3638c0c0e29908fa2adfb3347121024fc211c9
```

The A9-reviewed constructor has the exact repaired bootstrap and retains the
existing runtime per-child wrapper. The outer invocation also has its own
one-child envelope. Two distinct post-preview reviewers pass before fresh
official construction; two distinct official reviewers pass before sealing.
The official bundle, evidence, and runtime roots retain their existing names.

The exact fresh official-construction vector is:

```text
/bin/busybox
sh
/var/tmp/reap-g3-draft-v7/construct-self-test.preview.sh
official-construct
argv_count=4
argv_nul_bytes=92
argv_nul_sha256=9b21cf81291eff79e2f1f0a7bea39d812de5d24977fda8a1cd8ae6b894939285
```

Official sealing retains Amendment 6's complete fourteen review-binding
arguments; its final complete argv digest is frozen and independently checked
before the one authorized sealing invocation.

If every gate passes, `G3` is the direct child of exact `G9_AUTH`, changes
only the handoff, and uses subject:

```text
docs: activate goal g amendment 3
```

Only these statuses change:

```text
goal_g_amendment_3_status: activation-stopped-inactive -> active-phase0
goal_g_amendment_9_status: authorized-inactive -> activation-complete-phase0-active
```

Amendments 5, 6, 7, and 8 remain stopped. `P0` remains the direct child of
`G3`, changes only the handoff, and uses
`docs: qualify goal g amendment 3 phase 0`. Only after valid `G3` may Cargo
become available.

## Failure and safety

Any nonzero gate, copy, construction, patch, review, preview, official,
sealing, activation, or one-child-boundary violation stops. Preserve every
created byte. Do not retry or repair in place.

When storage permits, a stop commit is the direct child of `G9_AUTH`, changes
only the handoff, replaces the one Amendment 9 status with
`activation-stopped-inactive`, appends one terminal block, and uses subject:

```text
docs: record goal g amendment 9 activation stop
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
push_authorized=false
```
