//! The Files endpoints over HTTP, against a real `priv` working in a scratch directory.

mod common;

use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;

use axum::http::{Method, StatusCode};
use common::{Browser, Harness, Opts};
use opsbox_proto::Role;
use serde_json::{Value, json};

fn q(p: &Path) -> String {
    // Percent-encode just enough for a path in a query string.
    p.to_string_lossy()
        .replace('%', "%25")
        .replace(' ', "%20")
        .replace('&', "%26")
        .replace('+', "%2B")
        .replace('#', "%23")
}
fn code(r: &common::Reply) -> String {
    r.error_code()
}
fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

async fn admin(h: &Harness) -> Browser {
    h.signed_in("theadmin", Role::Admin)
}

#[tokio::test]
async fn who_may_use_files() {
    let h = Harness::new(true);
    let mut anon = h.browser();
    assert_eq!(
        anon.get("/api/v1/files?path=/").await.status,
        StatusCode::UNAUTHORIZED
    );
    let mut viewer = h.signed_in("viewer1", Role::Viewer);
    for (m, path, body) in [
        (
            Method::GET,
            format!("/api/v1/files?path={}", q(&h.root)),
            None,
        ),
        (
            Method::GET,
            format!("/api/v1/files/content?path={}", q(&h.root)),
            None,
        ),
        (
            Method::PUT,
            "/api/v1/files/content".to_owned(),
            Some(json!({"path": s(&h.root.join("x")), "content": "x"})),
        ),
        (
            Method::POST,
            "/api/v1/files/mkdir".to_owned(),
            Some(json!({"path": s(&h.root.join("d"))})),
        ),
        (
            Method::POST,
            "/api/v1/files/delete".to_owned(),
            Some(json!({"paths": [s(&h.root.join("x"))]})),
        ),
        (
            Method::GET,
            format!("/api/v1/files/download?path={}", q(&h.root)),
            None,
        ),
        (
            Method::POST,
            format!("/api/v1/files/upload?dir={}&name=x", q(&h.root)),
            None,
        ),
    ] {
        let r = viewer.send(m.clone(), &path, body, Opts::default()).await;
        assert_eq!(
            (r.status, code(&r)),
            (StatusCode::FORBIDDEN, "forbidden".to_owned()),
            "viewers have no file access: {m} {path}"
        );
    }
    h.shutdown().await;
}

#[tokio::test]
async fn deployers_see_only_the_apps_directory_even_through_links() {
    let h = Harness::new(true);
    fs::create_dir_all(h.root.join("apps/web")).unwrap();
    fs::write(h.root.join("apps/web/.env"), "A=1").unwrap();
    fs::write(h.root.join("secret.txt"), "top secret").unwrap();
    symlink(&h.root, h.root.join("apps/escape")).unwrap();
    symlink(h.root.join("apps/web"), h.root.join("apps/shortcut")).unwrap();
    let mut d = h.signed_in("deployer1", Role::Deployer);

    let r = d
        .get(&format!("/api/v1/files?path={}", q(&h.root.join("apps"))))
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let names: Vec<String> = r.json()["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap().to_owned())
        .collect();
    assert!(names.contains(&"web".to_owned()));
    assert_eq!(
        d.get(&format!(
            "/api/v1/files?path={}",
            q(&h.root.join("apps/shortcut"))
        ))
        .await
        .status,
        StatusCode::OK,
        "a link that stays inside"
    );

    for outside in [
        h.root.clone(),
        h.root.join("secret.txt"),
        h.root.join("apps/escape"),
        h.root.join("apps/escape/secret.txt"),
        h.root.join("apps/web/../../secret.txt"),
        "/etc".into(),
    ] {
        for endpoint in ["files", "files/content", "files/download"] {
            let r = d
                .get(&format!("/api/v1/{endpoint}?path={}", q(&outside)))
                .await;
            assert_eq!(
                (r.status, code(&r)),
                (StatusCode::FORBIDDEN, "forbidden".to_owned()),
                "{endpoint} {}: {}",
                outside.display(),
                r.text()
            );
            assert!(
                r.json()["error"]["message"]
                    .as_str()
                    .unwrap()
                    .contains("outside the folders"),
                "{}",
                r.text()
            );
        }
    }
    // Writing, creating, deleting and uploading outside are refused too, and nothing changes.
    for (m, path, body) in [
        (
            Method::PUT,
            "/api/v1/files/content",
            json!({"path": s(&h.root.join("planted")), "content": "x"}),
        ),
        (
            Method::PUT,
            "/api/v1/files/content",
            json!({"path": s(&h.root.join("apps/escape/planted")), "content": "x"}),
        ),
        (
            Method::POST,
            "/api/v1/files/mkdir",
            json!({"path": s(&h.root.join("newdir"))}),
        ),
        (
            Method::POST,
            "/api/v1/files/delete",
            json!({"paths": [s(&h.root.join("secret.txt"))], "permanent": true}),
        ),
        (
            Method::POST,
            "/api/v1/files/chmod",
            json!({"path": s(&h.root.join("secret.txt")), "mode": "777"}),
        ),
    ] {
        let r = d.send(m.clone(), path, Some(body), Opts::default()).await;
        assert_eq!(r.status, StatusCode::FORBIDDEN, "{m} {path}: {}", r.text());
    }
    assert!(
        !h.root.join("planted").exists()
            && !h.root.join("newdir").exists()
            && h.root.join("secret.txt").exists()
    );
    // The admin is not confined.
    let mut a = admin(&h).await;
    assert_eq!(
        a.get(&format!("/api/v1/files?path={}", q(&h.root)))
            .await
            .status,
        StatusCode::OK
    );
    h.shutdown().await;
}

#[tokio::test]
async fn the_system_areas_are_never_available() {
    let h = Harness::new(true);
    let mut a = admin(&h).await;
    for p in [
        "/proc/self/environ",
        "/sys/kernel",
        "/dev/null",
        "/etc/opsbox/master.key",
        "/tmp/../proc/1/stat",
    ] {
        for endpoint in ["files", "files/content", "files/download"] {
            let r = a.get(&format!("/api/v1/{endpoint}?path={p}")).await;
            assert_eq!(
                (r.status, code(&r)),
                (StatusCode::FORBIDDEN, "forbidden".to_owned()),
                "{endpoint} {p}: {}",
                r.text()
            );
        }
    }
    let r = a
        .send(
            Method::POST,
            "/api/v1/files/delete",
            Some(json!({"paths": ["/", "/etc"], "permanent": true})),
            Opts::default(),
        )
        .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN, "{}", r.text());
    h.shutdown().await;
}

