//! FR-010 AC05/AC06/AC11 and B10-31～B10-46, B10-60～B10-64 over real HTTP:
//! pairing, code expiry and failures, rate limits, the 8-session limit,
//! same-IP and changed-IP re-pairing, admission (peer, Host, Origin, Cookie,
//! IP binding, issuer), expiry, revocation, whitelist and transport errors.

mod common;

use common::*;
use dgos_db::LanAuditKind;
use dgos_lan::{RevokeReason, SpaFiles, MAX_SESSIONS};
use serde_json::json;

fn ip(n: u8) -> String {
    format!("192.168.1.{n}")
}

#[tokio::test]
async fn a_valid_code_pairs_once_and_sets_an_httponly_strict_cookie() {
    let env = env().await;
    let code = env.code();
    let reply = env
        .post("/api/v1/lan/pair", json!({ "code": code }))
        .user_agent("Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit Version/17.0 Mobile Safari/604.1")
        .send()
        .await;
    assert_eq!(reply.status, 200);
    let set_cookie = reply.headers["set-cookie"].to_str().unwrap().to_owned();
    assert!(set_cookie.starts_with("dgos_lan="));
    for attribute in ["HttpOnly", "SameSite=Strict", "Path=/"] {
        assert!(set_cookie.contains(attribute), "{set_cookie}");
    }
    for absent in ["Secure", "Max-Age", "Expires", "Domain"] {
        assert!(!set_cookie.contains(absent), "{set_cookie}");
    }
    // The body is the session status; the token is only in the Cookie.
    let token = reply
        .cookie()
        .unwrap()
        .trim_start_matches("dgos_lan=")
        .to_owned();
    assert!(!reply.text.contains(&token));
    assert_eq!(reply.body["device_label"], "iPhone · Safari");
    let now = 1_800_000_000;
    assert_eq!(reply.body["created_at"], now);
    assert_eq!(reply.body["expires_at"], now + 12 * 3600);
    assert_eq!(reply.body["idle_expires_at"], now + 30 * 60);
    assert!(reply.body["short_id"].as_str().unwrap().len() == 8);
    // No CORS, ever.
    assert!(reply
        .headers
        .keys()
        .all(|name| !name.as_str().starts_with("access-control-")));

    // Single use: the same code again is "invalid or used".
    let again = env
        .post("/api/v1/lan/pair", json!({ "code": code }))
        .peer(&ip(11))
        .send()
        .await;
    assert_eq!((again.status, again.code()), (401, Some(1010004)));

    let cookie = reply.cookie().unwrap();
    let session = env.get("/api/v1/lan/session").cookie(&cookie).send().await;
    assert_eq!(session.status, 200);
    assert_eq!(session.headers["cache-control"], "no-store");
    assert_eq!(env.core.list_devices().len(), 1);
    assert_eq!(env.core.list_devices()[0].peer_ip, PEER);
}

#[tokio::test]
async fn codes_are_case_and_dash_insensitive_and_wrong_codes_are_rejected() {
    let env = env().await;
    let code = env.code();
    let wrong = env
        .post(
            "/api/v1/lan/pair",
            json!({ "code": "0000-0000-0000-0000-0000-0000-00" }),
        )
        .send()
        .await;
    assert_eq!((wrong.status, wrong.code()), (401, Some(1010004)));
    assert_eq!(wrong.body["message"], "invalid_access_token");
    let garbage = env
        .post("/api/v1/lan/pair", json!({ "code": "not a code" }))
        .send()
        .await;
    assert_eq!(garbage.code(), Some(1010004));
    // A wrong attempt does not void the code; the right one (lower case, no dashes) works.
    let relaxed = code.replace('-', "").to_lowercase();
    let ok = env
        .post("/api/v1/lan/pair", json!({ "code": relaxed }))
        .send()
        .await;
    assert_eq!(ok.status, 200);
}

