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


class TaskMapTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)

    def write(self, relative, content):
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")

    def test_nested_routes_methods_auth_websocket_and_test_exclusion(self):
        self.write("crates/opsbox-api/src/lib.rs", '''
fn router() { Router::new().nest("/api/v1", things::router()) }
// .route("/comment", get(fake))
#[cfg(test)] fn test_router() { Router::new().route("/test", get(fake)) }
''')
        self.write("crates/opsbox-api/src/things.rs", '''
fn router() { Router::new()
    .route("/things", get(list).post(create))
    .route("/things/{id}", put(update).delete(remove))
    .route("/stream", get(stream)) }
fn list(Viewer(who): Viewer) {}
fn create(Admin(who): Admin, client: Client, ApiJson(body): ApiJson<CreateBody>) { save(); }
fn save() { state.store.put_thing(); helper.ping(); }
fn update(Deployer(who): Deployer) { deny_token(&who); }
fn remove(client: Client) {}
fn stream(ws: WebSocketUpgrade) {}
''')
        routes, registry, names = GEN.route_data(self.root)
        self.assertEqual(len(routes), 5)
        keyed = {(row[1], row[2]): row for row in routes}
        self.assertEqual(keyed["POST", "/api/v1/things"][-1], "Admin, Client")
        self.assertEqual(keyed["PUT", "/api/v1/things/{id}"][-1], "Deployer, token refused")
        self.assertEqual(keyed["DELETE", "/api/v1/things/{id}"][-1], "Client")
        self.assertEqual(keyed["GET (WS)", "/api/v1/stream"][-1], "public")
        self.write("docs/API.md", '`GET /things` and `POST /removed`\n')
        variants = {"Ping": {"clients": ["ping"], "handlers": [], "request": "unit"}}
        output = GEN.routes_map(self.root, routes, registry, names, variants, {"put_thing"})
        self.assertIn("things::save", output)
        self.assertIn("CreateBody", output)
        self.assertIn("| ping | put_thing |", output)
        self.assertIn("- `POST /api/v1/removed`", output)
        self.assertIn("- `POST /api/v1/things`", output)
        self.assertNotIn("/comment", output)
        self.assertNotIn("/test", output)

    def test_unsupported_routes_fail_loudly(self):
        self.write("crates/opsbox-api/src/lib.rs", 'fn router() { Router::new().route(PATH, get(handler)) }')
        with self.assertRaisesRegex(ValueError, "unsupported route"):
            GEN.route_data(self.root)

    def test_privops_grouped_inline_tuple_timeout_stream_and_unwired(self):
        self.write("crates/opsbox-proto/src/privilege.rs", '''
pub enum PrivOp { Ping, First(Request), Second(Box<Request>),
    Inline { value: String, size: u64 }, Stream { id: String }, Unwired }
''')
        self.write("crates/opsbox-priv/src/ops/mod.rs", '''
fn dispatch(op: &PrivOp) { match op {
    PrivOp::Ping => ping::run(),
    PrivOp::First(_) | PrivOp::Second(_) => work::run(),
    PrivOp::Inline { .. } => { let v = 1; Ok(v) },
    PrivOp::Stream { .. } => Err(error()),
} }
''')
        self.write("crates/opsbox-priv/src/ops/ping.rs", 'fn run() {}')
        self.write("crates/opsbox-priv/src/ops/work.rs", 'fn run() {}')
        self.write("crates/opsbox-priv/src/client.rs", '''
impl PrivClient {
fn ping() { self.call(PrivOp::Ping); }
fn first() { self.with_timeout(Duration::from_secs(4 * 60)).call(PrivOp::First(req)); }
fn wrapper() { self.first(); }
fn stream() { self.call(PrivOp::Stream { id }); }
#[cfg(test)] fn fake() { self.call(PrivOp::Unwired); }
}
''')
        self.write("crates/opsbox-priv/src/server.rs", '''
fn serve() { if let PrivOp::Stream { id } = &request.op { open_stream(id).await; } }
fn open_stream() {}
''')
        variants = GEN.privop_data(self.root)
        self.assertEqual(len(variants), 6)
        self.assertEqual(variants["Second"]["request"], "Box<Request>")
        self.assertIn("size: u64", variants["Inline"]["request"])
        self.assertEqual(variants["First"]["handlers"], variants["Second"]["handlers"])
        output = GEN.privops_map(self.root, variants)
        self.assertIn("first [timeout Duration::from_secs(4 * 60)]", output)
        self.assertIn("wrapper [delegates]", output)
        self.assertIn("open_stream (stream)", output)
        self.assertIn("Unwired: no handler or client method", output)
        self.assertNotIn("fake", output)

    def test_schema_alters_indexes_method_delegation_and_literal_settings(self):
        self.write("crates/opsbox-store/migrations/0001_initial.sql", '''
-- CREATE TABLE fake(secret TEXT);
CREATE TABLE things(id TEXT PRIMARY KEY, payload TEXT CHECK(json_valid(payload)));
CREATE UNIQUE INDEX things_payload ON things(payload);
''')
        self.write("crates/opsbox-store/migrations/0002_extra.sql", 'ALTER TABLE things ADD COLUMN enabled INTEGER;')
        self.write("crates/opsbox-store/src/lib.rs", '''
impl Store {
fn save(&self) { conn.execute("INSERT INTO things VALUES (?, ?, ?)"); }
fn wrapper(&self) { self.save(); }
#[cfg(test)] fn test_only() { conn.execute("SELECT * FROM things"); }
}
fn outside() { conn.execute("SELECT * FROM things"); }
''')
        self.write("crates/opsbox-api/src/settings.rs", '''
fn read() { store.get_setting("literal.key"); store.get_setting(&format!("computed.{}", id)); }
#[cfg(test)] fn fake() { store.get_setting("test.key"); }
// store.get_setting("comment.key");
''')
        sources, methods = GEN.store_data(self.root)
        output = GEN.store_map(self.root, sources, methods)
        self.assertIn("id TEXT, payload TEXT, enabled INTEGER", output)
        self.assertIn("CREATE UNIQUE INDEX things_payload ON things(payload)", output)
        self.assertIn("UNIQUE (id) [implicit]", output)
        self.assertIn("0001_initial.sql", output)
        self.assertIn("| wrapper | crates/opsbox-store/src/lib.rs | save |", output)
        self.assertIn("`literal.key`", output)
        for excluded in ("fake", "outside", "test_only", "test.key", "comment.key", "computed."):
            self.assertNotIn(excluded, output)

    def test_js_lexer_templates_regex_and_default_arguments(self):
        source = '''
// export function fake() {}
export async function load({ id = '}' } = {}) {
  const regex = /["'{}\\/]/g;
  const text = `quoted ${id ? `nested ${id}` : '}'}`;
  return http(`/things/${encodeURIComponent(id)}`, { method: 'PATCH' });
}
'''
        code, fns = GEN.js_functions(source, exported=True)
        self.assertEqual([fn.name for fn in fns], ["load"])
        self.assertEqual(GEN.js_endpoints(source, fns[0], code), [("PATCH", "/api/v1/things/{id}")])
        self.assertNotIn("fake", code)
        self.assertIn("return http", code)

    def test_rebuilt_and_renamed_tables_do_not_leave_temporary_rows(self):
        self.write("crates/opsbox-store/migrations/0001_initial.sql", 'CREATE TABLE things(id TEXT);')
        self.write("crates/opsbox-store/migrations/0002_rebuild.sql", '''
CREATE TABLE things_new(id TEXT, enabled INTEGER);
DROP TABLE things;
ALTER TABLE things_new RENAME TO things;
CREATE TABLE old_name(value TEXT);
ALTER TABLE old_name RENAME TO new_name;
''')
        sources, methods = GEN.store_data(self.root)
        output = GEN.store_map(self.root, sources, methods)
        self.assertIn("| things | id TEXT, enabled INTEGER |", output)
        self.assertIn("0001_initial.sql", output)
        self.assertIn("| new_name | value TEXT |", output)
        self.assertNotIn("| things_new |", output)
        self.assertNotIn("| old_name |", output)

    def test_ui_real_mock_helpers_hash_css_and_conditional_paths(self):
        self.write("web/index.html", '<link rel="stylesheet" href="css/app.css">')
        self.write("web/js/main.js", r"const ROUTES = [[/^\/db\/[^/]+\/?$/, 'database']];")
        self.write("web/js/pages/database.js", '''
import * as api from '../api.js';
const base = new URL('../../vendor/tool/', import.meta.url).href;
link.href = base + 'tool.css';
export async function view() { api.create(); api.mockAction(); api.auth.login(); }
// api.fake();
''')
        self.write("web/js/api.js", '''
const REAL = new Set(['create', 'auth.login', 'backup']);
export async function create() { return http('/things', { method: 'POST' }); }
export function get() { if (MODE === 'http') return http('/things'); }
export function mockAction() { sample('mockAction'); }
export function local() { return 1; }
export async function backup(id) { const path = id ? `/backups/${enc(id)}` : '/backups'; return http(path, { method: 'POST' }); }
export function stream() { if (MODE !== 'http') return; return new WebSocket(`${location.host}/api/v1/stream/logs?tail=1`); }
export const auth = { async login() { return http('/auth/login', { method: 'POST' }); } };
''')
        self.write("web/js/ui.js", '''
// Escape user text.
export const esc = (text) => text;
// Present a dialog.
export function dialog() {}
''')
        output = GEN.ui_map(self.root)
        self.assertLess(output.index("## Still mock"), output.index("## Pages"))
        self.assertIn("| mockAction | database |", output)
        self.assertIn("| create | REAL | POST /api/v1/things | database |", output)
        self.assertIn("| get | REAL | GET /api/v1/things |", output)
        self.assertIn("POST /api/v1/backups; POST /api/v1/backups/{id}", output)
        self.assertIn("| local | REAL | local utility (no HTTP) |", output)
        self.assertIn("| auth.login | REAL | POST /api/v1/auth/login | database |", output)
        self.assertIn("GET (WS) /api/v1/stream/logs", output)
        self.assertIn("web/vendor/tool/tool.css", output)
        self.assertIn("# matching /^\\/db\\/[^/]+\\/?$/", output)
        self.assertIn("| esc | Escape user text. |", output)
        self.assertIn("| dialog | Present a dialog. |", output)
        self.assertNotIn("fake", output)
        self.assertNotIn("unrouted", output)
        self.assertNotIn("| unresolved |", output)


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

    def test_new_maps_and_dummy_route_check_is_read_only(self):
        source = 'fn router() { Router::new().route("/one", get(health)) }\nfn health() {}\n'
        self.write("crates/opsbox-api/src/lib.rs", source)
        self.assertEqual(self.run_generator().returncode, 0)
        for name in ("routes", "privops", "store", "ui"):
            self.assertTrue((self.root / "docs/index" / (name + ".md")).is_file())
        initial = self.snapshot()
        self.write("crates/opsbox-api/src/lib.rs", source.replace('.route("/one", get(health))',
                   '.route("/one", get(health)).route("/dummy", get(health))'))
        check = self.run_generator("--check")
        self.assertEqual(check.returncode, 1)
        self.assertIn("a/docs/index/routes.md", check.stdout)
        self.assertIn("/dummy", check.stdout)
        self.assertEqual(self.snapshot(), initial)
        self.assertEqual(self.run_generator().returncode, 0)
        self.assertEqual(self.run_generator("--check").returncode, 0)
        self.write("crates/opsbox-api/src/lib.rs", source)
        self.assertEqual(self.run_generator("--check").returncode, 1)
        self.assertEqual(self.run_generator().returncode, 0)
        self.assertEqual(self.run_generator("--check").returncode, 0)


if __name__ == "__main__":
    unittest.main()
