#!/usr/bin/env python3
"""
pr.py — consolidated PR cycle helper.

Usage:
  pr.py PR_NUMBER COMMAND [COMMAND ...] [options]

Commands (executed in the order given):
  comments     Fetch and display all inline PR review comments (new vs pre-existing)
  threads      List all review threads with resolution status
  resolve      Resolve all open review threads
  rerequest    Re-request Copilot review
  minimize     Minimize (hide) all eligible pre-push comments as "resolved"
  ci           Run fmt/clippy/test (reqwest + ureq) and a README drift check
  readme       Regenerate README from src/lib.rs if that file changed vs origin/master
  pushpreview  Print the git log + diff --stat push preamble for the current branch
  diff         Print git diff origin/master (for handing to reviewer agents)

Options:
  --owner OWNER    GitHub owner (auto-detected from `gh repo view`)
  --repo REPO      GitHub repo  (auto-detected from `gh repo view`)
  --branch BRANCH  Branch name for pushpreview (auto-detected if omitted)
  --since TS       ISO timestamp; comments after this are 'new' (default: last push)
  --dry-run        For 'resolve'/'minimize': list without mutating

Examples:
  pr.py 181 comments
  pr.py 181 threads
  pr.py 181 resolve rerequest minimize
  pr.py 181 comments threads resolve rerequest minimize
  pr.py 181 ci
  pr.py 181 readme
  pr.py 181 pushpreview
  pr.py 181 diff

The `ci`, `readme`, `pushpreview`, and `diff` commands do not use the PR number — pass 0
if you have no PR handy (e.g. `pr.py 0 ci`).
"""

import argparse
import json
import subprocess
import sys
from datetime import datetime, timezone

COMMANDS = ("comments", "threads", "resolve", "rerequest", "minimize",
            "ci", "readme", "pushpreview", "diff")

# The full optional-feature set (archives + compression + signatures + s3 auth).
# The http client is selected separately: reqwest (default) vs ureq.
_FULL_FEATURES = ("archive-tar archive-zip compression-flate2 compression-zip-deflate "
                  "compression-zip-bzip2 signatures s3-auth checksums")
_UREQ_FEATURES = "ureq default-tls " + _FULL_FEATURES
# Async is reqwest-only; it adds the `*_async` API on top of the full reqwest feature set.
_ASYNC_FEATURES = "async " + _FULL_FEATURES


# ---------------------------------------------------------------------------
# Shared helpers
# ---------------------------------------------------------------------------

def gh(*args, check=True, **kwargs):
    return subprocess.run(["gh", *args], capture_output=True, text=True, check=check, **kwargs)


def repo_info():
    r = gh("repo", "view", "--json", "owner,name", check=False)
    if r.returncode != 0:
        sys.exit("Cannot auto-detect owner/repo. Pass --owner and --repo explicitly.")
    d = json.loads(r.stdout)
    return d["owner"]["login"], d["name"]


def graphql(query):
    return gh("api", "graphql", "-f", f"query={query}")


def parse_utc(ts):
    # GitHub emits UTC with a trailing "Z" (e.g. "2024-01-02T03:04:05Z"); older
    # fromisoformat() rejects the "Z", so normalize it to an explicit +00:00.
    # Any genuine offset is then converted to UTC rather than silently discarded.
    if ts.endswith("Z"):
        ts = ts[:-1] + "+00:00"
    dt = datetime.fromisoformat(ts)
    return dt.astimezone(timezone.utc) if dt.tzinfo else dt.replace(tzinfo=timezone.utc)


def last_push_ts(owner, repo, pr):
    """Timestamp of the PR's most recent commit — the 'last push' cutoff used to
    split comments into new vs pre-existing. Paginate WITHOUT `--jq`: a per-page
    `--jq '[-1]…'` filter emits one line per page and yields a multi-line string
    (then breaks parse_utc) once the commit history exceeds one page. Index the
    merged array in Python instead. Returns None if it can't be determined."""
    r = gh("api", f"repos/{owner}/{repo}/pulls/{pr}/commits", "--paginate", check=False)
    if r.returncode != 0 or not r.stdout.strip():
        return None
    commits = json.loads(r.stdout)
    return commits[-1]["commit"]["committer"]["date"] if commits else None


def section(title):
    print(f"\n{'=' * 3} {title} {'=' * (max(0, 60 - len(title)))}")


# ---------------------------------------------------------------------------
# comments
# ---------------------------------------------------------------------------