#[tokio::test]
async fn an_expired_code_is_1010005_and_then_gone() {
    let env = env().await;
    let code = env.code();
    env.clock.advance(10 * 60);
    let expired = env
        .post("/api/v1/lan/pair", json!({ "code": code }))
        .send()
        .await;
    assert_eq!((expired.status, expired.code()), (401, Some(1010005)));
    assert_eq!(expired.body["message"], "token_expired");
    let again = env
        .post("/api/v1/lan/pair", json!({ "code": code }))
        .send()
        .await;
    assert_eq!(again.code(), Some(1010004));
}

#[tokio::test]
async fn ten_failures_void_the_code_and_a_new_code_voids_the_old_one() {
    let env = env().await;
    let code = env.code();
    for attempt in 0..10u8 {
        let reply = env
            .post("/api/v1/lan/pair", json!({ "code": "0".repeat(26) }))
            .peer(&ip(20 + attempt / 4))
            .send()
            .await;
        assert_eq!(reply.code(), Some(1010004));
    }
    let late = env
        .post("/api/v1/lan/pair", json!({ "code": code }))
        .peer(&ip(40))
        .send()
        .await;
    assert_eq!(late.code(), Some(1010004), "voided after 10 failures");

    let first = env.code();
    let second = env.code();
    let old = env
        .post("/api/v1/lan/pair", json!({ "code": first }))
        .peer(&ip(41))
        .send()
        .await;
    assert_eq!(old.code(), Some(1010004), "replaced by the newer code");
    let new = env
        .post("/api/v1/lan/pair", json!({ "code": second }))
        .peer(&ip(41))
        .send()
        .await;
    assert_eq!(new.status, 200);
    let reasons: Vec<_> = env
        .audit
        .take()
        .into_iter()
        .filter(|r| r.event == LanAuditKind::CodeVoided)
        .map(|r| r.reason.unwrap())
        .collect();
    assert_eq!(reasons, ["too_many_failures", "replaced"]);
}

#[tokio::test]
async fn pairing_is_limited_per_ip_and_globally_with_retry_after() {
    let env = env().await;
    let _code = env.code();
    for _ in 0..5 {
        let reply = env
            .post("/api/v1/lan/pair", json!({ "code": "x" }))
            .send()
            .await;
        assert_eq!(reply.code(), Some(1010004));
    }
    env.clock.advance(20);
    let limited = env
        .post("/api/v1/lan/pair", json!({ "code": "x" }))
        .send()
        .await;
    assert_eq!((limited.status, limited.code()), (429, Some(1010011)));
    assert_eq!(
        limited.body["detail"],
        json!({"scope": "pair_ip", "retry_after_seconds": 40})
    );
    assert_eq!(limited.headers["retry-after"], "40");
    // Rejected requests do not extend the window.
    env.clock.advance(40);
    let reopened = env
        .post("/api/v1/lan/pair", json!({ "code": "x" }))
        .send()
        .await;
    assert_eq!(reopened.code(), Some(1010004));

    // Global: 20 per minute from any sources.
    env.clock.advance(61);
    for n in 0..20u8 {
        let reply = env
            .post("/api/v1/lan/pair", json!({ "code": "x" }))
            .peer(&ip(100 + n))
            .send()
            .await;
        assert_eq!(reply.status, 401);
    }
    let global = env
        .post("/api/v1/lan/pair", json!({ "code": "x" }))
        .peer(&ip(200))
        .send()
        .await;
    assert_eq!(global.status, 429);
    assert_eq!(global.body["detail"]["scope"], "pair_global");
    assert_eq!(global.body["detail"]["retry_after_seconds"], 60);
    assert!(env
        .audit
        .take()
        .iter()
        .any(|r| r.event == LanAuditKind::PairRateLimited && r.reason == Some("pair_global")));
}

