#!/usr/bin/env python3
"""`session.py executor`'s printed arguments, with `brief.show` stubbed.

Run as `python3 -I tooling/session/test_session.py`; see the model-tiered ADR's 2026-11-15 amendment.
"""

import contextlib
import io
import json
import os
import sys
import unittest

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import session  # noqa: E402

PLANNER = "https://claude.ai/code/session_PLANNER"
ADR = ".claude/decisions/2026-11-13-development-model-tiered-agent-briefs.md"


def brief(model="sonnet", adr=ADR):
    return {
        "id": "b7",
        "planner": PLANNER,
        "revisions": [
            {"draft": {"model": model, "title": "Old"}},
            {"draft": {"model": model, "title": "A step", "adr": adr}},
        ],
    }


class Executor(unittest.TestCase):
    def call(self, argv, answer=None, repository="https://github.com/noisypigeon/pigeon"):
        asked = []

        def run(operation, args, timeout=30):
            asked.append((operation, args))
            return answer

        out, err = io.StringIO(), io.StringIO()
        saved = session.run, session.repository
        session.run, session.repository = run, lambda: repository
        try:
            with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
                code = session.executor(argv)
        finally:
            session.run, session.repository = saved
        return code, out.getvalue(), err.getvalue(), asked

    def test_it_prints_every_argument_create_session_needs(self):
        code, out, _, asked = self.call(["b7"], brief())
        self.assertEqual(code, 0)
        self.assertEqual(asked, [("brief.show", {"id": "b7"})])
        got = json.loads(out)
        self.assertEqual(set(got), {"model", "title", "prompt", "source_url", "tags"})
        self.assertEqual(got["model"], "claude-sonnet-5-5")
        self.assertEqual(got["title"], "pigeon b7 (sonnet): A step")
        self.assertEqual(got["source_url"], "https://github.com/noisypigeon/pigeon")
        self.assertEqual(
            got["tags"],
            ["pigeon:brief-b7", "pigeon:adr-2026-11-13-development-model-tiered-agent-briefs"],
        )

    def test_the_prompt_checks_the_checkout_and_tools_before_anything(self):
        prompt = json.loads(self.call(["b7"], brief())[1])["prompt"]
        guard = prompt.index("0. Check first")
        self.assertLess(guard, prompt.index("1. Per CLAUDE.md"))
        self.assertIn("`send_message` your planner's session, `session_PLANNER`", prompt)
        self.assertIn("Never `add_repo`", prompt)
        self.assertIn("revision 2", prompt)
        self.assertIn(f'`planner` "{PLANNER}"', prompt)

    def test_tags_add_each_tag_once(self):
        got = json.loads(self.call(["b7", "--tag", "pigeon:reliability", "--tag", "pigeon:brief-b7"], brief())[1])
        self.assertEqual(got["tags"][-1], "pigeon:reliability")
        self.assertEqual(got["tags"].count("pigeon:brief-b7"), 1)

    def test_a_brief_without_an_adr_still_has_its_brief_tag(self):
        got = json.loads(self.call(["b7"], brief(adr=None))[1])
        self.assertEqual(got["tags"], ["pigeon:brief-b7"])

    def test_no_repository_prints_nothing_and_asks_nothing(self):
        code, out, err, asked = self.call(["b7"], brief(), repository=None)
        self.assertEqual((code, out, asked), (1, "", []))
        self.assertIn("no checkout", err)

    def test_refusals(self):
        self.assertEqual(self.call([])[0], 2)
        self.assertEqual(self.call(["b7", "--tag"])[0], 2)
        self.assertEqual(self.call(["b7", "extra"])[0], 2)
        self.assertEqual(self.call(["b7"], None)[0], 1)
        self.assertEqual(self.call(["b7"], brief(model="opus"))[0], 1)

    def test_the_repository_is_config_yamls(self):
        root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
        with open(os.path.join(root, "config.yaml")) as f:
            want = next(r.split(":", 1)[1].strip() for r in f if r.startswith("repository:"))
        self.assertEqual(session.repository(), want)


if __name__ == "__main__":
    unittest.main()
