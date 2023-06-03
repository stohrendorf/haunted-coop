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
