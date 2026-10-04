use multi_agent_on_chat::{
    application::{media as app, Scope},
    domain::*,
    media::{self, extractions as jobs, extractors::Options},
    store::Store,
};
use serde_json::json;
fn setup() -> (tempfile::TempDir, Store) {
    let d = tempfile::tempdir().unwrap();
    let s = Store::open(d.path()).unwrap();
    s.register_template(&serde_json::from_value(json!({"id":"default"})).unwrap())
        .unwrap();
    for id in ["one", "two"] {
        s.add_bot_with_defaults(&Bot {
            id: id.into(),
            channel: "local".into(),
            account_id: id.into(),
            credential_ref: None,
            enabled: true,
            owner: Some("local".into()),
        })
        .unwrap();
    }
    (d, s)
}
fn pdf(path: &std::path::Path, text: &str) {
    let stream = format!("BT /F1 16 Tf 72 720 Td ({text}) Tj ET");
    let objects=["<< /Type /Catalog /Pages 2 0 R >>".to_owned(),"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".into(),"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),format!("<< /Length {} >>
stream
{}
endstream",stream.len(),stream)];
    let mut bytes = b"%PDF-1.4
"
    .to_vec();
    let mut offsets = vec![0];
    for (i, obj) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(
            format!(
                "{} 0 obj
{}
endobj
",
                i + 1,
                obj
            )
            .as_bytes(),
        );
    }
    let xref = bytes.len();
    bytes.extend_from_slice(
        b"xref
0 6
0000000000 65535 f 
",
    );
    for offset in &offsets[1..] {
        bytes.extend_from_slice(
            format!(
                "{offset:010} 00000 n 
"
            )
            .as_bytes(),
        );
    }
    bytes.extend_from_slice(
        format!(
            "trailer
<< /Size 6 /Root 1 0 R >>
startxref
{xref}
%%EOF
"
        )
        .as_bytes(),
    );
    std::fs::write(path, bytes).unwrap();
}
#[tokio::test]
async fn pdf_job_result_cache_and_scope() {
    let (d, s) = setup();
    let p = d.path().join("sample.pdf");
    pdf(&p, "Known PDF text 42");
    let a = media::import(&s, "one", "chat", &p, "attachment", None).unwrap();
    let j = jobs::request(
        &s,
        &a.id,
        Some("one"),
        Some("chat"),
        Options::default(),
        None,
        None,
    )
    .await
    .unwrap();
    let id = j["id"].as_str().unwrap();
    assert_eq!(j["state"], "queued");
    assert!(jobs::work_once(&s).await.unwrap());
    let result = jobs::result(&s, id, Some("one"), Some("chat"), 0, 4000).unwrap();
    assert_eq!(result["state"], "succeeded");
    assert_eq!(result["segments"][0]["page"], 1);
    assert!(result["segments"][0]["text"]
        .as_str()
        .unwrap()
        .contains("Known PDF text 42"));
    assert!(jobs::result(&s, id, Some("two"), None, 0, 4000).is_err());
    assert!(jobs::status(&s, id, Some("one"), Some("other")).is_err());
    let cached = jobs::request(
        &s,
        &a.id,
        Some("one"),
        Some("chat"),
        Options::default(),
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(cached["id"], j["id"]);
    assert_eq!(cached["cached"], true);
}
#[tokio::test]
async fn text_unicode_pagination_and_recovery() {
    let (d, s) = setup();
    let p = d.path().join("text.txt");
    std::fs::write(&p, "你好世界abcdef").unwrap();
    let a = media::import(&s, "one", "chat", &p, "attachment", None).unwrap();
    let j = jobs::request(&s, &a.id, None, None, Options::default(), None, None)
        .await
        .unwrap();
    let id = j["id"].as_str().unwrap();
    s.conn().unwrap().execute("UPDATE media_extraction_jobs SET state='running',generation=1,data=json_set(data,'$.state','running','$.generation',1) WHERE id=?1",[id]).unwrap();
    jobs::recover(&s).unwrap();
    assert_eq!(jobs::status(&s, id, None, None).unwrap()["state"], "queued");
    jobs::work_once(&s).await.unwrap();
    let first = jobs::result(&s, id, None, None, 0, 3).unwrap();
    assert_eq!(first["segments"][0]["text"], "你好世");
    assert_eq!(first["next_offset"], 3);
    assert_eq!(
        jobs::result(&s, id, None, None, 3, 3).unwrap()["segments"][0]["text"],
        "界ab"
    );
}
#[tokio::test]
async fn cancellation_qr_and_clear() {
    let (d, s) = setup();
    let p = d.path().join("text.txt");
    std::fs::write(&p, "sample").unwrap();
    let qr = media::import(&s, "one", "chat", &p, "auth_qr", None).unwrap();
    assert!(
        jobs::request(&s, &qr.id, None, None, Options::default(), None, None)
            .await
            .is_err()
    );
    let a = media::import(&s, "one", "chat", &p, "attachment", None).unwrap();
    let j = jobs::request(&s, &a.id, None, None, Options::default(), None, None)
        .await
        .unwrap();
    let id = j["id"].as_str().unwrap();
    s.clear("one-main", "chat").unwrap();
    assert_eq!(
        jobs::status(&s, id, None, None).unwrap()["state"],
        "cancelled"
    );
    assert!(!jobs::work_once(&s).await.unwrap());
}
#[tokio::test]
async fn scan_and_corrupt_pdf_fail_explicitly() {
    let (d, s) = setup();
    for (filename, code) in [("scan.pdf", "needs_ocr"), ("bad.pdf", "invalid_document")] {
        let p = d.path().join(filename);
        if filename == "scan.pdf" {
            pdf(&p, "");
        } else {
            std::fs::write(&p, b"%PDF-broken").unwrap();
        }
        let a = media::import(&s, "one", "chat", &p, "attachment", None).unwrap();
        let j = jobs::request(&s, &a.id, None, None, Options::default(), None, None)
            .await
            .unwrap();
        jobs::work_once(&s).await.unwrap();
        let state = jobs::status(&s, j["id"].as_str().unwrap(), None, None).unwrap();
        assert_eq!(state["state"], "failed");
        assert_eq!(state["error"]["code"], code);
    }
}
#[tokio::test]
async fn gc_retention_starts_after_reference_release() {
    let (d, s) = setup();
    let p = d.path().join("text.txt");
    std::fs::write(&p, "hello").unwrap();
    let mut a = media::import(&s, "one", "chat", &p, "attachment", None).unwrap();
    a.created = now() - 90 * 86400;
    s.conn()
        .unwrap()
        .execute(
            "UPDATE media_assets SET data=?1 WHERE id=?2",
            rusqlite::params![serde_json::to_string(&a).unwrap(), a.id],
        )
        .unwrap();
    let j = jobs::request(&s, &a.id, None, None, Options::default(), None, None)
        .await
        .unwrap();
    assert_eq!(
        app::gc(&s, &Scope::Local, false).unwrap()["assets"],
        json!([])
    );
    jobs::cancel(&s, j["id"].as_str().unwrap(), None, None).unwrap();
    assert_eq!(
        app::gc(&s, &Scope::Local, false).unwrap()["assets"],
        json!([])
    );
    s.save("media_unreferenced", &a.id, &(now() - 8 * 86400))
        .unwrap();
    assert_eq!(
        app::gc(&s, &Scope::Local, false).unwrap()["assets"],
        json!([a.id])
    );
    assert!(!a.path.exists());
}
#[tokio::test]
async fn chat_adapter_scopes_requests_and_formats_result() {
    let (d, s) = setup();
    let p = d.path().join("text.txt");
    std::fs::write(&p, "chat parser content").unwrap();
    let a = media::import(&s, "one", "chat", &p, "attachment", None).unwrap();
    let response = multi_agent_on_chat::service::chat(
        &s,
        "one-main",
        "chat",
        &format!("/media extract {}", a.id),
        None,
    )
    .await
    .unwrap();
    let id = response["id"].as_str().unwrap();
    assert_eq!(response["state"], "queued");
    jobs::work_once(&s).await.unwrap();
    let value = multi_agent_on_chat::service::chat(
        &s,
        "one-main",
        "chat",
        &format!("/media result {id}"),
        None,
    )
    .await
    .unwrap();
    assert!(
        multi_agent_on_chat::presentation::render("/media result", &value)
            .contains("chat parser content")
    );
    assert!(multi_agent_on_chat::service::chat(
        &s,
        "one-main",
        "other",
        &format!("/media status {id}"),
        None
    )
    .await
    .is_err());
}
#[tokio::test]
async fn cancelled_generation_cannot_publish_result_or_notify() {
    let (d, s) = setup();
    let p = d.path().join("text.txt");
    std::fs::write(&p, "text").unwrap();
    let a = media::import(&s, "one", "chat", &p, "attachment", None).unwrap();
    let job = jobs::request(
        &s,
        &a.id,
        None,
        None,
        Options::default(),
        Some(ReplyTarget {
            chat_id: "chat".into(),
            thread_id: None,
            context_token: None,
        }),
        None,
    )
    .await
    .unwrap();
    let id = job["id"].as_str().unwrap();
    jobs::cancel(&s, id, None, None).unwrap();
    assert!(!jobs::work_once(&s).await.unwrap());
    assert_eq!(
        jobs::status(&s, id, None, None).unwrap()["result_available"],
        false
    );
    assert!(s.deliveries().unwrap().is_empty());
}
#[tokio::test]
async fn missing_source_prevents_cached_result_read() {
    let (d, s) = setup();
    let p = d.path().join("text.txt");
    std::fs::write(&p, "text").unwrap();
    let a = media::import(&s, "one", "chat", &p, "attachment", None).unwrap();
    let job = jobs::request(&s, &a.id, None, None, Options::default(), None, None)
        .await
        .unwrap();
    jobs::work_once(&s).await.unwrap();
    std::fs::remove_file(a.path).unwrap();
    assert!(jobs::result(&s, job["id"].as_str().unwrap(), None, None, 0, 4000).is_err());
}
#[test]
fn v2_upgrade_is_repeatable_and_new_references_reset_retention() {
    let (d, s) = setup();
    s.conn().unwrap().execute_batch("DROP TRIGGER media_ref_resets_retention; DROP TABLE media_extraction_jobs; PRAGMA user_version=2;").unwrap();
    let s = Store::open(d.path()).unwrap();
    assert_eq!(
        s.conn()
            .unwrap()
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        3
    );
    let p = d.path().join("file.txt");
    std::fs::write(&p, "hello").unwrap();
    let a = media::import(&s, "one", "chat", &p, "attachment", None).unwrap();
    s.save("media_unreferenced", &a.id, &1).unwrap();
    s.submit_media(
        "one-main",
        "chat",
        "read",
        None,
        std::slice::from_ref(&a.id),
        None,
    )
    .unwrap();
    assert!(s.get::<i64>("media_unreferenced", &a.id).is_err());
    Store::open(d.path()).unwrap();
}
