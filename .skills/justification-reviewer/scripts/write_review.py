#!/usr/bin/env python3
"""Write a reviewer result without asking the agent to serialize JSON."""

import json
import os
from pathlib import Path
import sys
import tempfile

GRADES = {"excellent", "good", "average", "poor"}
SEVERITIES = {"blocking", "should-fix", "nit"}
FIELDS = {"commit", "severity", "file", "line", "description", "suggestion"}


def fail(message):
    raise SystemExit(f"write_review.py: {message}")


def value(args, index, name):
    token = args[index]
    prefix = f"--{name}="
    if token.startswith(prefix):
        return token[len(prefix):], index + 1
    if token == f"--{name}" and index + 1 < len(args):
        return args[index + 1], index + 2
    return None, index


def parse(args):
    grade = None
    workflow = False
    issues = []
    current = None
    index = 0
    while index < len(args):
        token = args[index]
        if token == "--workflow":
            workflow = True
            index += 1
            continue
        if token == "--issue":
            current = {}
            issues.append(current)
            index += 1
            continue
        matched = False
        for name in ["grade", *FIELDS]:
            item, next_index = value(args, index, name)
            if next_index == index:
                continue
            matched = True
            index = next_index
            if name == "grade":
                grade = item
            elif current is None:
                fail(f"--{name} requires a preceding --issue")
            else:
                current[name] = item
            break
        if not matched:
            fail(f"unknown or incomplete argument: {token}")
    if grade not in GRADES:
        fail(f"--grade must be one of: {', '.join(sorted(GRADES))}")
    required = FIELDS
    for number, issue in enumerate(issues, 1):
        missing = sorted(required - issue.keys())
        if missing:
            fail(f"issue {number} is missing: {', '.join('--' + item for item in missing)}")
        try:
            issue["line"] = int(issue["line"])
        except ValueError:
            fail(f"issue {number} --line must be an integer")
        if issue["line"] < 0:
            fail(f"issue {number} --line must be non-negative")
        if issue["severity"] not in SEVERITIES:
            fail(f"issue {number} --severity must be one of: {', '.join(sorted(SEVERITIES))}")
    return grade, workflow, issues


def workflow_comment(issue):
    return (
        f"[{issue['severity']}] commit {issue['commit']}; file {issue['file']}; "
        f"line {issue['line']}; description: {issue['description']}; suggestion: {issue['suggestion']}"
    )


def output_path():
    configured = os.environ.get("SCSH_RESULT")
    if configured:
        return Path(configured)
    skill = Path(__file__).resolve().parent.parent.name
    return Path("tmp") / f"code-review-{skill}.json"


def write_atomic(path, document):
    path.parent.mkdir(parents=True, exist_ok=True)
    handle, temporary = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent, text=True)
    try:
        with os.fdopen(handle, "w", encoding="utf-8") as stream:
            json.dump(document, stream, ensure_ascii=False, indent=2)
            stream.write("\n")
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    except BaseException:
        try:
            os.unlink(temporary)
        except FileNotFoundError:
            pass
        raise


def main():
    grade, workflow, issues = parse(sys.argv[1:])
    if workflow:
        document = {"grade": grade, "comments": [workflow_comment(issue) for issue in issues]}
    else:
        document = {"result": {"grade": grade, "issues_found": len(issues)}, "issues": issues}
    write_atomic(output_path(), document)


if __name__ == "__main__":
    main()