#[tokio::test]
async fn the_ninth_device_is_refused_without_consuming_the_code() {
    let env = env().await;
    let mut cookies = Vec::new();
    for n in 0..MAX_SESSIONS as u8 {
        cookies.push(env.paired(&ip(10 + n)).await);
    }
    assert_eq!(env.core.session_count(), 8);
    let code = env.code();
    let ninth = env
        .post("/api/v1/lan/pair", json!({ "code": code }))
        .peer(&ip(50))
        .send()
        .await;
    assert_eq!((ninth.status, ninth.code()), (429, Some(1010011)));
    assert_eq!(ninth.body["detail"], json!({"scope": "session_limit"}));
    assert!(ninth.headers.get("retry-after").is_none());
    // Room again: the same code still works (it was not consumed or counted as a failure).
    let first = env.core.list_devices()[0].session_id.clone();
    assert!(env.core.disconnect(&first).await);
    let retry = env
        .post("/api/v1/lan/pair", json!({ "code": code }))
        .peer(&ip(50))
        .send()
        .await;
    assert_eq!(retry.status, 200);
}

#[tokio::test]
async fn the_same_device_re_pairs_at_the_limit_and_keeps_its_session_on_a_wrong_code() {
    let env = env().await;
    let mut cookies = Vec::new();
    for n in 0..MAX_SESSIONS as u8 {
        cookies.push(env.paired(&ip(10 + n)).await);
    }
    let mine = cookies[3].clone();
    // Wrong code: 401, the old session is untouched (D101).
    let wrong = env
        .post("/api/v1/lan/pair", json!({ "code": "0".repeat(26) }))
        .peer(&ip(13))
        .cookie(&mine)
        .send()
        .await;
    assert_eq!(wrong.code(), Some(1010004));
    assert_eq!(
        env.get("/api/v1/lan/session")
            .peer(&ip(13))
            .cookie(&mine)
            .send()
            .await
            .status,
        200
    );
    // Right code: not limited, replaces the old session, count stays 8.
    let code = env.code();
    let again = env
        .post("/api/v1/lan/pair", json!({ "code": code }))
        .peer(&ip(13))
        .cookie(&mine)
        .send()
        .await;
    assert_eq!(again.status, 200, "{}", again.text);
    assert_eq!(env.core.session_count(), 8);
    let old = env
        .get("/api/v1/lan/session")
        .peer(&ip(13))
        .cookie(&mine)
        .send()
        .await;
    assert_eq!((old.status, old.code()), (401, Some(1010006)));
    let new = again.cookie().unwrap();
    assert_eq!(
        env.get("/api/v1/lan/session")
            .peer(&ip(13))
            .cookie(&new)
            .send()
            .await
            .status,
        200
    );
    assert!(env
        .audit
        .take()
        .iter()
        .any(|r| r.event == LanAuditKind::SessionRevoked
            && r.reason == Some(RevokeReason::RePaired.as_str())));
}

#[tokio::test]
async fn a_device_on_a_new_ip_loses_its_old_session_before_the_limit_check() {
    let env = env().await;
    let mut cookies = Vec::new();
    for n in 0..MAX_SESSIONS as u8 {
        cookies.push(env.paired(&ip(10 + n)).await);
    }
    // At the limit, the device that moved to .60 pairs: its old session is
    // revoked first (ip_changed), so the limit is not hit (D105).
    let code = env.code();
    let moved = env
        .post("/api/v1/lan/pair", json!({ "code": code }))
        .peer(&ip(60))
        .cookie(&cookies[0])
        .send()
        .await;
    assert_eq!(moved.status, 200, "{}", moved.text);
    assert_eq!(env.core.session_count(), 8);
    assert!(env
        .audit
        .take()
        .iter()
        .any(|r| r.event == LanAuditKind::SessionRevoked
            && r.reason == Some("ip_changed")
            && r.peer_ip == Some(ip(10).parse().unwrap())));

    // A wrong code after moving: the old session is gone anyway.
    let wrong = env
        .post("/api/v1/lan/pair", json!({ "code": "0".repeat(26) }))
        .peer(&ip(61))
        .cookie(&cookies[1])
        .send()
        .await;
    assert_eq!(wrong.code(), Some(1010004));
    assert_eq!(env.core.session_count(), 7);
    let gone = env
        .get("/api/v1/lan/session")
        .peer(&ip(11))
        .cookie(&cookies[1])
        .send()
        .await;
    assert_eq!(gone.code(), Some(1010006));
}

