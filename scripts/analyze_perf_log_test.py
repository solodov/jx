"""Regression coverage for sync diagnostics in the perf-log analyzer."""

import contextlib
import importlib.util
import io
import json
import tempfile
import unittest
from datetime import datetime, timedelta, timezone
from pathlib import Path

SPEC = importlib.util.spec_from_file_location(
    "analyze_perf_log", Path(__file__).with_name("analyze-perf-log.py")
)
analyzer = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(analyzer)


class SyncDiagnosticsTest(unittest.TestCase):
    def test_diagnostics_survive_timing_limit_and_follow_execution_order(self):
        start = datetime(2026, 10, 1, tzinfo=timezone.utc)
        fetch = record("jj.fetch_origin", start, 2)
        fetch["rebase_strategy"] = "always"
        fetch["steps"] = [
            {
                "name": "git_fetch", "duration_us": 900,
            },
            {
                "name": "rebase_decision", "diagnostic": True,
                "recorded_at": (start + timedelta(seconds=1)).isoformat(),
                "change": "root", "decision": "replay", "duration_us": 0,
            },
            {
                "name": "rebase_result", "diagnostic": True,
                "recorded_at": (start + timedelta(seconds=3)).isoformat(),
                "change": "root", "conflict_before": False,
                "conflict_after": True, "duration_us": 0,
            },
        ]
        update = record("github.update_pull_request", start + timedelta(seconds=2), 3)
        update.update(number=42, update_base=True, requested_base="main", base="main")
        command = record("command.run", start, 4)
        command["duration_us"] = 10_000_000
        command["_end"] = start + timedelta(seconds=10)
        output = capture(analyzer.print_window, [fetch, update, command], command, 1)
        diagnostics, timings = output.split("aggregate by op", 1)
        self.assertLess(diagnostics.index("rebase_decision"), diagnostics.index("github.update_pull_request"))
        self.assertLess(diagnostics.index("github.update_pull_request"), diagnostics.index("rebase_result"))
        self.assertIn("requested_base=main", diagnostics)
        self.assertIn("conflict_after=True", diagnostics)
        self.assertIn("git_fetch", timings)
        self.assertNotIn("rebase_decision", timings)
        self.assertNotIn("rebase_result", timings)

    def test_other_process_is_excluded_from_command_window(self):
        start = datetime(2026, 10, 1, tzinfo=timezone.utc)
        command = record("command.run", start, 1)
        command["_end"] = start + timedelta(seconds=10)
        other = record("github.update_pull_request", start + timedelta(seconds=1), 2)
        other.update(pid=999, requested_base="wrong-process")
        output = capture(analyzer.print_window, [command, other], command, 30)
        self.assertNotIn("wrong-process", output)
        self.assertIn("nested records: 0", output)

    def test_legacy_timing_steps_still_render(self):
        start = datetime(2026, 10, 1, tzinfo=timezone.utc)
        legacy = record("jj.fetch_origin", start, 1)
        legacy["steps"] = [{"name": "rebase_trunk_children", "duration_us": 10}]
        output = capture(analyzer.print_step_spans, [legacy], 30)
        self.assertIn("rebase_trunk_children", output)

    def test_diagnostics_without_timestamps_preserve_stored_order(self):
        start = datetime(2026, 10, 1, tzinfo=timezone.utc)
        fetch = record("jj.fetch_origin", start, 1)
        fetch["steps"] = [
            {"name": "first_decision", "diagnostic": True},
            {"name": "second_decision", "diagnostic": True},
        ]
        output = capture(analyzer.print_sync_diagnostics, [fetch])
        self.assertLess(output.index("first_decision"), output.index("second_decision"))

    def test_new_and_existing_pr_fields_are_visible_without_duplicates(self):
        output = analyzer.event_extras({
            "number": 42, "merged": True, "base": "main",
            "update_base": False, "force_rebase": True,
        })
        self.assertIn("merged=True", output)
        self.assertIn("base=main", output)
        self.assertIn("update_base=False", output)
        self.assertIn("force_rebase=True", output)
        self.assertEqual(output.count("number=42"), 1)

    def test_jsonl_diagnostics_can_be_read_with_malformed_lines(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "perf.log"
            path.write_text("not json\n" + json.dumps({
                "op": "jj.fetch_origin", "started_at": "2026-10-01T00:00:00Z",
                "steps": [{"name": "rebase_result", "diagnostic": True, "outcome": "abandoned"}],
            }) + "\n", encoding="utf-8")
            records, skipped = analyzer.read_records(path)
        self.assertEqual(skipped, 1)
        self.assertIn("outcome=abandoned", capture(analyzer.print_sync_diagnostics, records))


def record(op, start, lineno):
    return {
        "op": op, "started_at": start.isoformat(), "_start": start,
        "_end": start, "_lineno": lineno, "pid": 123, "duration_us": 0,
    }


def capture(function, *args):
    output = io.StringIO()
    with contextlib.redirect_stdout(output):
        function(*args)
    return output.getvalue()


if __name__ == "__main__":
    unittest.main()
