use anyhow::Result;
use axum::{
    http::{HeaderMap, HeaderValue, header},
    response::Response,
};
use chrono::Utc;
use rusqlite::{OptionalExtension, params};

use crate::service::{Service, token};

#[derive(Clone)]
pub(super) struct Identity {
    pub(super) owner: String,
    pub(super) username: String,
    pub(super) guest: bool,
    pub(super) cookie: Option<String>,
}

pub(super) fn append_cookie(response: &mut Response, cookie: &str) -> Result<()> {
    response
        .headers_mut()
        .append(header::SET_COOKIE, HeaderValue::from_str(cookie)?);
    Ok(())
}

pub(super) fn cookie_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .filter_map(|item| item.trim().split_once('='))
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value)
}

pub(super) fn cookie(service: &Service, name: &str, value: &str, seconds: i64) -> String {
    format!(
        "{name}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={seconds}{}",
        if service.config.application.app_url.starts_with("https:") {
            "; Secure"
        } else {
            ""
        }
    )
}

pub(super) async fn identity(service: &Service, headers: &HeaderMap) -> Result<Identity> {
    let authentication = &service.config.authentication;
    let admin = cookie_value(headers, &authentication.administrator_cookie)
        .filter(|v| v.len() == crate::service::TOKEN_LENGTH)
        .map(str::to_owned);
    let guest = cookie_value(headers, &authentication.guest_cookie)
        .filter(|v| v.len() == crate::service::TOKEN_LENGTH)
        .map(str::to_owned);
    for (value, want_guest) in [(admin, false), (guest, true)] {
        if let Some(value) = value {
            let row = service
                .store
                .run(move |db| {
                    Ok(db
                        .query_row(
                            "SELECT owner,username,guest FROM sessions WHERE token=?1 AND expires>?2 AND guest=?3",
                            params![value, Utc::now().timestamp(), want_guest],
                            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, bool>(2)?)),
                        )
                        .optional()?)
                })
                .await?;
            if let Some((owner, username, guest)) = row {
                return Ok(Identity {
                    owner,
                    username,
                    guest,
                    cookie: None,
                });
            }
        }
    }
    let value = token();
    let owner = format!("guest:{}", token());
    let stored = value.clone();
    let stored_owner = owner.clone();
    let session_seconds = authentication.guest_session_seconds;
    service
        .store
        .run(move |db| {
            db.execute("DELETE FROM sessions WHERE expires<=?1", [Utc::now().timestamp()])?;
            db.execute(
                "INSERT INTO sessions(token,owner,username,guest,expires) VALUES(?1,?2,'游客',1,?3)",
                params![stored, stored_owner, Utc::now().timestamp() + session_seconds],
            )?;
            Ok(())
        })
        .await?;
    Ok(Identity {
        owner,
        username: "游客".into(),
        guest: true,
        cookie: Some(cookie(
            service,
            &authentication.guest_cookie,
            &value,
            session_seconds,
        )),
    })
}