#[tokio::test]
async fn listing_reading_and_saving_an_edit() {
    let h = Harness::new(true);
    let dir = h.root.join("site");
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("index.html"), "<h1>hi</h1>\n").unwrap();
    fs::write(dir.join(".env"), "SECRET=1").unwrap();
    fs::create_dir(dir.join("logs")).unwrap();
    let mut a = admin(&h).await;

    let v = a
        .get(&format!("/api/v1/files?path={}", q(&dir)))
        .await
        .json();
    assert_eq!(v["path"], s(&dir));
    let e = v["entries"].as_array().unwrap();
    assert_eq!(
        e.iter()
            .map(|x| x["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["logs", ".env", "index.html"],
        "folders first"
    );
    assert_eq!(
        (
            e[0]["type"].clone(),
            e[2]["type"].clone(),
            e[2]["size"].clone()
        ),
        (json!("d"), json!("f"), json!(12))
    );
    assert!(e[1]["warn"].is_null() || e[1]["warn"] == "Readable by everyone");
    for f in ["name", "path", "mode", "mtime", "link", "brokenLink"] {
        assert!(!e[2][f].is_null(), "{f}");
    }

    let r = a
        .get(&format!(
            "/api/v1/files/content?path={}",
            q(&dir.join("index.html"))
        ))
        .await
        .json();
    assert_eq!(
        (r["content"].clone(), r["size"].clone()),
        (json!("<h1>hi</h1>\n"), json!(12))
    );
    let mtime = r["mtime"].as_i64().unwrap();

    let saved = a.send(Method::PUT, "/api/v1/files/content", Some(json!({"path": s(&dir.join("index.html")), "content": "<h1>edited</h1>\n", "mtime": mtime})), Opts::default()).await;
    assert_eq!(saved.status, StatusCode::OK, "{}", saved.text());
    assert!(saved.json()["mtime"].as_i64().unwrap() >= mtime);
    assert_eq!(
        fs::read_to_string(dir.join("index.html")).unwrap(),
        "<h1>edited</h1>\n"
    );

    // A stale editor (someone saved since) is a conflict and loses nothing.
    let stale = a.send(Method::PUT, "/api/v1/files/content", Some(json!({"path": s(&dir.join("index.html")), "content": "<h1>mine</h1>\n", "mtime": mtime - 1000})), Opts::default()).await;
    assert_eq!(
        (stale.status, code(&stale)),
        (StatusCode::CONFLICT, "conflict".to_owned())
    );
    assert!(
        stale.json()["error"]["message"]
            .as_str()
            .unwrap()
            .contains("changed on the server")
    );
    assert_eq!(
        fs::read_to_string(dir.join("index.html")).unwrap(),
        "<h1>edited</h1>\n"
    );

    // Saving needs the CSRF header like every change.
    let r = a
        .send(
            Method::PUT,
            "/api/v1/files/content",
            Some(json!({"path": s(&dir.join("index.html")), "content": "x"})),
            Opts::bare(),
        )
        .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);

    // Every save is in the audit log.
    let audited = h
        .store
        .recent_audit(50)
        .unwrap()
        .into_iter()
        .filter(|e| e.action == "files.write")
        .count();
    assert_eq!(audited, 1, "only the successful save is audited as a write");
    h.shutdown().await;
}

