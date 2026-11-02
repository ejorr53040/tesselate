mod common;

use serde_json::{json, Value};
use std::path::PathBuf;

fn call(tool: &str, fixture: &str) -> Vec<u8> {
    let out = common::tesselate()
        .args(["call", tool, "{}"])
        .current_dir(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(fixture),
        )
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}

fn outline(fixture: &str) -> Value {
    serde_json::from_slice(&call("outline", fixture)).unwrap()
}

#[test]
fn outline_lists_every_module_and_definition_with_parent_and_extent() {
    assert_eq!(
        outline("call_repo"),
        json!({
            "schema_version": 1,
            "definitions": [
                {"id": "app", "kind": "module", "name": "app", "path": "app/__init__.py", "line": 1, "end_line": 1},
                {"id": "app.main", "kind": "module", "name": "main", "parent": "app", "path": "app/main.py", "line": 1, "end_line": 12},
                {"id": "app.main.main", "kind": "function", "name": "main", "parent": "app.main", "path": "app/main.py", "line": 7, "end_line": 12},
                {"id": "app.models", "kind": "module", "name": "models", "parent": "app", "path": "app/models.py", "line": 1, "end_line": 11},
                {"id": "app.models.Base", "kind": "class", "name": "Base", "parent": "app.models", "path": "app/models.py", "line": 1, "end_line": 6},
                {"id": "app.models.Base.run", "kind": "method", "name": "run", "parent": "app.models.Base", "path": "app/models.py", "line": 2, "end_line": 3},
                {"id": "app.models.Base.step", "kind": "method", "name": "step", "parent": "app.models.Base", "path": "app/models.py", "line": 5, "end_line": 6},
                {"id": "app.models.Sub", "kind": "class", "name": "Sub", "parent": "app.models", "path": "app/models.py", "line": 9, "end_line": 11},
                {"id": "app.models.Sub.step", "kind": "method", "name": "step", "parent": "app.models.Sub", "path": "app/models.py", "line": 10, "end_line": 11},
                {"id": "app.util", "kind": "module", "name": "util", "parent": "app", "path": "app/util.py", "line": 1, "end_line": 6},
                {"id": "app.util.helper", "kind": "function", "name": "helper", "parent": "app.util", "path": "app/util.py", "line": 1, "end_line": 2},
                {"id": "app.util.other", "kind": "function", "name": "other", "parent": "app.util", "path": "app/util.py", "line": 5, "end_line": 6},
            ],
            "unindexed": [],
        })
    );
}

fn entry<'a>(o: &'a Value, id: &str) -> &'a Value {
    o["definitions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["id"] == id)
        .unwrap_or_else(|| panic!("no `{id}` in outline"))
}

#[test]
fn rust_items_nest_under_their_type_or_module() {
    let o = outline("rust_repo");
    let want = [
        ("shapes.Circle", "struct", "shapes", 3, 5),
        ("shapes.Circle.r", "field", "shapes.Circle", 4, 4),
        ("shapes.Kind.Round", "variant", "shapes.Kind", 8, 8),
        ("shapes.Shape.describe", "method", "shapes.Shape", 13, 15),
        ("shapes.Circle.twice", "method", "shapes.Circle", 34, 38),
        ("shapes.Radius", "type", "shapes", 60, 60),
        ("shapes.util", "module", "shapes", 1, 49),
        ("shapes.util.double_it", "function", "shapes.util", 35, 37),
        // `impl crate::Circle` in util.rs: filed under the type, where calls point.
        ("shapes.Circle.extra", "method", "shapes.Circle", 29, 31),
    ];
    for (id, kind, parent, line, end_line) in want {
        let d = entry(&o, id);
        assert_eq!(
            (&d["kind"], &d["parent"], &d["line"]),
            (&json!(kind), &json!(parent), &json!(line)),
            "{id}"
        );
        assert_eq!(d["end_line"], end_line, "{id}");
    }
    assert_eq!(entry(&o, "shapes.Circle.twice")["name"], "twice");
    assert!(entry(&o, "shapes").get("parent").is_none());
}

#[test]
fn every_edge_caller_joins_an_outline_id() {
    for fixture in ["call_repo", "py_repo", "rust_repo", "web_repo"] {
        let o = outline(fixture);
        let kinds = |id: &str| -> Vec<String> {
            o["definitions"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|d| d["id"] == id)
                .map(|d| d["kind"].as_str().unwrap().to_string())
                .collect()
        };
        let edges: Value = serde_json::from_slice(&call("edges", fixture)).unwrap();
        for e in edges.as_array().unwrap() {
            let caller = e["caller"].as_str().unwrap();
            assert!(
                !kinds(caller).is_empty(),
                "{fixture}: caller `{caller}` not in outline"
            );
            if e["state"] != "resolved" && e["state"] != "candidate" {
                continue;
            }
            for t in e["targets"].as_array().unwrap() {
                let t = t.as_str().unwrap();
                let k = kinds(t);
                assert!(!k.is_empty(), "{fixture}: target `{t}` not in outline");
                // A Rust field and method may share an id; the edge kind says which one.
                if e["kind"] == "field" {
                    assert!(
                        k.contains(&"field".into()),
                        "{fixture}: field edge to `{t}` ({k:?})"
                    );
                }
            }
        }
    }
}

#[test]
fn a_shared_id_is_only_a_field_and_method_namesake() {
    let o = outline("rust_repo");
    let mut by_id: std::collections::BTreeMap<&str, Vec<&str>> = Default::default();
    for d in o["definitions"].as_array().unwrap() {
        by_id
            .entry(d["id"].as_str().unwrap())
            .or_default()
            .push(d["kind"].as_str().unwrap());
    }
    let shared: Vec<_> = by_id.into_iter().filter(|(_, k)| k.len() > 1).collect();
    // shapes/src/lib.rs: `struct Gauge { level: u32 }` and `fn level(&self)`.
    assert_eq!(shared, [("shapes.Gauge.level", vec!["field", "method"])]);
}

#[test]
fn outline_is_deterministic() {
    assert_eq!(outline("rust_repo"), outline("rust_repo"));
    assert_eq!(call("outline", "web_repo"), call("outline", "web_repo"));
}

#[test]
fn empty_files_are_modules_and_skipped_files_are_listed() {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(d.path().join("src")).unwrap();
    std::fs::write(d.path().join("empty.py"), "").unwrap();
    std::fs::write(d.path().join("src/main.go"), "package main\n").unwrap();
    let out = common::tesselate()
        .args(["call", "outline", "{}"])
        .current_dir(d.path())
        .output()
        .unwrap();
    let o: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        o["definitions"],
        json!([{"id": "empty", "kind": "module", "name": "empty", "path": "empty.py", "line": 1, "end_line": 1}])
    );
    assert_eq!(
        o["unindexed"],
        json!([{"path": "src/main.go", "reason": "unsupported-language"}])
    );
}