def cmd_comments(pr, owner, repo, since=None):
    section(f"PR #{pr} inline comments")

    r = gh("api", f"repos/{owner}/{repo}/pulls/{pr}/comments", "--paginate")
    comments = json.loads(r.stdout)

    if since is None:
        since = last_push_ts(owner, repo, pr)

    since_dt = parse_utc(since) if since else None

    new_comments, old_comments = [], []
    for c in comments:
        if since_dt and parse_utc(c["created_at"]) > since_dt:
            new_comments.append(c)
        else:
            old_comments.append(c)

    print(f"Total inline comments: {len(comments)}")
    if since_dt:
        print(f"  New (after {since}): {len(new_comments)}")
        print(f"  Pre-existing: {len(old_comments)}")
    print()

    def show(c, label):
        line = c.get("line") or c.get("original_line") or "?"
        print(f"[{label}] id={c['id']}  {c['created_at']}  {c['path']}:{line}  @{c['user']['login']}")
        body = c["body"].replace("\n", " ").strip()
        if len(body) > 320:
            body = body[:317] + "..."
        print(f"  {body}")
        print()

    if new_comments:
        print("=== NEW COMMENTS ===")
        for c in new_comments:
            show(c, "NEW")

    if old_comments:
        print("=== PRE-EXISTING COMMENTS ===")
        for c in old_comments:
            show(c, "OLD")


# ---------------------------------------------------------------------------
# threads
# ---------------------------------------------------------------------------

_THREADS_QUERY = """\
{{
  repository(owner: "{owner}", name: "{repo}") {{
    pullRequest(number: {pr}) {{
      reviewThreads(first: 100) {{
        nodes {{
          id
          isResolved
          comments(first: 1) {{
            nodes {{
              databaseId
              createdAt
              body
            }}
          }}
        }}
      }}
    }}
  }}
}}"""


def fetch_threads(owner, repo, pr):
    r = graphql(_THREADS_QUERY.format(owner=owner, repo=repo, pr=pr))
    nodes = json.loads(r.stdout)["data"]["repository"]["pullRequest"]["reviewThreads"]["nodes"]
    threads = []
    for n in nodes:
        first = n["comments"]["nodes"][0] if n["comments"]["nodes"] else {}
        threads.append({
            "id": n["id"],
            "isResolved": n["isResolved"],
            "created_at": first.get("createdAt", ""),
            "database_id": first.get("databaseId"),
            "body_preview": first.get("body", "")[:200],
        })
    return threads


def cmd_threads(pr, owner, repo):
    section(f"PR #{pr} review threads")
    threads = fetch_threads(owner, repo, pr)
    unresolved = sum(1 for t in threads if not t["isResolved"])
    print(f"Total: {len(threads)}  Unresolved: {unresolved}")
    print(json.dumps(threads, indent=2))


# ---------------------------------------------------------------------------
# resolve
# ---------------------------------------------------------------------------

_RESOLVE_MUTATION = """\
mutation {{
  resolveReviewThread(input: {{threadId: "{thread_id}"}}) {{
    thread {{ id isResolved }}
  }}
}}"""


def cmd_resolve(pr, owner, repo, dry_run=False):
    section(f"PR #{pr} resolve threads")
    threads = fetch_threads(owner, repo, pr)
    unresolved = [t["id"] for t in threads if not t["isResolved"]]
    print(f"Found {len(unresolved)} unresolved thread(s).")

    if not unresolved:
        print("Nothing to do.")
        return

    if dry_run:
        print("Dry run — would resolve:")
        for tid in unresolved:
            print(f"  {tid}")
        return

    resolved = 0
    for tid in unresolved:
        print(f"  Resolving {tid} ...", end=" ", flush=True)
        r = graphql(_RESOLVE_MUTATION.format(thread_id=tid))
        if r.returncode == 0:
            print("ok")
            resolved += 1
        else:
            print(f"FAILED: {r.stderr.strip()}")

    print(f"\nResolved {resolved}/{len(unresolved)} thread(s).")
    if resolved < len(unresolved):
        sys.exit(1)


# ---------------------------------------------------------------------------
# minimize
# ---------------------------------------------------------------------------

_MINIMIZE_MUTATION = """\
mutation {{
  minimizeComment(input: {{subjectId: "{node_id}", classifier: RESOLVED}}) {{
    minimizedComment {{
      isMinimized
      minimizedReason
    }}
  }}
}}"""


