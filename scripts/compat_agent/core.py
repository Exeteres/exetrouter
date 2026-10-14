"""Trusted API, state and policy boundaries. No payload logging."""
import base64
import copy
import hashlib
import json
import re
import urllib.error
import urllib.parse
import urllib.request
from pathlib import PurePosixPath

STATE_BRANCH = "automation/upstream-state"
MODEL = "gpt-6.1-sol"
MAX_BYTES = 4 * 1024 * 1024
MAX_BATCH_FILES = 32
TRACKS = {
    "codex": ("openai/codex", "@openai/codex", "rust-v"),
    "opencode": ("anomalyco/opencode", "@opencode/cli", "v"),
    "opencode-v1": ("anomalyco/opencode", "opencode-ai", "v"),
}
DECISIONS = {
    "triage": {"no_change", "docs_only", "code_change", "needs_human"},
    "verify": {"confirmed", "rejected", "needs_human"},
    "implement": {"ready", "needs_human"},
    "review": {"approve", "request_changes", "needs_human"},
}


class Stop(RuntimeError):
    """Fixed, content-free failure category."""


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        raise Stop("redirect_rejected")


def request(url, token=None, method="GET", body=None, limit=MAX_BYTES, timeout=900):
    headers = {"User-Agent": "exetrouter-compat-agent", "Accept": "application/json"}
    if token:
        headers["Authorization"] = "Bearer " + token
    data = None
    if body is not None:
        data = json.dumps(body).encode()
        if len(data) > 16 * 1024 * 1024:
            raise Stop("request_too_large")
        headers["Content-Type"] = "application/json"
    req = urllib.request.Request(url, data=data, headers=headers, method=method)
    try:
        with urllib.request.build_opener(NoRedirect).open(req, timeout=timeout) as resp:
            raw = resp.read(limit + 1)
        if len(raw) > limit:
            raise Stop("response_too_large")
        return json.loads(raw)
    except urllib.error.HTTPError as error:
        # Do not read/echo credential-bearing or arbitrary error payloads.
        raise Stop("http_" + str(error.code)) from None
    except (OSError, ValueError):
        raise Stop("transport_or_json_error") from None


def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True).encode()).hexdigest()


def source_path(path):
    if not isinstance(path, str) or not path or len(path) > 500:
        raise Stop("invalid_path")
    p = PurePosixPath(path)
    if p.is_absolute() or ".." in p.parts or "\\" in path or str(p) != path:
        raise Stop("invalid_path")
    if any(ord(c) < 32 for c in path):
        raise Stop("invalid_path")
    return path


def writable(path):
    source_path(path)
    return (
        path.startswith(("src/", "tests/", "docs/"))
        and not path.startswith(("src/migrations/", "docs/assets/"))
        and path.endswith((".rs", ".md", ".json"))
        and PurePosixPath(path).name not in {"AGENTS.md", "CLAUDE.md", "SECURITY.md"}
        and path not in {"docs/protocol-sources.json", "docs/compat-agent.md"}
    )


def decision(phase, value):
    if not isinstance(value, dict) or value.get("decision") not in DECISIONS[phase]:
        raise Stop("invalid_decision")
    coverage = value.get("coverage_complete")
    if type(coverage) is not bool or (not coverage and value["decision"] != "needs_human"):
        raise Stop("incomplete_coverage")
    if not isinstance(value.get("summary"), str) or len(value["summary"]) > 6000:
        raise Stop("invalid_summary")
    findings = value.get("findings")
    if not isinstance(findings, list) or len(findings) > 20:
        raise Stop("invalid_findings")
    if phase in {"triage", "verify"} and value["decision"] in {
        "docs_only", "code_change", "confirmed"
    } and not findings:
        raise Stop("missing_evidence")
    for finding in findings:
        if not isinstance(finding, dict):
            raise Stop("invalid_finding")
        for field in ("problem", "fix", "test"):
            if not isinstance(finding.get(field), str) or not finding[field]:
                raise Stop("invalid_finding")
        for field in ("upstream", "exetrouter"):
            citations = finding.get(field)
            if not isinstance(citations, list) or not citations:
                raise Stop("missing_citation")
            for cite in citations:
                if not isinstance(cite, dict) or not re.fullmatch(r"[0-9a-f]{40}", cite.get("sha", "")):
                    raise Stop("invalid_citation")
                source_path(cite.get("path"))
                if not isinstance(cite.get("symbol"), str) or not cite["symbol"]:
                    raise Stop("invalid_citation")
    if len(json.dumps(value)) > 64000:
        raise Stop("decision_too_large")
    return value


