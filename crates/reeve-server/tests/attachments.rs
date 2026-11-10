//! The attachment routes (kb/ADR-0022, kb/ADR-0023) end to end: upload limits and types, headers, access, revocation,
//! the ETag path, cookie writes, claims through the JSON writes, and multi-chunk serving.
mod common;

mod attachments {
    use super::common::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use serde_json::{Value, json};

    fn png(w: u32, h: u32, noise: bool) -> Vec<u8> {
        let img = image::RgbaImage::from_fn(w, h, |x, y| {
            if noise {
                // A cheap hash so the PNG does not compress: big files from small dimensions.
                let v = (x.wrapping_mul(2_654_435_761) ^ y.wrapping_mul(40_503)).to_le_bytes();
                image::Rgba([v[0], v[1], v[2], 255])
            } else {
                image::Rgba([10, 20, 30, 255])
            }
        });
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    fn upload_req(token: &str, name: &str, idem: Option<&str>, body: Body) -> Request<Body> {
        let mut uri = format!("/api/v1/attachments?filename={name}");
        if let Some(k) = idem {
            uri.push_str(&format!("&idempotency_key={k}"));
        }
        Request::post(uri)
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/octet-stream")
            .body(body)
            .unwrap()
    }

    async fn upload(h: &Harness, token: &str, bytes: Vec<u8>) -> (u16, Value) {
        let res = call_raw(
            &h.app,
            upload_req(token, "shot.png", None, Body::from(bytes)),
        )
        .await;
        let status = res.status().as_u16();
        (status, body_json(res).await)
    }

    async fn get(
        h: &Harness,
        token: &str,
        id: &str,
        etag: Option<&str>,
    ) -> axum::http::Response<Body> {
        let mut r = Request::get(format!("/api/v1/attachments/{id}"))
            .header("authorization", format!("Bearer {token}"));
        if let Some(t) = etag {
            r = r.header("if-none-match", t);
        }
        call_raw(&h.app, r.body(Body::empty()).unwrap()).await
    }

    async fn bytes_of(res: axum::http::Response<Body>) -> Vec<u8> {
        res.into_body().collect().await.unwrap().to_bytes().to_vec()
    }

    /// A story created as dan, commenting with `ids` claimed for it; its key.
    async fn story_with(h: &Harness, ids: &[&str]) -> String {
        let (s, v) = call(
            &h.app,
            Some(&h.admin_token),
            "POST",
            "/api/v1/stories",
            Some(json!({"idempotency_key": format!("s-{}", ids.join("-")), "title": "S", "kind": "story", "criteria": [], "labels": [], "attachments": ids})),
        )
        .await;
        assert_eq!(s, 200, "{v}");
        v["value"]["key"].as_str().unwrap().to_string()
    }

    #[tokio::test]
    async fn an_upload_is_served_with_the_safety_headers() {
        let h = harness().await;
        let bytes = png(8, 8, false);
        let (s, v) = upload(&h, &h.admin_token, bytes.clone()).await;
        assert_eq!(s, 200, "{v}");
        let id = v["value"]["id"].as_str().unwrap();
        assert_eq!(v["value"]["url"], format!("/api/v1/attachments/{id}"));
        assert!(v["value"].get("owner").is_none());
        let res = get(&h, &h.admin_token, id, None).await;
        assert_eq!(res.status(), 200);
        let hd = res.headers().clone();
        assert_eq!(hd["content-type"], "image/png");
        assert_eq!(hd["x-content-type-options"], "nosniff");
        assert_eq!(hd["content-security-policy"], "default-src 'none'; sandbox");
        assert_eq!(hd["cache-control"], "private, no-cache");
        assert!(
            hd["content-disposition"]
                .to_str()
                .unwrap()
                .starts_with("inline; filename*=UTF-8''shot.png")
        );
        assert!(hd["etag"].to_str().unwrap().len() == 66, "a quoted sha256");
        assert_eq!(
            bytes_of(res).await,
            bytes,
            "a clean PNG comes back unchanged"
        );
    }

    #[tokio::test]
    async fn the_declared_type_is_ignored_and_other_types_are_refused() {
        let h = harness().await;
        let res = call_raw(
            &h.app,
            Request::post("/api/v1/attachments?filename=x.jpg")
                .header("authorization", format!("Bearer {}", h.admin_token))
                .header("content-type", "image/jpeg")
                .body(Body::from(png(4, 4, false)))
                .unwrap(),
        )
        .await;
        assert_eq!(body_json(res).await["value"]["content_type"], "image/png");
        let (s, v) = upload(
            &h,
            &h.admin_token,
            b"<svg xmlns='http://www.w3.org/2000/svg'/>".to_vec(),
        )
        .await;
        assert_eq!(
            (s, v["code"].as_str(), v["message"].as_str()),
            (
                400,
                Some("validation"),
                Some("only PNG, JPEG and WebP images can be attached")
            )
        );
    }

    #[tokio::test]
    async fn the_cap_is_inclusive_and_enforced_with_or_without_a_length() {
        let h = harness().await;
        let cap = 10_485_760;
        let mut at_cap = b"\x89PNG\r\n\x1a\n".to_vec();
        at_cap.resize(cap, 0);
        let (s, v) = upload(&h, &h.admin_token, at_cap.clone()).await;
        assert_eq!(
            (s, v["message"].as_str()),
            (400, Some("the image could not be read")),
            "exactly the cap is read"
        );
        at_cap.push(0);
        let (s, v) = upload(&h, &h.admin_token, at_cap.clone()).await;
        assert_eq!(
            (s, v["code"].as_str(), v["message"].as_str()),
            (
                413,
                Some("validation"),
                Some("the file is larger than 10 MiB")
            )
        );
        // A streamed body, with no Content-Length.
        let chunks: Vec<Result<Vec<u8>, std::io::Error>> =
            at_cap.chunks(1 << 20).map(|c| Ok(c.to_vec())).collect();
        let res = call_raw(
            &h.app,
            upload_req(
                &h.admin_token,
                "x.png",
                None,
                Body::from_stream(futures_util::stream::iter(chunks)),
            ),
        )
        .await;
        assert_eq!(res.status(), 413);
        assert_eq!(body_json(res).await["code"], "validation");
    }

    #[tokio::test]
    async fn other_routes_keep_the_default_body_limit() {
        let h = harness().await;
        let (s, v) = call(
            &h.app,
            Some(&h.admin_token),
            "POST",
            "/api/v1/stories",
            Some(json!({"idempotency_key": "big", "title": "S", "kind": "story", "criteria": [], "labels": [], "description": "x".repeat(3 << 20)})),
        )
        .await;
        assert_eq!(s, 400, "{v}");
        assert!(
            v["message"].as_str().unwrap().contains("length limit"),
            "{v}"
        );
    }

    #[tokio::test]
    async fn only_the_uploader_sees_an_upload_until_a_write_claims_it() {
        let h = harness().await;
        let (_, v) = upload(&h, &h.admin_token, png(4, 4, false)).await;
        let id = v["value"]["id"].as_str().unwrap().to_string();
        assert_eq!(get(&h, &h.victim_token, &id, None).await.status(), 404);
        let del = call(
            &h.app,
            Some(&h.victim_token),
            "DELETE",
            &format!("/api/v1/attachments/{id}"),
            None,
        )
        .await;
        assert_eq!(del.0, 404);
        // Another member's conditional request is refused too, not answered 304.
        let tag = get(&h, &h.admin_token, &id, None).await.headers()["etag"]
            .to_str()
            .unwrap()
            .to_string();
        assert_eq!(
            get(&h, &h.victim_token, &id, Some(&tag)).await.status(),
            404
        );
        let unauth = call_raw(
            &h.app,
            Request::get(format!("/api/v1/attachments/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(unauth.status(), 401);

        let key = story_with(&h, &[&id]).await;
        assert_eq!(get(&h, &h.victim_token, &id, None).await.status(), 200);
        let (s, list) = call(
            &h.app,
            Some(&h.victim_token),
            "GET",
            &format!("/api/v1/items/{key}/attachments"),
            None,
        )
        .await;
        assert_eq!(s, 200);
        assert_eq!(list["value"][0]["id"], id.as_str());
        assert_eq!(list["value"][0]["owner"]["key"], key.as_str());
        let (s, _) = call(
            &h.app,
            Some(&h.victim_token),
            "GET",
            "/api/v1/items/ABC-999/attachments",
            None,
        )
        .await;
        assert_eq!(s, 404);
    }

    #[tokio::test]
    async fn a_304_comes_only_after_the_access_check() {
        let h = harness().await;
        let (_, v) = upload(&h, &h.admin_token, png(4, 4, false)).await;
        let id = v["value"]["id"].as_str().unwrap().to_string();
        story_with(&h, &[&id]).await;
        let first = get(&h, &h.victim_token, &id, None).await;
        let tag = first.headers()["etag"].to_str().unwrap().to_string();
        let again = get(&h, &h.admin_token, &id, Some(&tag)).await;
        assert_eq!(again.status(), 304);
        assert_eq!(again.headers()["x-content-type-options"], "nosniff");
        assert_eq!(again.headers()["cache-control"], "private, no-cache");
        assert_eq!(again.headers()["etag"], tag.as_str());
        assert!(bytes_of(again).await.is_empty());
        // A weakened tag (a proxy may add W/) and a wildcard match too.
        assert_eq!(
            get(&h, &h.admin_token, &id, Some(&format!("W/{tag}")))
                .await
                .status(),
            304
        );
        assert_eq!(get(&h, &h.admin_token, &id, Some("*")).await.status(), 304);

        // Disabled: refused on the next request, conditional or not.
        let (s, _) = call(
            &h.app,
            Some(&h.admin_token),
            "POST",
            &format!("/api/v1/users/{}/disable", h.victim_email),
            None,
        )
        .await;
        assert_eq!(s, 200);
        assert_eq!(
            get(&h, &h.victim_token, &id, Some(&tag)).await.status(),
            401
        );

        // Deleted: gone for everyone, conditional or not.
        let (s, _) = call(
            &h.app,
            Some(&h.admin_token),
            "DELETE",
            &format!("/api/v1/attachments/{id}"),
            None,
        )
        .await;
        assert_eq!(s, 200);
        assert_eq!(get(&h, &h.admin_token, &id, Some(&tag)).await.status(), 404);
    }

    #[tokio::test]
    async fn cookie_writes_need_the_client_header() {
        let h = harness().await;
        let req = |header: bool, method: &str, uri: String, body: Body| {
            let mut r = Request::builder()
                .method(method)
                .uri(uri)
                .header("cookie", format!("reeve_session={}", h.admin_token));
            if header {
                r = r.header("x-reeve-client", "web");
            }
            r.body(body).unwrap()
        };
        let up = |header| {
            req(
                header,
                "POST",
                "/api/v1/attachments?filename=a.png".into(),
                Body::from(png(4, 4, false)),
            )
        };
        assert_eq!(call_raw(&h.app, up(false)).await.status(), 403);
        let res = call_raw(&h.app, up(true)).await;
        assert_eq!(res.status(), 200);
        let id = body_json(res).await["value"]["id"]
            .as_str()
            .unwrap()
            .to_string();
        let del = |header| {
            req(
                header,
                "DELETE",
                format!("/api/v1/attachments/{id}"),
                Body::empty(),
            )
        };
        assert_eq!(call_raw(&h.app, del(false)).await.status(), 403);
        assert_eq!(call_raw(&h.app, del(true)).await.status(), 200);
    }

    #[tokio::test]
    async fn claims_through_comments_are_logged_via_automation_for_a_bot() {
        let h = harness().await;
        let key = story_with(&h, &[]).await;
        let (_, v) = upload(&h, &h.bot_token, png(4, 4, false)).await;
        let id = v["value"]["id"].as_str().unwrap();
        let (s, v) = call(
            &h.app,
            Some(&h.bot_token),
            "POST",
            &format!("/api/v1/items/{key}/comments"),
            Some(json!({"body": "see", "attachments": [id]})),
        )
        .await;
        assert_eq!(s, 200, "{v}");
        let via: String =
            sqlx::query_scalar("SELECT via FROM activity WHERE action = 'attachment_added'")
                .fetch_one(&h.state.pool)
                .await
                .unwrap();
        assert_eq!(via, "automation");
        // A decision comment refuses images.
        let (s, v) = call(
            &h.app,
            Some(&h.admin_token),
            "POST",
            "/api/v1/decisions/1/comments",
            Some(json!({"body": "x", "attachments": [id]})),
        )
        .await;
        assert_eq!(
            (s, v["message"].as_str()),
            (400, Some("images can't be attached to decisions"))
        );
    }

    #[tokio::test]
    async fn a_multi_chunk_image_round_trips_exactly() {
        let h = harness().await;
        let bytes = png(1100, 800, true);
        assert!(
            bytes.len() > 2 * (1 << 20),
            "{} bytes is not multi-chunk",
            bytes.len()
        );
        let (s, v) = upload(&h, &h.admin_token, bytes.clone()).await;
        assert_eq!(s, 200, "{v}");
        let res = get(&h, &h.admin_token, v["value"]["id"].as_str().unwrap(), None).await;
        assert_eq!(
            res.headers()["content-length"],
            bytes.len().to_string().as_str()
        );
        assert_eq!(bytes_of(res).await, bytes);
    }

    #[tokio::test]
    async fn an_empty_idempotency_key_is_no_key() {
        let h = harness().await;
        let first = call_raw(
            &h.app,
            upload_req(
                &h.admin_token,
                "a.png",
                Some(""),
                Body::from(png(4, 4, false)),
            ),
        )
        .await;
        let second = call_raw(
            &h.app,
            upload_req(
                &h.admin_token,
                "b.png",
                Some(""),
                Body::from(png(5, 5, false)),
            ),
        )
        .await;
        let (a, b) = (body_json(first).await, body_json(second).await);
        assert_ne!(
            a["value"]["id"], b["value"]["id"],
            "two uploads with an empty key were merged"
        );
        let long = "k".repeat(201);
        let res = call_raw(
            &h.app,
            upload_req(
                &h.admin_token,
                "c.png",
                Some(&long),
                Body::from(png(4, 4, false)),
            ),
        )
        .await;
        assert_eq!(res.status(), 400);
    }
}
