//! GitHub provider against a loopback mock of the REST API.

use std::collections::BTreeMap;

use loadout_members::GithubTeams;
use loadout_members::github::GithubClient;
use loadout_members::mock_http::MockServer;
use secrecy::SecretString;

fn client(server: &MockServer) -> GithubClient {
    GithubClient::new(
        server.url().to_owned(),
        Ok(SecretString::from("test-token".to_owned())),
    )
}

#[test]
fn active_membership_and_404() {
    let server = MockServer::github("octo", &["acme/engineering"]);
    let c = client(&server);
    assert!(c.is_member("acme", "engineering").unwrap());
    assert!(!c.is_member("acme", "cio").unwrap());

    let reqs = server.requests();
    // /user is fetched once and cached.
    assert_eq!(reqs.iter().filter(|r| r.path == "/user").count(), 1);
    assert!(
        reqs.iter()
            .all(|r| r.authorization.as_deref() == Some("Bearer test-token"))
    );
    assert!(
        reqs.iter()
            .any(|r| r.path == "/orgs/acme/teams/cio/memberships/octo")
    );
}

#[test]
fn pending_membership_is_not_membership() {
    let mut routes = BTreeMap::new();
    routes.insert("/user".into(), (200, r#"{"login":"octo"}"#.into()));
    routes.insert(
        "/orgs/acme/teams/eng/memberships/octo".into(),
        (200, r#"{"state":"pending"}"#.into()),
    );
    let server = MockServer::start(routes);
    assert!(!client(&server).is_member("acme", "eng").unwrap());
}

#[test]
fn bad_token_is_a_provider_error() {
    let mut routes = BTreeMap::new();
    routes.insert(
        "/user".into(),
        (401, r#"{"message":"Bad credentials"}"#.into()),
    );
    let server = MockServer::start(routes);
    let err = client(&server).is_member("acme", "eng").unwrap_err();
    assert!(err.message.contains("HTTP 401"), "{err}");
    assert!(
        !err.to_string().contains("test-token"),
        "token must not leak"
    );
}

#[test]
fn forbidden_membership_lookup_is_an_error() {
    let mut routes = BTreeMap::new();
    routes.insert("/user".into(), (200, r#"{"login":"octo"}"#.into()));
    routes.insert(
        "/orgs/acme/teams/eng/memberships/octo".into(),
        (403, "{}".into()),
    );
    let server = MockServer::start(routes);
    let err = client(&server).is_member("acme", "eng").unwrap_err();
    assert!(err.message.contains("read:org"), "{err}");
}

#[test]
fn discover_uses_env_token_and_api_override() {
    let server = MockServer::github("octo", &["acme/eng"]);
    let url = server.url().to_owned();
    let env = move |k: &str| match k {
        "GH_TOKEN" => Some("env-token".to_owned()),
        "LOADOUT_GITHUB_API" => Some(url.clone()),
        _ => None,
    };
    let tmp = tempfile::tempdir().unwrap();
    let git = loadout_git::fixture::isolated_git(tmp.path());
    let c = GithubClient::discover(&env, &git);
    assert!(c.is_member("acme", "eng").unwrap());
    assert_eq!(
        server.requests()[0].authorization.as_deref(),
        Some("Bearer env-token")
    );
}