#[tokio::test]
async fn sessions_expire_after_30_idle_minutes_or_12_hours_without_renewal() {
    let env = env().await;
    let cookie = env.paired(PEER).await;
    env.clock.advance(29 * 60);
    let alive = env.get("/api/v1/lan/session").cookie(&cookie).send().await;
    assert_eq!(alive.status, 200);
    assert_eq!(
        alive.body["idle_expires_at"],
        1_800_000_000 + 29 * 60 + 30 * 60
    );
    env.clock.advance(30 * 60);
    let idle = env.get("/api/v1/lan/session").cookie(&cookie).send().await;
    assert_eq!((idle.status, idle.code()), (401, Some(1010006)));
    assert!(idle.headers["set-cookie"]
        .to_str()
        .unwrap()
        .contains("Max-Age=0"));

    let cookie = env.paired(PEER).await;
    // Active every 20 minutes still ends at 12 hours.
    for _ in 0..35 {
        env.clock.advance(20 * 60);
        assert_eq!(
            env.get("/api/v1/lan/session")
                .cookie(&cookie)
                .send()
                .await
                .status,
            200
        );
    }
    env.clock.advance(20 * 60);
    let absolute = env.get("/api/v1/lan/session").cookie(&cookie).send().await;
    assert_eq!(absolute.status, 401);
    let reasons: Vec<_> = env
        .audit
        .take()
        .into_iter()
        .filter(|r| r.event == LanAuditKind::SessionRevoked)
        .map(|r| r.reason.unwrap())
        .collect();
    assert_eq!(reasons, ["idle_expired", "absolute_expired"]);
}

#[tokio::test]
async fn the_sweep_removes_expired_sessions_and_codes() {
    let env = env().await;
    env.paired(PEER).await;
    let _code = env.code();
    assert!(env.core.code_expires_at().is_some());
    env.clock.advance(31 * 60);
    assert!(
        env.core.list_devices().is_empty(),
        "expired sessions are not listed"
    );
    env.core.sweep().await;
    assert_eq!(env.core.code_expires_at(), None);
    let records = env.audit.take();
    assert!(records
        .iter()
        .any(|r| r.event == LanAuditKind::SessionRevoked && r.reason == Some("idle_expired")));
    assert!(records
        .iter()
        .any(|r| r.event == LanAuditKind::CodeVoided && r.reason == Some("expired")));
}

#[tokio::test]
async fn logout_of_an_expired_session_is_recorded_as_its_expiry() {
    let env = env().await;
    let cookie = env.paired(PEER).await;
    env.audit.take();
    // Idle past 30 minutes, before the sweep runs.
    env.clock.advance(31 * 60);
    let out = env
        .post("/api/v1/lan/logout", json!({}))
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(out.status, 204);
    let reasons: Vec<_> = env
        .audit
        .take()
        .into_iter()
        .filter(|r| r.event == LanAuditKind::SessionRevoked)
        .map(|r| r.reason.unwrap())
        .collect();
    assert_eq!(reasons, ["idle_expired"]);
}

#[tokio::test]
async fn a_session_used_from_another_ip_is_revoked() {
    let env = env().await;
    let cookie = env.paired(PEER).await;
    let elsewhere = env
        .get("/api/v1/lan/session")
        .peer("192.168.1.99")
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(elsewhere.code(), Some(1010006));
    let home = env.get("/api/v1/lan/session").cookie(&cookie).send().await;
    assert_eq!(home.code(), Some(1010006), "revoked, not merely refused");
}

