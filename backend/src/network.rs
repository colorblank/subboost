use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    time::Duration,
};
use url::Url;

use crate::config::Network;
use crate::refresh::SubscriptionSnapshot;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestPurpose {
    Content,
    UserInfo,
    GitHubMetadata,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourcePolicy {
    PublicOnly,
    AllowPrivate,
}

#[derive(Debug)]
pub struct FetchResponse {
    pub content: String,
    pub headers: BTreeMap<String, String>,
}

fn v4_in(ip: Ipv4Addr, base: u32, bits: u32) -> bool {
    u32::from(ip) >> (32 - bits) == base >> (32 - bits)
}
fn v6_in(ip: Ipv6Addr, base: u128, bits: u32) -> bool {
    u128::from(ip) >> (128 - bits) == base >> (128 - bits)
}
pub fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ![
            (0x00000000, 8),
            (0x0a000000, 8),
            (0x64400000, 10),
            (0x7f000000, 8),
            (0xa9fe0000, 16),
            (0xac100000, 12),
            (0xc0000000, 24),
            (0xc0000200, 24),
            (0xc0586300, 24),
            (0xc0a80000, 16),
            (0xc6120000, 15),
            (0xc6336400, 24),
            (0xcb007100, 24),
            (0xe0000000, 4),
            (0xf0000000, 4),
        ]
        .iter()
        .any(|&(base, bits)| v4_in(ip, base, bits)),
        IpAddr::V6(ip) => {
            let s = ip.segments();
            if let Some(v4) = ip.to_ipv4_mapped() {
                return is_public_ip(v4.into());
            }
            if s[..6].iter().all(|&v| v == 0) || (s[0] == 0x64 && s[1] == 0xff9b && s[2] == 0) {
                return is_public_ip(
                    Ipv4Addr::new((s[6] >> 8) as u8, s[6] as u8, (s[7] >> 8) as u8, s[7] as u8)
                        .into(),
                );
            }
            if s[0] == 0x2002 {
                return is_public_ip(
                    Ipv4Addr::new((s[1] >> 8) as u8, s[1] as u8, (s[2] >> 8) as u8, s[2] as u8)
                        .into(),
                );
            }
            let raw = u128::from(ip);
            if [
                0x20010001000000000000000000000001u128,
                0x20010001000000000000000000000002,
                0x20010001000000000000000000000003,
            ]
            .contains(&raw)
                || v6_in(ip, 0x20010003000000000000000000000000, 32)
                || v6_in(ip, 0x20010004011200000000000000000000, 48)
                || v6_in(ip, 0x20010020000000000000000000000000, 28)
                || v6_in(ip, 0x20010030000000000000000000000000, 28)
            {
                return true;
            }
            ![
                (0x20010000000000000000000000000000, 23),
                (0x20010db8000000000000000000000000, 32),
                (0x3fff0000000000000000000000000000, 20),
            ]
            .iter()
            .any(|&(base, bits)| v6_in(ip, base, bits))
                && v6_in(ip, 0x20000000000000000000000000000000, 3)
        }
    }
}

pub fn validate_url(input: &str) -> Result<Url> {
    let url = Url::parse(input).context("无效的订阅 URL")?;
    ensure!(
        matches!(url.scheme(), "http" | "https"),
        "只支持 HTTP 或 HTTPS 订阅 URL"
    );
    ensure!(
        url.username().is_empty() && url.password().is_none(),
        "订阅 URL 不允许包含用户名或密码"
    );
    ensure!(url.host_str().is_some(), "订阅 URL 缺少主机名");
    Ok(url)
}

async fn addresses(url: &Url, policy: SourcePolicy, settings: &Network) -> Result<Vec<SocketAddr>> {
    let hostname = url
        .host_str()
        .context("Missing hostname")?
        .trim_matches(['[', ']']);
    let allow_private = policy == SourcePolicy::AllowPrivate;
    ensure!(
        allow_private
            || !(hostname.eq_ignore_ascii_case("localhost")
                || hostname.ends_with(".localhost")
                || hostname.ends_with(".local")),
        "禁止访问本机或内网地址"
    );
    let port = url.port_or_known_default().context("Invalid port")?;
    let mut result: Vec<_> = if let Ok(ip) = hostname.parse::<IpAddr>() {
        vec![SocketAddr::new(ip, port)]
    } else {
        tokio::net::lookup_host((hostname, port))
            .await
            .context("DNS 暂时不可用，请稍后再试")?
            .collect()
    };
    if !allow_private
        && !result.is_empty()
        && result
            .iter()
            .all(|address| matches!(address.ip(), IpAddr::V4(ip) if v4_in(ip,0xc6120000,15)))
    {
        result = resolve_fake_ip(hostname, port, settings).await?;
    }
    ensure!(!result.is_empty(), "DNS 没有返回地址");
    ensure!(
        allow_private || result.iter().all(|address| is_public_ip(address.ip())),
        "禁止访问本机或内网地址"
    );
    Ok(result)
}