#[tokio::test]
async fn the_editor_accepts_big_saves_but_not_beyond_five_megabytes_of_text() {
    let h = Harness::new(true);
    let mut a = admin(&h).await;
    let target = h.root.join("big.txt");
    // ~4.8 MB of text, whose JSON form is far above the 1 MiB limit of ordinary calls.
    let ok = "line of text\n".repeat(370_000);
    let r = a
        .send(
            Method::PUT,
            "/api/v1/files/content",
            Some(json!({"path": s(&target), "content": ok})),
            Opts::default(),
        )
        .await;
    assert_eq!(
        r.status,
        StatusCode::OK,
        "{}",
        &r.text()[..r.text().len().min(200)]
    );
    assert_eq!(fs::metadata(&target).unwrap().len(), ok.len() as u64);
    // Over 5 MB of content: a sentence, not a crash.
    let too_big = "x".repeat(5 * 1024 * 1024 + 1);
    let r = a
        .send(
            Method::PUT,
            "/api/v1/files/content",
            Some(json!({"path": s(&target), "content": too_big})),
            Opts::default(),
        )
        .await;
    assert_eq!(
        (r.status, code(&r)),
        (StatusCode::BAD_REQUEST, "validation".to_owned())
    );
    assert!(
        r.json()["error"]["message"]
            .as_str()
            .unwrap()
            .contains("too big to save")
    );
    assert_eq!(
        fs::metadata(&target).unwrap().len(),
        ok.len() as u64,
        "the file is untouched"
    );
    // Reading a file over the limit says to download it.
    let huge = h.root.join("huge.bin");
    fs::File::create(&huge)
        .unwrap()
        .set_len(6 * 1024 * 1024)
        .unwrap();
    let r = a
        .get(&format!("/api/v1/files/content?path={}", q(&huge)))
        .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert!(
        r.json()["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Download it instead")
    );
    // Ordinary endpoints still have the 1 MiB limit.
    let r = a
        .send(
            Method::POST,
            "/api/v1/files/mkdir",
            Some(json!({"path": "x".repeat(2 * 1024 * 1024)})),
            Opts::default(),
        )
        .await;
    assert_eq!(r.status, StatusCode::PAYLOAD_TOO_LARGE);
    h.shutdown().await;
}

#[tokio::test]
async fn folders_permissions_and_deleting() {
    let h = Harness::new(true);
    let mut a = admin(&h).await;
    let d = h.root.join("work");
    assert_eq!(
        a.post("/api/v1/files/mkdir", json!({"path": s(&d)}))
            .await
            .status,
        StatusCode::OK
    );
    assert!(d.is_dir());
    let again = a.post("/api/v1/files/mkdir", json!({"path": s(&d)})).await;
    assert_eq!(
        (again.status, code(&again)),
        (StatusCode::CONFLICT, "conflict".to_owned())
    );

    fs::write(d.join("f"), "x").unwrap();
    assert_eq!(
        a.post(
            "/api/v1/files/chmod",
            json!({"path": s(&d.join("f")), "mode": "600"})
        )
        .await
        .status,
        StatusCode::OK
    );
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        fs::metadata(d.join("f")).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let bad = a
        .post(
            "/api/v1/files/chmod",
            json!({"path": s(&d.join("f")), "mode": "999"}),
        )
        .await;
    assert_eq!(
        (
            bad.status,
            bad.json()["error"]["message"].as_str().unwrap().to_owned()
        ),
        (
            StatusCode::BAD_REQUEST,
            "The mode must be 3 or 4 octal digits, for example 600 or 644.".to_owned()
        )
    );

    // Delete goes to the trash by default, and can be made permanent.
    fs::write(d.join("g"), "y").unwrap();
    let r = a
        .post("/api/v1/files/delete", json!({"paths": [s(&d.join("f"))]}))
        .await;
    assert_eq!(
        (
            r.status,
            r.json()["removed"].clone(),
            r.json()["permanent"].clone()
        ),
        (StatusCode::OK, json!(1), json!(false))
    );
    assert!(!d.join("f").exists());
    assert!(
        fs::read_dir(h.root.join("trash")).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with("-f")),
        "it is in the trash"
    );
    let r = a
        .post(
            "/api/v1/files/delete",
            json!({"paths": [s(&d.join("g"))], "permanent": true}),
        )
        .await;
    assert_eq!(r.json()["permanent"], true);
    assert!(!d.join("g").exists());
    assert!(
        !fs::read_dir(h.root.join("trash")).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with("-g"))
    );
    let actions: Vec<String> = h
        .store
        .recent_audit(50)
        .unwrap()
        .into_iter()
        .map(|e| e.action)
        .collect();
    for expected in ["files.mkdir", "files.chmod", "files.trash", "files.delete"] {
        assert!(
            actions.iter().any(|a| a == expected),
            "{expected} in {actions:?}"
        );
    }
    h.shutdown().await;
}

