#!/usr/bin/env python3
"""Focused standard-library regressions for the index's lightweight scanners."""

import importlib.util
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("gen-index.py")
SPEC = importlib.util.spec_from_file_location("gen_index", SCRIPT)
GEN = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = GEN
SPEC.loader.exec_module(GEN)


class ScannerTests(unittest.TestCase):
    def rows(self, source):
        code, docs = GEN.production_code(source)
        return GEN.symbols("crates/example/src/lib.rs", code, docs)

    def test_comments_literals_visibility_and_multiline_declarations(self):
        source = '''
//! Crate docs are not an item summary.
// pub struct Comment;
/* outer /* pub fn nested() {} */ pub enum Nope {} */
const RAW: &str = br###"pub fn raw() { /* } */ }"###;
const TEXT: &str = "escaped \\\" pub struct String;";
const CHARACTER: char = '}';
pub(crate) fn restricted() {}
pub(super) struct Restricted;
fn private() {}
/// First summary.
/// Second summary.
#[derive(Debug)]
pub struct Visible;
impl Visible {
    /// Method summary.
    #[inline]
    pub async fn method<'a>(&'a self) {}
    pub const fn constant() -> usize { 1 }
}
pub\nunsafe extern "C" fn exported() {}
pub type Alias = Visible;
pub trait Trait {}
pub enum Choice { One }
pub const VALUE: usize = 4;
pub mod nested;
pub use nested::{Item, Other as Renamed};
'''
        rows = self.rows(source)
        self.assertEqual({row[2] for row in rows}, {
            "Visible", "method", "constant", "exported", "Alias", "Trait", "Choice", "VALUE", "nested",
            "nested::{Item, Other as Renamed}",
        })
        summaries = {row[2]: row[3] for row in rows}
        self.assertEqual(summaries["Visible"], "First summary.")
        self.assertEqual(summaries["method"], "Method summary.")
        self.assertEqual(summaries["constant"], "")

    def test_test_items_do_not_hide_later_production_or_leak_docs(self):
        source = '''
/// Test helper docs must not migrate.
#[cfg(test)]
pub fn helper() { let _ = "}"; }
pub fn before() {}
#[cfg(test)]
mod tests { pub struct Fake; #[test] fn nested() {} }
#[cfg(all(unix, test))]
pub fn test_on_unix() {}
#[tokio::test]
async fn async_test() { pub struct Inside; }
#[cfg(any(test, unix))]
pub fn also_production() {}
/// Real documentation.
#[cfg(not(test))]
pub fn after() {}
'''
        rows = self.rows(source)
        self.assertEqual([row[2] for row in rows], ["before", "also_production", "after"])
        self.assertEqual([row[3] for row in rows], ["", "", "Real documentation."])
        code, _ = GEN.production_code(source)
        self.assertNotIn("helper", code)
        self.assertNotIn("Fake", code)
        self.assertIn("before", code)

    def test_block_documentation(self):
        rows = self.rows("/** First line.\n * Second line.\n */\n#[inline]\npub fn block_doc() {}")
        self.assertEqual(rows[0][3], "First line.")

    def test_manifest_dependency_kinds(self):
        package, dependencies = GEN.manifest_fields('''
[package]
name = "example"
description = "Example package."
[dependencies]
opsbox-proto.workspace = true
alias = { package = "opsbox-store", path = "../opsbox-store" }
[dev-dependencies]
opsbox-api.workspace = true
[build-dependencies]
opsbox-version.workspace = true
[target.'cfg(unix)'.dependencies]
opsbox-priv.workspace = true
[dependencies.opsbox-backup]
path = "../opsbox-backup"
[dependencies.renamed]
package = "opsbox-docker"
path = "../opsbox-docker"
''')
        self.assertEqual(package, {"name": "example", "description": "Example package."})
        self.assertTrue({"opsbox-proto", "opsbox-store", "opsbox-version", "opsbox-priv", "opsbox-backup"} <= dependencies)
        self.assertNotIn("opsbox-api", dependencies)
        self.assertIn("opsbox-docker", dependencies)
        self.assertNotIn("renamed", dependencies)

    def test_truncation_keeps_root_exports_then_documented_items(self):
        items = [("crates/example/src/lib.rs", "use", "api::Export", "", True)]
        items += [("crates/example/src/api.rs", "fn", f"documented_{i:03}", "Summary", False) for i in range(420)]
        items += [("crates/example/src/api.rs", "fn", "undocumented", "", False)]
        crate = GEN.Crate("example", "Example", [], "", 0, items)
        output = GEN.symbol_map(crate)
        self.assertEqual(len(output.splitlines()), 400)
        self.assertIn("use api::Export", output)
        self.assertNotIn("fn undocumented", output)
        self.assertTrue(output.endswith("(truncated, 30 more)\n"))

    def test_sensitive_prose_is_redacted(self):
        text = GEN.safe_summary("See /Users/alice/private and /home/ubuntu/private; token=abcd ghp_examplevalue")
        self.assertNotIn("alice", text)
        self.assertNotIn("ubuntu", text)
        self.assertNotIn("abcd", text)
        self.assertNotIn("ghp_", text)


class GenerationTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.write("scripts/gen-index.py", SCRIPT.read_text())
        # Runtime startup discovery uses the same dispatch shape as the binary.
        self.write("crates/opsbox/Cargo.toml", '[package]\nname = "opsbox"\n')
        main = "//! Binary.\nfn run() { match command {\n"
        for role, module, runtime in (("Serve", "serve", "api"), ("Priv", "privileged", "priv"),
                                      ("Proxy", "proxy", "proxy"), ("Watchdog", "watchdog", "watchdog")):
            main += f"Command::{role}(_) => {{\ncmd::{module}::run(config)?;\n}}\n"
            self.write(f"crates/opsbox/src/cmd/{module}.rs", f"use opsbox_{runtime}::Config;\npub fn run() {{}}\n")
        self.write("crates/opsbox/src/main.rs", main + "} }\n")
        self.write("crates/example/Cargo.toml", '[package]\nname = "example"\ndescription = "Fallback purpose."\n')
        self.write("crates/example/src/lib.rs", "pub struct Visible;\n")

    def write(self, relative, content):
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content)

    def run_generator(self, *args):
        return subprocess.run([sys.executable, str(self.root / "scripts/gen-index.py"), *args],
                              cwd=self.root, capture_output=True, text=True)

    def snapshot(self):
        return {path.relative_to(self.root).as_posix(): (path.read_bytes(), path.stat().st_mtime_ns)
                for path in (self.root / "docs").rglob("*") if path.is_file()}

    def test_generation_idempotency_freshness_preservation_and_obsolete_maps(self):
        self.assertEqual(self.run_generator("--check").returncode, 1)
        self.assertFalse((self.root / "docs").exists())
        first = self.run_generator()
        self.assertEqual(first.returncode, 0, first.stderr)
        self.assertIn("TODO: add a //! line", first.stderr)
        self.assertIn("Fallback purpose.", (self.root / "docs/INDEX.md").read_text())
        initial = self.snapshot()
        self.assertEqual(self.run_generator().returncode, 0)
        self.assertEqual(self.snapshot(), initial)
        self.assertEqual(self.run_generator("--check").returncode, 0)
        self.assertEqual(self.snapshot(), initial)
        self.write("crates/example/src/lib.rs", "pub struct Visible;\npub fn added() {}\n")
        check = self.run_generator("--check")
        self.assertEqual(check.returncode, 1)
        self.assertIn("--- a/docs/index/symbols/example.md", check.stdout)
        self.assertIn("+crates/example/src/lib.rs: fn added", check.stdout)
        self.assertEqual(self.snapshot(), initial)
        index = self.root / "docs/INDEX.md"
        index.write_text("Human header.\n" + index.read_text() + "Human footer.\n")
        self.write("docs/index/symbols/removed.md", "# Obsolete map\n")
        self.assertEqual(self.run_generator("--check").returncode, 1)
        self.assertEqual(self.run_generator().returncode, 0)
        self.assertTrue(index.read_text().startswith("Human header.\n"))
        self.assertTrue(index.read_text().endswith("Human footer.\n"))
        self.assertFalse((self.root / "docs/index/symbols/removed.md").exists())
        self.assertEqual(self.run_generator("--check").returncode, 0)
        index.write_text("Missing markers.\n")
        self.assertEqual(self.run_generator().returncode, 1)
        self.assertEqual(index.read_text(), "Missing markers.\n")

    def test_ignored_generated_and_test_sources_are_excluded(self):
        self.write(".gitignore", "ignored.rs\n/crates/example/src/root_only.rs\n*.rs.tmp\n")
        self.write("crates/example/.gitignore", "src/hidden_*.rs\n!src/hidden_keep.rs\nsrc/partial/**\n!src/partial/keep.rs\n")
        for name in ("src/ignored.rs", "src/root_only.rs", "src/hidden_drop.rs", "tests/integration.rs",
                     "target/output.rs", "node_modules/lib.rs", "src/generated/source.rs", "src/vendor/lib.rs"):
            self.write("crates/example/" + name, "pub struct Excluded;\n")
        self.write("crates/example/src/auto.rs", "// @generated DO NOT EDIT\npub struct Generated;\n")
        self.write("crates/example/src/hidden_keep.rs", "pub struct Kept;\n")
        self.write("crates/example/src/partial/keep.rs", "pub struct AlsoKept;\n")
        self.write("crates/example/src/partial/drop.rs", "pub struct Excluded;\n")
        self.assertEqual(self.run_generator().returncode, 0)
        output = (self.root / "docs/index/symbols/example.md").read_text()
        self.assertNotIn("Excluded", output)
        self.assertNotIn("struct Generated", output)
        self.assertIn("struct Kept", output)
        self.assertIn("struct AlsoKept", output)
        self.assertIn("struct Visible", output)

    def test_size_guard_does_not_write_partial_output(self):
        self.write("docs/index/other.md", "x" * 500_000)
        before = self.snapshot()
        result = self.run_generator()
        self.assertEqual(result.returncode, 1)
        self.assertIn("500 KB", result.stderr)
        self.assertEqual(self.snapshot(), before)


if __name__ == "__main__":
    unittest.main()
