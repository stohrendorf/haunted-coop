use reqwest::{Client, Error};
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Debug)]
pub struct SuccessResponse {
    pub message: String,
    pub success: bool,
}

#[derive(Serialize, Debug)]
pub struct SessionAccessRequest {
    pub api_key: String,
    pub auth_token: String,
    pub session_id: String,
    pub username: String,
}

#[derive(Serialize, Debug)]
pub struct SessionPlayers {
    pub session_id: String,
    pub usernames: Vec<String>,
}

#[derive(Serialize, Debug)]
pub struct SessionsPlayersRequest {
    pub api_key: String,
    pub sessions: Vec<SessionPlayers>,
}

/// Sends a request to Haunted Manager to verify that a user is allowed to log in.
///
/// # Arguments
///
/// * `base_url`: The base URL of Haunted Manager.
/// * `api_key`: The API key for the user stored in Haunted Manager.
/// * `auth_token`: The Auth Token stored for the user in Haunted Manager.
/// * `session_id`: The session ID checked against for access.
/// * `username`: The user name of the user.
pub async fn check_permission(
    base_url: &str,
    api_key: &str,
    auth_token: &str,
    session_id: &str,
    username: &str,
) -> Result<SuccessResponse, Error> {
    let url = format!("{base_url}/api/v0/sessions/check-access");
    let session_id: Vec<&str> = session_id.split('/').take(1).collect();
    if session_id.is_empty() {
        return Ok(SuccessResponse {
            message: "missing session id".into(),
            success: false,
        });
    }
    Client::builder()
        .build()?
        .post(url)
        .json(&SessionAccessRequest {
            api_key: api_key.into(),
            auth_token: auth_token.into(),
            session_id: session_id[0].into(),
            username: username.into(),
        })
        .send()
        .await?
        .json::<SuccessResponse>()
        .await
}

/// Sends the currently active users to the backend.
///
/// # Arguments
///
/// * `base_url`: The base URL of the Haunted Manager backed to send requests to.
/// * `request`: The request to send to the backend, containing the currently logged in players
///              in each session.
pub async fn update_sessions_players(
    base_url: &str,
    request: &SessionsPlayersRequest,
) -> Result<(), Error> {
    let url = format!("{base_url}/api/v0/sessions/session-players");
    Client::builder()
        .build()?
        .post(url)
        .json(&request)
        .send()
        .await?;
    Ok(())
}
