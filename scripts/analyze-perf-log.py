#!/usr/bin/env python3
"""Summarize jx JSONL perf logs around command executions."""

from __future__ import annotations

import argparse
import json
from collections import defaultdict
from datetime import datetime, timedelta
from pathlib import Path
from typing import Any

DEFAULT_LOG_PATH = Path.home() / ".local" / "state" / "jx" / "jx-perf.log"


def main() -> int:
    args = parse_args()
    records = read_records(args.path.expanduser())
    if not records:
        print(f"no perf records found in {args.path}")
        return 1

    print(f"log: {args.path.expanduser()}")
    print(f"records: {len(records)}  lines: {records[-1]['_lineno']}")

    if args.tail:
        print(f"\nlast {args.tail} records:")
        for record in records[-args.tail :]:
            print_event(record)

    commands = matching_commands(records, args.command)
    if not commands:
        if args.command:
            print(f"\nno command.run records matched {args.command!r}")
        else:
            print("\nno command.run records found")
        return 1

    print("\nlatest matching command.run records:")
    for command in commands[-args.latest :]:
        print_command(command)
        for step in command.get("steps", []):
            print_step(step, indent="  ")

    for command in commands[-args.latest :]:
        print_window(records, command, args.top_steps)

    return 0


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Summarize jx JSONL perf logs around command.run spans."
    )
    parser.add_argument(
        "path",
        nargs="?",
        type=Path,
        default=DEFAULT_LOG_PATH,
        help=f"perf log path (default: {DEFAULT_LOG_PATH})",
    )
    parser.add_argument(
        "--command",
        help=(
            "command filter. Matches command, command_path, or command_path prefix "
            "such as sync, stack, or stack.publish"
        ),
    )
    parser.add_argument(
        "--latest",
        type=positive_int,
        default=1,
        help="number of latest matching command windows to analyze (default: 1)",
    )
    parser.add_argument(
        "--tail",
        type=positive_int,
        default=0,
        help="also print the last N raw perf records",
    )
    parser.add_argument(
        "--top-steps",
        type=positive_int,
        default=30,
        help="maximum slow steps to print per span (default: 30)",
    )
    return parser.parse_args()


def positive_int(value: str) -> int:
    parsed = int(value)
    if parsed < 0:
        raise argparse.ArgumentTypeError("value must be non-negative")
    return parsed


def read_records(path: Path) -> list[dict[str, Any]]:
    records: list[dict[str, Any]] = []
    with path.open(encoding="utf-8") as handle:
        for lineno, line in enumerate(handle, 1):
            line = line.strip()
            if not line:
                continue
            try:
                record = json.loads(line)
            except json.JSONDecodeError:
                continue
            record["_lineno"] = lineno
            if started_at := record.get("started_at"):
                start = datetime.fromisoformat(started_at.replace("Z", "+00:00"))
                record["_start"] = start
                record["_end"] = start + timedelta(
                    microseconds=record.get("duration_us", 0)
                )
            records.append(record)
    return records


def matching_commands(
    records: list[dict[str, Any]], command_filter: str | None
) -> list[dict[str, Any]]:
    commands = [record for record in records if record.get("op") == "command.run"]
    if not command_filter:
        return commands
    return [record for record in commands if command_matches(record, command_filter)]


def command_matches(record: dict[str, Any], command_filter: str) -> bool:
    command = str(record.get("command", ""))
    command_path = str(record.get("command_path", ""))
    return (
        command == command_filter
        or command_path == command_filter
        or command_path.startswith(f"{command_filter}.")
    )


def print_window(
    records: list[dict[str, Any]], command: dict[str, Any], top_steps: int
) -> None:
    start = command.get("_start")
    end = command.get("_end")
    if not start or not end:
        return

    nested = [
        record
        for record in records
        if record is not command
        and record.get("_start")
        and start <= record["_start"] <= end
    ]
    print(
        f"\n=== window for command line {command['_lineno']} "
        f"{command.get('started_at', '')} {duration(command)} ==="
    )
    print_command(command)
    print(f"nested records: {len(nested)}")
    print_aggregate(nested)
    print_step_spans(nested, top_steps)