#[tokio::test]
async fn sessions_follow_the_issuing_desktop_user() {
    let env = env().await;
    let cookie = env.paired(PEER).await;
    // Switching (resolver closed): refused and revoked (B10-38).
    env.issuer.set(None);
    assert_eq!(env.core.issue_code().map(|c| c.code), None);
    let switching = env.get("/api/v1/lan/session").cookie(&cookie).send().await;
    assert_eq!(switching.code(), Some(1010006));
    env.issuer.set(Some(ALICE));
    let back = env.get("/api/v1/lan/session").cookie(&cookie).send().await;
    assert_eq!(back.code(), Some(1010006));

    // Another user signed in: refused.
    let cookie = env.paired(PEER).await;
    env.issuer.set(Some(BOB));
    assert_eq!(
        env.get("/api/v1/lan/session")
            .cookie(&cookie)
            .send()
            .await
            .code(),
        Some(1010006)
    );

    // FR-008 step 4: revoke_issued_by ends the user's sessions and unused code.
    env.issuer.set(Some(ALICE));
    let a = env.paired(PEER).await;
    let b = env.paired("192.168.1.12").await;
    let code = env.code();
    env.issuer.set(Some(BOB));
    let bob = env.paired("192.168.1.13").await;
    env.core.revoke_issued_by(ALICE).await;
    env.issuer.set(Some(ALICE));
    for (peer, cookie) in [(PEER, &a), ("192.168.1.12", &b)] {
        assert_eq!(
            env.get("/api/v1/lan/session")
                .peer(peer)
                .cookie(cookie)
                .send()
                .await
                .code(),
            Some(1010006)
        );
    }
    let used = env
        .post("/api/v1/lan/pair", json!({ "code": code }))
        .peer("192.168.1.14")
        .send()
        .await;
    assert_eq!(used.code(), Some(1010004), "alice's unused code is void");
    env.issuer.set(Some(BOB));
    assert_eq!(
        env.get("/api/v1/lan/session")
            .peer("192.168.1.13")
            .cookie(&bob)
            .send()
            .await
            .status,
        200,
        "bob's session is not alice's"
    );
}

#[tokio::test]
async fn logout_and_host_disconnect_revoke_and_clear_the_cookie() {
    let env = env().await;
    let cookie = env.paired(PEER).await;
    let out = env
        .post("/api/v1/lan/logout", json!({}))
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(out.status, 204);
    assert!(out.text.is_empty());
    assert!(out.headers["set-cookie"]
        .to_str()
        .unwrap()
        .contains("Max-Age=0"));
    assert_eq!(
        env.get("/api/v1/lan/session")
            .cookie(&cookie)
            .send()
            .await
            .code(),
        Some(1010006)
    );
    // Without a session: still 204.
    assert_eq!(
        env.post("/api/v1/lan/logout", json!({}))
            .send()
            .await
            .status,
        204
    );

    let a = env.paired(PEER).await;
    let b = env.paired("192.168.1.12").await;
    let id = env.core.list_devices()[0].session_id.clone();
    assert!(env.core.disconnect(&id).await);
    assert!(
        !env.core.disconnect(&id).await,
        "second disconnect is a no-op"
    );
    assert!(!env.core.disconnect("unknown").await);
    assert_eq!(
        env.core.revoke_all(RevokeReason::HostDisconnectAll).await,
        1
    );
    for (peer, cookie) in [(PEER, &a), ("192.168.1.12", &b)] {
        assert_eq!(
            env.get("/api/v1/lan/session")
                .peer(peer)
                .cookie(cookie)
                .send()
                .await
                .status,
            401
        );
    }
}

