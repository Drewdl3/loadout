//! GitLab provider against a loopback mock of the REST API.

use std::collections::BTreeMap;

use loadout_members::GitlabGroups;
use loadout_members::gitlab::GitlabClient;
use loadout_members::mock_http::MockServer;
use secrecy::SecretString;

fn server() -> MockServer {
    let mut routes = BTreeMap::new();
    routes.insert(
        "/user".into(),
        (200, r#"{"id":42,"username":"octo"}"#.into()),
    );
    routes.insert(
        "/groups/acme%2Fpayments/members/all/42".into(),
        (200, r#"{"id":42,"state":"active"}"#.into()),
    );
    routes.insert(
        "/groups/acme%2Fblocked/members/all/42".into(),
        (200, r#"{"id":42,"state":"blocked"}"#.into()),
    );
    routes.insert(
        "/groups/acme%2Fsecret/members/all/42".into(),
        (403, "{}".into()),
    );
    MockServer::start(routes)
}

#[test]
fn membership_via_api() {
    let s = server();
    let c = GitlabClient::new(
        s.url().to_owned(),
        Ok(SecretString::from("glpat-test".to_owned())),
    );
    assert!(c.is_member("acme/payments").unwrap());
    assert!(!c.is_member("acme/other").unwrap(), "404 → not a member");
    assert!(!c.is_member("acme/blocked").unwrap());
    let e = c.is_member("acme/secret").unwrap_err();
    assert!(e.message.contains("HTTP 403"), "{e}");
    let reqs = s.requests();
    assert_eq!(reqs.iter().filter(|r| r.path == "/user").count(), 1);
    assert!(
        reqs.iter()
            .all(|r| r.authorization.as_deref() == Some("Bearer glpat-test"))
    );
}