def print_aggregate(records: list[dict[str, Any]]) -> None:
    aggregate: dict[str, dict[str, int]] = defaultdict(
        lambda: {"count": 0, "total": 0, "max": 0, "errors": 0}
    )
    for record in records:
        stats = aggregate[str(record.get("op", ""))]
        stats["count"] += 1
        duration_us = int(record.get("duration_us", 0))
        stats["total"] += duration_us
        stats["max"] = max(stats["max"], duration_us)
        stats["errors"] += int(record.get("status") == "error")

    if not aggregate:
        return

    print("aggregate by op (overlapping totals):")
    for op, stats in sorted(
        aggregate.items(), key=lambda item: item[1]["total"], reverse=True
    ):
        print(
            f"  {op:<48} count={stats['count']:>3} "
            f"total={seconds(stats['total']):>9} max={seconds(stats['max']):>9} "
            f"errs={stats['errors']}"
        )


def print_step_spans(records: list[dict[str, Any]], top_steps: int) -> None:
    spans = sorted(
        [record for record in records if record.get("steps")],
        key=lambda record: record.get("duration_us", 0),
        reverse=True,
    )
    if not spans:
        return

    print("spans with steps:")
    for span in spans:
        print(f"  span line {span['_lineno']} {event_label(span)} {duration(span)}")
        for step in sorted(
            span.get("steps", []), key=lambda item: item.get("duration_us", 0), reverse=True
        )[:top_steps]:
            print_step(step, indent="    ")


def print_command(record: dict[str, Any]) -> None:
    print_event(record)


def print_event(record: dict[str, Any]) -> None:
    extras = event_extras(record)
    suffix = f"  {extras}" if extras else ""
    print(
        f"{record['_lineno']:>5} {record.get('started_at', ''):<28} "
        f"{event_label(record):<48} {str(record.get('status', '')):<5} "
        f"{duration(record):>9}{suffix}"
    )


def print_step(step: dict[str, Any], indent: str) -> None:
    extras = step_extras(step)
    suffix = f"  {extras}" if extras else ""
    print(f"{indent}{step.get('name', ''):<60} {duration(step):>9}{suffix}")


def event_label(record: dict[str, Any]) -> str:
    return str(record.get("op") or "")


def event_extras(record: dict[str, Any]) -> str:
    keys = [
        "command_path",
        "command",
        "mode",
        "repo",
        "exit_code",
        "advance_trunk",
        "tracked_update_count",
        "pushable_update_count",
        "pushed_ref_count",
        "pushed_bookmark_count",
        "unchanged_bookmark_count",
        "pushed_commit_count",
        "bookmark_count",
        "metadata_node_count",
        "synced_pr_count",
        "pull_request_count",
        "cache_hit",
        "number",
        "head_branch",
        "found",
    ]
    return join_attrs(record, keys)


def step_extras(step: dict[str, Any]) -> str:
    keys = [
        "tracked_update_count",
        "pushable_update_count",
        "pushed_ref_count",
        "pushed_bookmark_count",
        "unchanged_bookmark_count",
        "pushed_commit_count",
        "bookmark_count",
        "metadata_node_count",
        "pull_request_count",
        "changed_remote_bookmarks",
        "rebased_commit_count",
        "conflicted_rebased_commit_count",
        "jj_total_us",
        "err",
    ]
    return join_attrs(step, keys)


def join_attrs(record: dict[str, Any], keys: list[str]) -> str:
    attrs = [f"{key}={record[key]}" for key in keys if key in record]
    if err := record.get("err"):
        err = str(err).replace("\n", " ")
        attrs.append(f"err={err[:100]}")
    return ", ".join(attrs)


def duration(record: dict[str, Any]) -> str:
    return seconds(int(record.get("duration_us", 0)))


def seconds(duration_us: int) -> str:
    return f"{duration_us / 1_000_000:.3f}s"


if __name__ == "__main__":
    raise SystemExit(main())