pub async fn fetch(
    input: &str,
    user_agent: &str,
    purpose: RequestPurpose,
    policy: SourcePolicy,
    settings: &Network,
) -> Result<FetchResponse> {
    tokio::time::timeout(
        Duration::from_secs(settings.request_timeout_seconds),
        async {
            let mut url = validate_url(input)?;
            for redirects in 0..=settings.maximum_redirects {
                ensure!(
                    url.as_str().len() <= settings.maximum_url_bytes,
                    "订阅 URL 过长"
                );
                let resolved = addresses(&url, policy, settings).await?;
                let host = url
                    .host_str()
                    .context("Missing hostname")?
                    .trim_matches(['[', ']']);
                // Pin validated DNS results and ignore ambient proxies; redirects are validated separately.
                let client = reqwest::Client::builder()
                    .no_proxy()
                    .redirect(reqwest::redirect::Policy::none())
                    .timeout(Duration::from_secs(settings.request_timeout_seconds))
                    .connect_timeout(Duration::from_secs(settings.connect_timeout_seconds))
                    .resolve_to_addrs(host, &resolved)
                    .build()?;
                let request = match purpose {
                    RequestPurpose::Content | RequestPurpose::GitHubMetadata => {
                        client.get(url.clone())
                    }
                    RequestPurpose::UserInfo => client.head(url.clone()),
                };
                let request = if purpose == RequestPurpose::GitHubMetadata {
                    let mut request = request
                        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
                        .header("X-GitHub-Api-Version", "2022-11-28");
                    if url.origin() == validate_url(&settings.github_api_base)?.origin() {
                        if let Ok(token) = std::env::var("GITHUB_TOKEN") {
                            if !token.is_empty() {
                                request = request.bearer_auth(token);
                            }
                        }
                    }
                    request
                } else {
                    request
                };
                let response = request
                    .header(reqwest::header::USER_AGENT, user_agent)
                    .send()
                    .await
                    .context("订阅请求失败")?;
                if response.status().is_redirection() {
                    ensure!(redirects < settings.maximum_redirects, "订阅重定向次数过多");
                    let location = response
                        .headers()
                        .get(reqwest::header::LOCATION)
                        .context("Redirect has no Location")?
                        .to_str()?;
                    url = validate_url(url.join(location)?.as_str())?;
                    continue;
                }
                ensure!(
                    response.status().is_success(),
                    "HTTP {}",
                    response.status().as_u16()
                );
                let headers = response
                    .headers()
                    .iter()
                    .filter_map(|(name, value)| {
                        value
                            .to_str()
                            .ok()
                            .map(|value| (name.to_string(), value.to_owned()))
                    })
                    .collect();
                let content = if purpose == RequestPurpose::UserInfo {
                    String::new()
                } else {
                    bounded_text(response, settings.maximum_response_bytes).await?
                };
                return Ok(FetchResponse { content, headers });
            }
            bail!("订阅重定向次数过多")
        },
    )
    .await
    .context("订阅请求超时")?
}