def cmd_minimize(pr, owner, repo, since=None, dry_run=False):
    section(f"PR #{pr} minimize comments")

    # Determine cutoff: only minimize comments created at or before this
    # timestamp, which defaults to the last push (same boundary cmd_comments
    # uses for "pre-existing"). This prevents accidentally hiding comment
    # threads that were opened after the cycle started.
    if since is None:
        since = last_push_ts(owner, repo, pr)

    since_dt = parse_utc(since) if since else None
    if since_dt:
        print(f"Cutoff: {since} (comments after this timestamp are skipped)")

    r = gh("api", f"repos/{owner}/{repo}/pulls/{pr}/comments", "--paginate")
    review_comments = json.loads(r.stdout)

    r2 = gh("api", f"repos/{owner}/{repo}/issues/{pr}/comments", "--paginate")
    issue_comments = json.loads(r2.stdout)

    eligible = [
        c for c in review_comments + issue_comments
        if since_dt is None or parse_utc(c["created_at"]) <= since_dt
    ]
    print(f"Found {len(eligible)} eligible comment(s) to minimize.")

    if dry_run:
        for c in eligible:
            print(f"  Would minimize: {c['node_id']}  @{c['user']['login']}  {c.get('path', '<issue comment>')}  {c['created_at']}")
        return

    minimized = 0
    for c in eligible:
        print(f"  Minimizing {c['node_id']} (@{c['user']['login']}) ...", end=" ", flush=True)
        r = graphql(_MINIMIZE_MUTATION.format(node_id=c["node_id"]))
        if r.returncode == 0:
            print("ok")
            minimized += 1
        else:
            print(f"FAILED: {r.stderr.strip()}")

    print(f"\nMinimized {minimized}/{len(eligible)} comment(s).")
    if minimized < len(eligible):
        sys.exit(1)


# ---------------------------------------------------------------------------
# rerequest
# ---------------------------------------------------------------------------

def cmd_rerequest(pr, owner, repo):
    section(f"PR #{pr} re-request Copilot review")
    gh("api", f"repos/{owner}/{repo}/pulls/{pr}/requested_reviewers",
       "-X", "POST", "-f", "reviewers[]=copilot-pull-request-reviewer[bot]")
    print(f"Copilot review re-requested for {owner}/{repo}#{pr}.")


# ---------------------------------------------------------------------------
# ci
# ---------------------------------------------------------------------------

def _run_step(label, argv):
    print(f"\n--- {label} ---")
    print("  $ " + " ".join(argv))
    r = subprocess.run(argv, capture_output=True, text=True)
    if r.returncode == 0:
        print(f"  OK: {label}")
        return True
    print(f"  FAILED ({r.returncode}): {label}")
    tail = (r.stdout + r.stderr).strip().splitlines()
    for line in tail[-40:]:
        print(f"    {line}")
    return False


def cmd_ci():
    section("CI")
    steps = [
        ("fmt", ["cargo", "fmt", "--check"]),
        ("clippy (reqwest)", ["cargo", "clippy", "--all-targets", "--features", _FULL_FEATURES]),
        ("clippy (ureq)", ["cargo", "clippy", "--all-targets", "--no-default-features",
                           "--features", _UREQ_FEATURES]),
        ("clippy (async)", ["cargo", "clippy", "--all-targets", "--features", _ASYNC_FEATURES]),
        ("test (reqwest)", ["cargo", "test", "--features", _FULL_FEATURES]),
        ("test (ureq)", ["cargo", "test", "--no-default-features", "--features", _UREQ_FEATURES]),
        ("test (async)", ["cargo", "test", "--features", _ASYNC_FEATURES]),
    ]

    failures = [label for (label, argv) in steps if not _run_step(label, argv)]

    # README drift check
    print("\n--- readme (drift check) ---")
    r = subprocess.run(["cargo", "readme", "--no-indent-headings"],
                       capture_output=True, text=True)
    if r.returncode != 0:
        print("  FAILED: cargo readme errored")
        print("   ", r.stderr.strip()[:400])
        failures.append("readme")
    else:
        try:
            with open("README.md") as f:
                current = f.read()
        except OSError:
            current = None
        if current is None or current != r.stdout:
            print("  FAILED: README.md is out of sync with src/lib.rs")
            print("    run `.agents/skills/pr-cycle/pr.py 0 readme` (or `./readme.sh`) to regenerate")
            failures.append("readme")
        else:
            print("  OK: README in sync")

    print()
    if failures:
        print(f"CI FAILED — {len(failures)} step(s): {', '.join(failures)}")
        sys.exit(1)
    print("CI passed (all steps green).")