#[tokio::test]
async fn peer_host_and_origin_are_checked_before_anything_else() {
    let env = env().await;
    let cookie = env.paired(PEER).await;
    let ok = env.get("/api/v1/lan/session").cookie(&cookie).send().await;
    assert_eq!(ok.status, 200);
    for peer in ["8.8.8.8", "127.0.0.1", "169.254.3.4", "100.64.0.1"] {
        let reply = env
            .get("/api/v1/lan/session")
            .peer(peer)
            .cookie(&cookie)
            .send()
            .await;
        assert_eq!((reply.status, reply.code()), (403, Some(1010007)), "{peer}");
    }
    let addr = env.addr;
    for host in [
        "evil.example".to_owned(),
        format!("evil.example:{}", addr.port()),
        addr.ip().to_string(),
        format!("localhost:{}", addr.port()),
    ] {
        let reply = env.get("/").host(&host).send().await;
        assert_eq!((reply.status, reply.code()), (403, Some(1010007)), "{host}");
    }
    // State-changing requests need the exact origin; null and missing are refused.
    for origin in [
        None,
        Some("null"),
        Some("http://evil.example"),
        Some("https://127.0.0.1"),
    ] {
        let reply = env
            .post("/api/v1/lan/logout", json!({}))
            .origin(origin)
            .cookie(&cookie)
            .send()
            .await;
        assert_eq!(reply.code(), Some(1010007), "{origin:?}");
    }
    // A GET with a foreign Origin is refused too; without one it passes (navigation).
    let foreign = env
        .get("/api/v1/lan/session")
        .origin(Some("http://evil.example"))
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(foreign.code(), Some(1010007));
    // The session survived all of that.
    assert_eq!(
        env.get("/api/v1/lan/session")
            .cookie(&cookie)
            .send()
            .await
            .status,
        200
    );
    let reasons: Vec<_> = env
        .audit
        .take()
        .into_iter()
        .filter(|r| r.event == LanAuditKind::RequestRejected)
        .map(|r| r.reason.unwrap())
        .collect();
    assert!(reasons.contains(&"peer_not_private"));
    assert!(reasons.contains(&"host_mismatch"));
    assert!(reasons.contains(&"origin_mismatch"));
}

#[tokio::test]
async fn host_admission_reaches_loopback_with_an_environment_proxy() {
    let proxy = Upstream::start().await;
    let mut child = tokio::process::Command::new(std::env::current_exe().unwrap());
    child.args([
        "--exact",
        "peer_host_and_origin_are_checked_before_anything_else",
        "--nocapture",
    ]);
    for key in [
        "HTTP_PROXY",
        "http_proxy",
        "HTTPS_PROXY",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
    ] {
        child.env(key, format!("http://{}", proxy.addr));
    }
    // Only the child gets proxy settings; parallel tests keep their own environment.
    child.env("NO_PROXY", "").env("no_proxy", "").kill_on_drop(true);
    let output = tokio::time::timeout(std::time::Duration::from_secs(60), child.output())
        .await
        .expect("host admission child timed out")
        .unwrap();
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert_eq!(proxy.hits(), 0, "host admission requests escaped loopback");
}

#[tokio::test]
async fn only_whitelisted_routes_exist_and_errors_have_no_cors() {
    let env = env().await;
    let cookie = env.paired(PEER).await;
    for path in [
        "/api/v1/providers",
        "/api/providers",
        "/api/v1/lan/accounts",
        "/api/v1/lan/chat/sessions/x/rename",
        "/api/v1/lan/mcp/tools",
        "/api/chat/send",
        "/api/v1/auth/login",
    ] {
        let reply = env.call("POST", path).cookie(&cookie).send().await;
        assert_eq!((reply.status, reply.text.as_str()), (404, ""), "{path}");
    }
    // Registered paths, other methods: 405 with an empty body (no OPTIONS preflight).
    for (method, path) in [
        ("GET", "/api/v1/lan/pair"),
        ("DELETE", "/api/v1/lan/chat/sessions"),
        ("PUT", "/api/v1/lan/chat/sessions"),
        ("OPTIONS", "/api/v1/lan/pair"),
        ("PATCH", "/api/v1/lan/session"),
    ] {
        let reply = env
            .call(method, path)
            .origin(Some(&env.origin()))
            .cookie(&cookie)
            .send()
            .await;
        assert_eq!(
            (reply.status, reply.text.as_str()),
            (405, ""),
            "{method} {path}"
        );
        assert!(reply
            .headers
            .keys()
            .all(|name| !name.as_str().starts_with("access-control-")));
    }
    // The session routes need a session.
    for path in [
        "/api/v1/lan/session",
        "/api/v1/lan/targets",
        "/api/v1/lan/chat/sessions",
    ] {
        let reply = env.get(path).send().await;
        assert_eq!(reply.code(), Some(1010006), "{path}");
    }
}