pub async fn import(request: &Value, policy: SourcePolicy, settings: &Network) -> Result<Value> {
    struct Candidate {
        response: FetchResponse,
        parsed: Value,
        snapshot: SubscriptionSnapshot,
    }
    let input = request["url"]
        .as_str()
        .context("Subscription URL required")?;
    validate_url(input)?;
    let deadline =
        tokio::time::Instant::now() + Duration::from_secs(settings.negotiation_timeout_seconds);
    let mut best: Option<Candidate> = None;
    let mut last_error = None;
    for agent in settings
        .user_agents
        .iter()
        .take(settings.maximum_profile_requests)
    {
        if tokio::time::Instant::now() >= deadline {
            break;
        }
        let response = tokio::time::timeout_at(
            deadline,
            fetch(input, agent, RequestPurpose::Content, policy, settings),
        )
        .await;
        let response = match response {
            Ok(Ok(response)) => response,
            Ok(Err(error)) => {
                last_error = Some(error);
                continue;
            }
            Err(error) => {
                last_error = Some(error.into());
                break;
            }
        };
        let candidate = tokio::task::spawn_blocking(move || -> Result<Candidate> {
            let parsed = crate::parser::parse(&response.content)?;
            let snapshot = SubscriptionSnapshot::new(&parsed);
            ensure!(snapshot.is_usable(), "订阅内容中没有可用节点");
            Ok(Candidate {
                response,
                parsed,
                snapshot,
            })
        })
        .await?;
        match candidate {
            Ok(candidate) => {
                if best.as_ref().is_none_or(|current| {
                    current
                        .snapshot
                        .prefers(&candidate.snapshot, settings.snapshot_comparisons)
                }) {
                    best = Some(candidate);
                }
            }
            Err(error) => last_error = Some(error),
        }
    }
    let mut best =
        best.ok_or_else(|| last_error.unwrap_or_else(|| anyhow::anyhow!("订阅导入失败")))?;
    if (request["userinfoUrl"]
        .as_str()
        .is_some_and(|url| !url.trim().is_empty())
        || request["userinfoUserAgent"]
            .as_str()
            .is_some_and(|agent| !agent.trim().is_empty()))
        && tokio::time::Instant::now() < deadline
    {
        let url = request["userinfoUrl"]
            .as_str()
            .filter(|url| !url.trim().is_empty())
            .unwrap_or(input);
        let agent = request["userinfoUserAgent"]
            .as_str()
            .filter(|agent| !agent.is_empty())
            .unwrap_or(&settings.user_agents[0]);
        // Supplemental metadata failure must not discard a valid subscription snapshot.
        if let Ok(Ok(response)) = tokio::time::timeout_at(
            deadline,
            fetch(url, agent, RequestPurpose::UserInfo, policy, settings),
        )
        .await
        {
            best.response.headers.extend(response.headers);
        }
    }
    Ok(
        json!({"content":best.response.content,"headers":best.response.headers,"parseResult":best.parsed}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn denies_reserved_and_embedded_private_addresses() {
        for ip in [
            "127.0.0.1",
            "10.0.0.1",
            "100.64.0.1",
            "198.18.0.1",
            "192.0.2.1",
            "::1",
            "::ffff:127.0.0.1",
            "64:ff9b::a00:1",
            "2002:7f00:1::",
            "2001:db8::1",
            "3fff::1",
        ] {
            assert!(!is_public_ip(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["8.8.8.8", "1.1.1.1", "2606:4700:4700::1111", "2001:1::1"] {
            assert!(is_public_ip(ip.parse().unwrap()), "{ip}");
        }
        assert!(validate_url("file:///etc/passwd").is_err());
        assert!(validate_url("https://user:pass@example.com/sub").is_err());
    }
    #[tokio::test]
    async fn rejects_localhost_without_contacting_it() {
        let settings = crate::config::Config::default().network;
        assert!(
            fetch(
                "http://localhost:1/",
                "test",
                RequestPurpose::Content,
                SourcePolicy::PublicOnly,
                &settings
            )
            .await
            .expect_err("private address")
            .to_string()
            .contains("内网")
        );
    }
}

async fn bounded_text(mut response: reqwest::Response, maximum_bytes: usize) -> Result<String> {
    ensure!(
        response
            .content_length()
            .is_none_or(|length| length <= maximum_bytes as u64),
        "订阅响应过大"
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            bytes
                .len()
                .checked_add(chunk.len())
                .is_some_and(|length| length <= maximum_bytes),
            "订阅响应过大"
        );
        bytes.extend_from_slice(&chunk);
    }
    String::from_utf8(bytes).context("订阅内容不是 UTF-8")
}

async fn resolve_fake_ip(hostname: &str, port: u16, settings: &Network) -> Result<Vec<SocketAddr>> {
    let endpoint = validate_url(&settings.doh_url)?;
    let host = endpoint.host_str().context("DoH hostname missing")?;
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(settings.request_timeout_seconds))
        .resolve_to_addrs(host, &settings.doh_addresses)
        .build()?;
    let mut addresses = Vec::new();
    for kind in ["A", "AAAA"] {
        let response = client
            .get(endpoint.clone())
            .query(&[("name", hostname), ("type", kind)])
            .header(reqwest::header::ACCEPT, "application/dns-json")
            .send()
            .await?
            .error_for_status()?;
        let text = bounded_text(response, settings.maximum_response_bytes).await?;
        let result: Value = serde_json::from_str(&text)?;
        for answer in result["Answer"].as_array().into_iter().flatten() {
            if let Some(ip) = answer["data"]
                .as_str()
                .and_then(|data| data.parse::<IpAddr>().ok())
            {
                ensure!(is_public_ip(ip), "禁止访问本机或内网地址");
                addresses.push(SocketAddr::new(ip, port));
            }
        }
    }
    ensure!(
        !addresses.is_empty(),
        "Fake-IP DNS recheck returned no public addresses"
    );
    Ok(addresses)
}
