#!/usr/bin/env python3
"""Regressions for lookup output, lexical matching, bounds and rg fallback."""

import importlib.util
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch


SCRIPT = Path(__file__).with_name("where.py")
SPEC = importlib.util.spec_from_file_location("where", SCRIPT)
WHERE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(WHERE)


class LookupTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)

    def write(self, path, text):
        target = self.root / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(text, encoding="utf-8")
        return target

    def test_definitions_first_signatures_lines_substrings_and_kinds(self):
        self.write("crates/example/src/lib.rs", '''// pub fn phantom() {}
pub struct Widget;
pub enum Choice { One }
pub trait Worker {}
pub const LIMIT: usize = 4;
pub async fn widget_load(
    id: usize,
) -> Widget { Widget }
fn usage() { widget_load(1); }
''')
        rows = WHERE.lookup(self.root, "widget")
        self.assertEqual(rows[0][:3], ("crates/example/src/lib.rs", 6, "fn"))
        self.assertEqual(rows[0][3], "pub async fn widget_load( id: usize, ) -> Widget")
        self.assertEqual(rows[-1][1:3], (9, "use"))
        for symbol, kind in (("Widget", "struct"), ("Choice", "enum"),
                             ("Worker", "trait"), ("LIMIT", "const")):
            self.assertEqual(WHERE.lookup(self.root, symbol)[0][2], kind)
        self.assertEqual(WHERE.lookup(self.root, "phantom"), [])

    def test_qualified_generic_inherent_trait_impl_and_module(self):
        self.write("crates/example/src/things.rs", '''
struct Widget<T>(T);
impl<T> Widget<T> {
    pub fn load(&self) {}
}
impl<T> Worker for Widget<T> {
    fn reload(&self) {}
}
impl Other { fn load(&self) {} }
fn load_free() {}
fn usage() { Widget::load(); thing.load(); }
''')
        rows = WHERE.lookup(self.root, "Widget::load")
        self.assertEqual([row[3] for row in rows if row[2] == "fn"],
                         ["pub fn load(&self)", "fn reload(&self)"])
        self.assertTrue(any("thing.load()" in row[3] for row in rows if row[2] == "use"))
        self.assertTrue(any("load_free" in row[3] for row in WHERE.lookup(self.root, "things::load")))

    def test_js_functions_exports_and_comments_literals(self):
        self.write("web/js/example.js", '''// function phantom() {}
const text = "export function phantom() {}";
const regex = /function phantom() {}/;
export async function fetchWidget({ id = 1 } = {}) { return id; }
function localWidget() {}
export const WidgetValue = 1;
fetchWidget();
''')
        rows = WHERE.lookup(self.root, "Widget")
        self.assertEqual([row[2] for row in rows], ["export", "function", "export", "use"])
        self.assertIn("{ id = 1 } = {}", rows[0][3])
        self.assertEqual(WHERE.lookup(self.root, "phantom"), [])

    def test_output_limits_and_definition_priority_across_files(self):
        self.write("crates/example/a.rs", "\n".join("lookup();" for _ in range(40)))
        self.write("crates/example/z.rs", "\n".join(f"pub fn lookup_{n}() {{}}" for n in range(70)))
        rows = WHERE.lookup(self.root, "lookup")
        self.assertEqual(len(rows), 59)
        self.assertEqual(sum(row[2] == "use" for row in rows), 20)
        self.assertTrue(all(row[2] == "fn" for row in rows[:39]))

    def test_rg_and_python_fallback_agree_with_ignores_and_symlinks(self):
        self.write(".gitignore", "ignored.rs\n")
        self.write("crates/example/.gitignore", "hidden*.rs\n!hidden_keep.rs\n")
        self.write("crates/example/lib.rs", "pub fn locate() {}\nlocate();\n")
        self.write("crates/example/hidden_keep.rs", "pub struct locateKeep;\n")
        for path in ("crates/example/ignored.rs", "crates/example/hidden_drop.rs",
                     "crates/example/target/generated.rs", "web/vendor/dependency.js",
                     "web/node_modules/dependency.js", "web/generated/file.js"):
            self.write(path, "function locate() {}\n")
        linked = self.write("outside.rs", "pub fn locate_link() {}\n")
        (self.root / "crates/example/link.rs").symlink_to(linked)
        with patch.object(WHERE.shutil, "which", return_value=None):
            fallback = WHERE.lookup(self.root, "locate")
        self.assertEqual(len(fallback), 3)
        if shutil.which("rg"):
            self.assertEqual(WHERE.lookup(self.root, "locate"), fallback)

    def test_cli_plain_text_from_other_directory_and_invalid_input(self):
        result = subprocess.run([sys.executable, str(SCRIPT), "PrivOp"], cwd=self.root,
                                capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertRegex(result.stdout.splitlines()[0], r"^crates/[^:]+:\d+  enum  pub enum PrivOp$")
        self.assertLess(len(result.stdout.splitlines()), 60)
        self.assertNotIn("\x1b", result.stdout)
        for query, status in (("___no_such_symbol___", 1), ("", 2), (".*", 2)):
            result = subprocess.run([sys.executable, str(SCRIPT), query], cwd=self.root,
                                    capture_output=True, text=True)
            self.assertEqual(result.returncode, status)


if __name__ == "__main__":
    unittest.main()