# ---------------------------------------------------------------------------
# readme
# ---------------------------------------------------------------------------

def cmd_readme():
    section("README regeneration")
    r = subprocess.run(
        ["git", "diff", "--name-only", "origin/master", "--", "src/lib.rs"],
        capture_output=True,
        text=True,
    )
    if not r.stdout.strip():
        print("src/lib.rs is unchanged vs origin/master — README regeneration skipped.")
        return

    print("src/lib.rs changed — regenerating README.md …")
    r2 = subprocess.run(
        ["cargo", "readme", "--no-indent-headings"],
        capture_output=True,
        text=True,
    )
    if r2.returncode != 0:
        print(f"cargo readme failed (exit {r2.returncode}):")
        print(r2.stderr)
        sys.exit(r2.returncode)

    with open("README.md", "w") as f:
        f.write(r2.stdout)
    print("README.md regenerated successfully.")


# ---------------------------------------------------------------------------
# pushpreview
# ---------------------------------------------------------------------------

def _current_branch():
    r = subprocess.run(
        ["git", "rev-parse", "--abbrev-ref", "HEAD"],
        capture_output=True, text=True, check=True,
    )
    return r.stdout.strip()


def cmd_pushpreview(branch=None):
    section("Push preview")
    if branch is None:
        branch = _current_branch()
    origin_ref = f"origin/{branch}"

    print(f"Branch: {branch}  →  {origin_ref}\n")

    r1 = subprocess.run(
        ["git", "log", f"{origin_ref}..HEAD", "--oneline"],
        capture_output=True, text=True,
    )
    if r1.stdout.strip():
        print("Commits to push:")
        for line in r1.stdout.strip().splitlines():
            print(f"  {line}")
    else:
        print("No commits ahead of remote (or remote branch does not exist yet).")

    print()
    r2 = subprocess.run(
        ["git", "diff", f"{origin_ref}", "--stat"],
        capture_output=True, text=True,
    )
    if r2.stdout.strip():
        print("Files changed:")
        print(r2.stdout.rstrip())
    else:
        print("No file changes vs remote.")


# ---------------------------------------------------------------------------
# diff
# ---------------------------------------------------------------------------

def cmd_diff():
    section("diff origin/master")
    r = subprocess.run(
        ["git", "diff", "origin/master"],
        capture_output=True, text=True,
    )
    print(r.stdout)
    if r.stderr:
        print(r.stderr, file=sys.stderr)


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def main():
    ap = argparse.ArgumentParser(
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    ap.add_argument("pr", type=int, help="PR number (pass 0 for ci/readme/pushpreview/diff)")
    ap.add_argument("commands", nargs="+", choices=COMMANDS,
                    metavar="COMMAND",
                    help=f"One or more of: {', '.join(COMMANDS)}")
    ap.add_argument("--owner")
    ap.add_argument("--repo")
    ap.add_argument("--branch", help="Branch name (for pushpreview; auto-detected if omitted)")
    ap.add_argument("--since", help="ISO timestamp for 'comments'/'minimize' new/old split")
    ap.add_argument("--dry-run", action="store_true", help="For 'resolve'/'minimize': list without mutating")
    args = ap.parse_args()

    # Commands that need the GitHub owner/repo (REST or GraphQL API).
    _GH_COMMANDS = {"comments", "threads", "resolve", "rerequest", "minimize"}
    # Fetch owner/repo lazily — only if at least one command needs it.
    owner = repo = None
    if any(cmd in _GH_COMMANDS for cmd in args.commands):
        owner, repo = args.owner, args.repo
        if not owner or not repo:
            owner, repo = repo_info()

    for cmd in args.commands:
        if cmd == "comments":
            cmd_comments(args.pr, owner, repo, since=args.since)
        elif cmd == "threads":
            cmd_threads(args.pr, owner, repo)
        elif cmd == "resolve":
            cmd_resolve(args.pr, owner, repo, dry_run=args.dry_run)
        elif cmd == "rerequest":
            cmd_rerequest(args.pr, owner, repo)
        elif cmd == "minimize":
            cmd_minimize(args.pr, owner, repo, since=args.since, dry_run=args.dry_run)
        elif cmd == "ci":
            cmd_ci()
        elif cmd == "readme":
            cmd_readme()
        elif cmd == "pushpreview":
            cmd_pushpreview(branch=args.branch)
        elif cmd == "diff":
            cmd_diff()


if __name__ == "__main__":
    main()
