//! API 鉴权与会话身份管理子模块。
//!
//! 负责从 HTTP Cookie 中提取会话令牌、校验管理员身份，或自动签发受限访客（guest）临时身份。

use anyhow::Result;
use axum::{
    http::{HeaderMap, HeaderValue, header},
    response::Response,
};
use chrono::Utc;
use rusqlite::{OptionalExtension, params};

use crate::service::{Service, token};

use super::csrf::is_https_request;

/// 请求发送者的认证身份上下文。
#[derive(Clone)]
pub(super) struct Identity {
    /// 资源归属者标识（管理员为用户名，访客为 `guest:<token>` 格式）。
    pub(super) owner: String,
    /// 登录用户名或访客显示名。
    pub(super) username: String,
    /// 是否为受限访客会话。
    pub(super) guest: bool,
    /// 新生成的 Set-Cookie 字符串（如新签发访客令牌时附带）。
    pub(super) cookie: Option<String>,
}

/// 向响应头部追加 `Set-Cookie` 指令。
pub(super) fn append_cookie(response: &mut Response, cookie: &str) -> Result<()> {
    response
        .headers_mut()
        .append(header::SET_COOKIE, HeaderValue::from_str(cookie)?);
    Ok(())
}

/// 从请求头部中按名称检索 Cookie 值。
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

/// 构造包含标准安全选项（HttpOnly, SameSite=Lax, Secure）的 Cookie 字符串。
pub(super) fn cookie(
    service: &Service,
    headers: &HeaderMap,
    name: &str,
    value: &str,
    seconds: i64,
) -> String {
    format!(
        "{name}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={seconds}{}",
        if is_https_request(service, headers) {
            "; Secure"
        } else {
            ""
        }
    )
}

/// 解析当前请求的身份，若管理员 Cookie 有效则确认为管理员，否则使用或生成访客身份。
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
            headers,
            &authentication.guest_cookie,
            &value,
            session_seconds,
        )),
    })
}