#[tokio::test]
async fn locale_bootstrap_is_public_but_returns_only_locale_metadata() {
    let env = env().await;
    let reply = env.get("/api/v1/lan/bootstrap").send().await;
    assert_eq!(reply.status, 200);
    assert_eq!(reply.body["locale"], "zh-CN");
    assert_eq!(reply.body["catalog_version"], "v1");
    assert!(reply.body.get("preference").is_none());
    assert!(reply.body.get("user_id").is_none());
}

#[tokio::test]
async fn transport_errors_are_empty_400_413_415() {
    let env = env().await;
    let pair = |body: Vec<u8>, kind: &str| env.call("POST", "/api/v1/lan/pair").raw(kind, body);
    let reply = pair(br#"{"code":"x"}"#.to_vec(), "text/plain").send().await;
    assert_eq!((reply.status, reply.text.as_str()), (415, ""));
    let reply = pair(
        br#"{"code":"x","user_id":"bob"}"#.to_vec(),
        "application/json",
    )
    .send()
    .await;
    assert_eq!((reply.status, reply.text.as_str()), (400, ""));
    let reply = pair(b"{".to_vec(), "application/json").send().await;
    assert_eq!((reply.status, reply.text.as_str()), (400, ""));
    let big = format!(r#"{{"code":"{}"}}"#, "A".repeat(2000)).into_bytes();
    let reply = pair(big, "application/json").send().await;
    assert_eq!((reply.status, reply.text.as_str()), (413, ""));
}

#[tokio::test]
async fn the_spa_is_served_without_a_session_with_a_strict_csp() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("assets")).unwrap();
    std::fs::write(
        dir.path().join("index.html"),
        "<!doctype html><title>DG-OS</title>",
    )
    .unwrap();
    std::fs::write(dir.path().join("assets/app-1.js"), "console.log(1)").unwrap();
    let env = env_with_spa(Some(SpaFiles::new(dir.path()))).await;
    for path in ["/", "/chat", "/some/client/route"] {
        let reply = env.get(path).send().await;
        assert_eq!(reply.status, 200, "{path}");
        assert!(reply.text.contains("DG-OS"));
        let csp = reply.headers["content-security-policy"].to_str().unwrap();
        assert!(csp.contains("default-src 'self'"));
        assert!(csp.contains(&format!("connect-src 'self' ws://{}", env.addr)));
        assert!(csp.contains("frame-ancestors 'none'"));
        assert_eq!(reply.headers["x-frame-options"], "DENY");
        assert_eq!(reply.headers["x-content-type-options"], "nosniff");
        assert_eq!(reply.headers["referrer-policy"], "no-referrer");
    }
    let js = env.get("/assets/app-1.js").send().await;
    assert_eq!(js.status, 200);
    assert!(js.headers["content-type"]
        .to_str()
        .unwrap()
        .starts_with("text/javascript"));
    assert_eq!(env.get("/assets/missing.js").send().await.status, 404);
    assert_eq!(env.get("/assets/..%2Findex.html").send().await.status, 404);
    // Non-GET on the SPA: 405.
    let post = env.post("/", json!({})).send().await;
    assert_eq!(post.status, 405);
}