#[tokio::test]
async fn uploads_are_streamed_installed_and_leave_nothing_behind() {
    let h = Harness::with_upload_limit(true, 1 << 20);
    let mut a = admin(&h).await;
    let dest = h.root.join("incoming");
    fs::create_dir(&dest).unwrap();
    let body: Vec<u8> = (0..700_000u32).map(|i| (i % 251) as u8).collect();

    let r = a
        .send_bytes(
            Method::POST,
            &format!("/api/v1/files/upload?dir={}&name=data%20set.bin", q(&dest)),
            body.clone(),
            Opts::default(),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    assert_eq!(r.json()["bytes"], 700_000);
    assert_eq!(
        fs::read(dest.join("data set.bin")).unwrap(),
        body,
        "byte for byte"
    );
    let leftovers = fs::read_dir(h.root.join("xfer")).unwrap().count();
    assert_eq!(leftovers, 0, "the staging folder is empty after an upload");

    // The name is taken: conflict, and nothing is left staged; overwrite=true replaces it.
    let again = a
        .send_bytes(
            Method::POST,
            &format!("/api/v1/files/upload?dir={}&name=data%20set.bin", q(&dest)),
            b"new".to_vec(),
            Opts::default(),
        )
        .await;
    assert_eq!(
        (again.status, code(&again)),
        (StatusCode::CONFLICT, "conflict".to_owned())
    );
    assert_eq!(fs::read_dir(h.root.join("xfer")).unwrap().count(), 0);
    let over = a
        .send_bytes(
            Method::POST,
            &format!(
                "/api/v1/files/upload?dir={}&name=data%20set.bin&overwrite=true",
                q(&dest)
            ),
            b"new".to_vec(),
            Opts::default(),
        )
        .await;
    assert_eq!(over.status, StatusCode::OK);
    assert_eq!(fs::read(dest.join("data set.bin")).unwrap(), b"new");

    // Over the limit: refused with a sentence, nothing created.
    let big = a
        .send_bytes(
            Method::POST,
            &format!("/api/v1/files/upload?dir={}&name=big.bin", q(&dest)),
            vec![7u8; (1 << 20) + 10],
            Opts::default(),
        )
        .await;
    assert!(
        big.status == StatusCode::PAYLOAD_TOO_LARGE || big.status == StatusCode::BAD_REQUEST,
        "{}",
        big.status
    );
    assert!(!dest.join("big.bin").exists());
    assert_eq!(
        fs::read_dir(h.root.join("xfer")).unwrap().count(),
        0,
        "a refused upload leaves no partial file"
    );

    // Bad names and bad folders.
    for name in ["..", "a%2Fb", "", "x%00y"] {
        let r = a
            .send_bytes(
                Method::POST,
                &format!("/api/v1/files/upload?dir={}&name={name}", q(&dest)),
                b"x".to_vec(),
                Opts::default(),
            )
            .await;
        assert!(r.status.is_client_error(), "{name}: {}", r.status);
    }
    let r = a
        .send_bytes(
            Method::POST,
            &format!(
                "/api/v1/files/upload?dir={}&name=x",
                q(&h.root.join("nope"))
            ),
            b"x".to_vec(),
            Opts::default(),
        )
        .await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    // CSRF applies to uploads too.
    let r = a
        .send_bytes(
            Method::POST,
            &format!("/api/v1/files/upload?dir={}&name=y", q(&dest)),
            b"x".to_vec(),
            Opts::bare(),
        )
        .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    assert!(
        h.store
            .recent_audit(50)
            .unwrap()
            .iter()
            .any(|e| e.action == "files.upload")
    );
    h.shutdown().await;
}

#[tokio::test]
async fn a_deployer_uploads_only_into_their_apps() {
    let h = Harness::new(true);
    fs::create_dir_all(h.root.join("apps/web")).unwrap();
    let mut d = h.signed_in("deployer1", Role::Deployer);
    let ok = d
        .send_bytes(
            Method::POST,
            &format!(
                "/api/v1/files/upload?dir={}&name=a.txt",
                q(&h.root.join("apps/web"))
            ),
            b"hi".to_vec(),
            Opts::default(),
        )
        .await;
    assert_eq!(ok.status, StatusCode::OK, "{}", ok.text());
    let no = d
        .send_bytes(
            Method::POST,
            &format!("/api/v1/files/upload?dir={}&name=a.txt", q(&h.root)),
            b"hi".to_vec(),
            Opts::default(),
        )
        .await;
    assert_eq!(no.status, StatusCode::FORBIDDEN);
    assert!(!h.root.join("a.txt").exists());
    assert_eq!(fs::read_dir(h.root.join("xfer")).unwrap().count(), 0);
    h.shutdown().await;
}

#[tokio::test]
async fn downloads_are_exact_uncompressed_and_leave_no_copy() {
    let h = Harness::new(true);
    let mut a = admin(&h).await;
    let data: Vec<u8> = (0..300_000u32).map(|i| (i % 253) as u8).collect();
    fs::write(h.root.join("héllo \"report\".bin"), &data).unwrap();

    let path = q(&h.root.join("héllo \"report\".bin")).replace('"', "%22");
    let r = a
        .send(
            Method::GET,
            &format!("/api/v1/files/download?path={path}"),
            None,
            Opts {
                extra: vec![("accept-encoding".into(), "gzip".into())],
                ..Opts::bare()
            },
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    assert_eq!(r.body, data, "byte for byte");
    assert_eq!(r.header("content-type"), "application/octet-stream");
    assert_eq!(r.header("content-length"), "300000");
    assert_eq!(
        r.header("content-encoding"),
        "",
        "a download is never recompressed"
    );
    assert_eq!(r.header("x-content-type-options"), "nosniff");
    let cd = r.header("content-disposition");
    assert!(
        cd.starts_with("attachment; filename=\"h_llo__report_.bin\"")
            && cd.contains("filename*=UTF-8''h%C3%A9llo%20%22report%22.bin"),
        "{cd}"
    );
    assert_eq!(
        fs::read_dir(h.root.join("xfer")).unwrap().count(),
        0,
        "no staged copy remains"
    );

    let dir = a
        .get(&format!("/api/v1/files/download?path={}", q(&h.root)))
        .await;
    assert_eq!(
        (dir.status, code(&dir)),
        (StatusCode::BAD_REQUEST, "validation".to_owned())
    );
    assert_eq!(
        a.get(&format!(
            "/api/v1/files/download?path={}",
            q(&h.root.join("missing"))
        ))
        .await
        .status,
        StatusCode::NOT_FOUND
    );
    assert!(
        h.store
            .recent_audit(20)
            .unwrap()
            .iter()
            .any(|e| e.action == "files.download")
    );
    h.shutdown().await;
}

#[tokio::test]
async fn without_the_helper_files_say_so() {
    let h = Harness::new(false);
    let mut a = admin(&h).await;
    let r = a.get("/api/v1/files?path=/").await;
    assert_eq!(
        (r.status, code(&r)),
        (StatusCode::SERVICE_UNAVAILABLE, "unavailable".to_owned())
    );
}

#[tokio::test]
async fn missing_and_empty_arguments_are_validation_errors() {
    let h = Harness::new(true);
    let mut a = admin(&h).await;
    for path in [
        "/api/v1/files",
        "/api/v1/files?path=",
        "/api/v1/files/content",
        "/api/v1/files/download",
    ] {
        let r = a.get(path).await;
        assert_eq!(
            (r.status, code(&r)),
            (StatusCode::BAD_REQUEST, "validation".to_owned()),
            "{path}"
        );
        assert_eq!(r.json()["error"]["field"], "path");
    }
    let r = a.get("/api/v1/files?path=relative/path").await;
    assert_eq!(
        (r.status, code(&r)),
        (StatusCode::BAD_REQUEST, "validation".to_owned())
    );
    assert!(
        r.json()["error"]["message"]
            .as_str()
            .unwrap()
            .contains("absolute")
    );
    let _: Value = r.json();
    h.shutdown().await;
}