class GitHub:
    def __init__(self, repo, token):
        if not re.fullmatch(r"[\w.-]+/[\w.-]+", repo):
            raise Stop("invalid_repository")
        self.repo, self.token = repo, token

    def api(self, path, method="GET", body=None):
        return request("https://api.github.com/" + path, self.token, method, body)

    def own(self, path, method="GET", body=None):
        return self.api("repos/" + self.repo + "/" + path, method, body)

    def file(self, repo, sha, path):
        source_path(path)
        if not re.fullmatch(r"[0-9a-f]{40}", sha):
            raise Stop("invalid_revision")
        data = self.api("repos/" + repo + "/contents/" + urllib.parse.quote(path) + "?ref=" + sha)
        if data.get("type") != "file" or data.get("encoding") != "base64":
            raise Stop("unsupported_source_file")
        raw = base64.b64decode(data["content"], validate=False)
        if len(raw) > MAX_BYTES:
            raise Stop("source_too_large")
        return raw.decode("utf-8")

    def resolve(self, repo, ref):
        data = self.api("repos/" + repo + "/commits/" + urllib.parse.quote(ref, safe=""))
        sha = data["sha"]
        if not re.fullmatch(r"[0-9a-f]{40}", sha):
            raise Stop("invalid_revision")
        return sha

    def tree(self, repo, sha):
        if not re.fullmatch(r"[0-9a-f]{40}", sha):
            raise Stop("invalid_revision")
        commit = self.api("repos/" + repo + "/git/commits/" + sha)
        data = self.api("repos/" + repo + "/git/trees/" + commit["tree"]["sha"] + "?recursive=1")
        if data.get("truncated") is not False or not isinstance(data.get("tree"), list):
            raise Stop("source_tree_incomplete")
        if len(data["tree"]) > 50000:
            raise Stop("source_tree_incomplete")
        result = {}
        for item in data["tree"]:
            path = source_path(item["path"])
            if item["type"] == "tree":
                continue
            if item["type"] not in {"blob", "commit"} or not re.fullmatch(r"[0-9a-f]{40}", item["sha"]):
                raise Stop("source_tree_incomplete")
            if path in result:
                raise Stop("source_tree_incomplete")
            result[path] = {k: item[k] for k in ("sha", "mode", "type")}
        return result

    def prs(self):
        result = []
        for page in range(1, 101):
            batch = self.own(f"pulls?state=all&per_page=100&page={page}")
            result.extend(batch)
            if len(batch) < 100:
                return result
        raise Stop("pr_inventory_incomplete")


class Ledger:
    def __init__(self, gh):
        self.gh = gh
        try:
            self.head = gh.own("git/ref/heads/" + STATE_BRANCH)["object"]["sha"]
            self.value = json.loads(gh.file(gh.repo, self.head, "state.json"))
            if self.value.get("schema") != 1:
                raise Stop("unknown_state_schema")
        except Stop as error:
            if str(error) != "http_404":
                raise
            self.head = None
            self.value = {"schema": 1, "cursors": {}, "attempts": {}}

    def save(self):
        # Fast-forward-only ref update provides compare-and-swap via ancestry.
        blob = self.gh.own("git/blobs", "POST", {
            "content": json.dumps(self.value, sort_keys=True), "encoding": "utf-8"})
        tree = self.gh.own("git/trees", "POST", {"tree": [{
            "path": "state.json", "mode": "100644", "type": "blob", "sha": blob["sha"]}]})
        commit = self.gh.own("git/commits", "POST", {
            "message": "Update compatibility agent state", "tree": tree["sha"],
            "parents": [self.head] if self.head else []})
        if self.head:
            self.gh.own("git/refs/heads/" + STATE_BRANCH, "PATCH", {
                "sha": commit["sha"], "force": False})
        else:
            self.gh.own("git/refs", "POST", {
                "ref": "refs/heads/" + STATE_BRANCH, "sha": commit["sha"]})
        self.head = commit["sha"]

    def attempted(self, event, scan=None):
        for item in self.value["attempts"].values():
            if "scan" in item:
                if scan and item["scan"] == scan["id"] and item["batch"] == scan["next_batch"]:
                    return True
                if (item["sha"] == event["after"] and item["repository"] == event["repository"]
                        and item["track"] != event["track"]
                        and item["status"] in {"published", "started", "uncertain", "blocked"}):
                    return True
            elif (item["sha"] == event["after"]
                  and item.get("repository") in {None, event["repository"]}
                  and item["status"] in {"published", "started", "uncertain", "blocked"}):
                return True
        return False

    def claim(self, key, track, sha, repository=None, scan=None):
        if key in self.value["attempts"]:
            return False
        if len(self.value["attempts"]) >= 10000:
            raise Stop("state_capacity_reached")
        if scan:
            if self.attempted({"after": sha, "repository": repository, "track": track}, scan):
                return False
            scans = self.value.setdefault("scans", {})
            if track in scans and scans[track] != scan:
                raise Stop("scan_changed")
            scans[track] = dict(scan)
        self.value["attempts"][key] = {"status": "started", "track": track, "sha": sha,
                                        "repository": repository}
        if scan:
            self.value["attempts"][key].update(scan=scan["id"], batch=scan["next_batch"])
        self.save()
        return True

    def finish(self, key, status, pr=None):
        previous = copy.deepcopy(self.value)
        try:
            self._finish(key, status, pr)
            self.save()
        except Exception:
            # A failed progress write must not advance the in-memory scan before
            # the controller records an uncertain outcome. Remote CAS still wins.
            self.value = previous
            raise

    def _finish(self, key, status, pr):
        item = self.value["attempts"][key]
        item["status"] = status
        if pr:
            item["pr"] = pr
        if status in {"no_change", "rejected", "published"}:
            if "scan" in item:
                scan = self.value["scans"][item["track"]]
                if scan["id"] != item["scan"] or scan["next_batch"] != item["batch"]:
                    raise Stop("scan_changed")
                scan["next_batch"] += 1
                if scan["next_batch"] == scan["batch_count"]:
                    self.value["cursors"][item["track"]] = item["sha"]
                    del self.value["scans"][item["track"]]
            else:
                self.value["cursors"][item["track"]] = item["sha"]


def review_batch(event, scan=None):
    """Freeze a complete inventory, exposing only one bounded slice to the model."""
    files = event["files"]
    plan = {k: event[k] for k in ("track", "repository", "before", "after", "version")}
    plan.update(inventory=digest(files), file_count=len(files), batch_size=MAX_BATCH_FILES,
                batch_count=max(1, (len(files) + MAX_BATCH_FILES - 1) // MAX_BATCH_FILES))
    plan["id"] = digest(plan)
    index = 0 if scan is None else scan.get("next_batch")
    if (type(index) is not int or not 0 <= index < plan["batch_count"]
            or (scan is not None and scan != {**plan, "next_batch": index})):
        raise Stop("scan_changed")
    selected = []
    for item in files[index * MAX_BATCH_FILES:(index + 1) * MAX_BATCH_FILES]:
        # Blob hashes stay in the inventory digest, not in every inference input.
        compact = {k: item[k] for k in ("filename", "status")}
        for side in ("before", "after"):
            compact[side] = ({k: item[side][k] for k in ("mode", "type")}
                             if item[side] is not None else None)
        selected.append(compact)
    return {**plan, "next_batch": index}, {**event, "files": selected,
        "batch": {"index": index + 1, "count": plan["batch_count"], "total_files": len(files)}}


def collect(gh, manifest, cursors, scans=None):
    baseline = {p["id"]: p for p in manifest["projects"]}
    observations = []
    for track, (repo, package, prefix) in TRACKS.items():
        meta = request("https://registry.npmjs.org/" + package + "/latest", timeout=60)
        version = meta["version"]
        if not re.fullmatch(r"\d+\.\d+\.\d+", version):
            raise Stop("invalid_stable_version")
        observations.append((track, repo, gh.resolve(repo, prefix + version), version))
    for repo in dict.fromkeys(p[0] for p in TRACKS.values()):
        branch = gh.api("repos/" + repo)["default_branch"]
        observations.append((repo + ":branch", repo, gh.resolve(repo, branch), None))
    events = []
    trees = {}
    for track, repo, sha, version in observations:
        fallback = baseline["codex" if repo == "openai/codex" else "opencode"]["commit"]
        if track in baseline:
            fallback = baseline[track]["commit"]
        before = cursors.get(track, fallback)
        scan = (scans or {}).get(track)
        if scan:
            if scan["track"] != track or scan["repository"] != repo or scan["before"] != before:
                raise Stop("scan_changed")
            before, sha, version = scan["before"], scan["after"], scan["version"]
        if before == sha:
            continue
        event = {"track": track, "repository": repo, "before": before,
                 "after": sha, "version": version, "comparison": "exact_source_trees"}
        try:
            # Compare both pinned snapshots, not a merge base. Release branches
            # legitimately diverge; Compare also caps its file inventory at 300.
            for revision in (before, sha):
                if (repo, revision) not in trees:
                    trees[repo, revision] = gh.tree(repo, revision)
            old, new = trees[repo, before], trees[repo, sha]
            files = []
            for path in sorted(old.keys() | new.keys()):
                if old.get(path) == new.get(path):
                    continue
                files.append({"filename": path, "status": "added" if path not in old else
                              "removed" if path not in new else "modified",
                              "before": old.get(path), "after": new.get(path)})
            if len(files) > 5000 or len(json.dumps(files).encode()) > 1024 * 1024:
                raise Stop("change_inventory_too_large")
            event["files"] = files
        except Stop as error:
            if str(error) not in {"source_tree_incomplete", "change_inventory_too_large"}:
                raise
            event["blocked"] = str(error)
        events.append(event)
    return events
