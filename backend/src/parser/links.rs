use super::*;
use std::collections::BTreeMap;

pub(super) type Params = BTreeMap<String, String>;
pub(super) fn pick<'a>(params: &'a Params, keys: &[&str]) -> &'a str {
    keys.iter()
        .find_map(|key| params.get(*key).filter(|s| !s.is_empty()))
        .map(String::as_str)
        .unwrap_or("")
}
pub(super) fn query(raw: &str) -> Params {
    url::form_urlencoded::parse(raw.as_bytes())
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
}
pub(super) fn headers(raw: &str) -> Option<Value> {
    if let Ok(value) = serde_json::from_str::<Value>(raw) {
        return value
            .as_object()
            .filter(|m| m.values().all(Value::is_string))
            .map(|_| value.clone());
    }
    let map: Map<String, Value> = raw
        .split('|')
        .filter_map(|part| part.split_once(':'))
        .filter(|(k, _)| !k.trim().is_empty())
        .map(|(k, v)| (k.trim().to_owned(), json!(v.trim())))
        .collect();
    if map.is_empty() {
        None
    } else {
        Some(Value::Object(map))
    }
}
struct Uri {
    auth: String,
    host: String,
    port: String,
    path: String,
    params: Params,
    name: String,
}
fn uri(raw: &str) -> Result<Uri> {
    let (raw, name) = raw.split_once('#').unwrap_or((raw, ""));
    let (raw, qs) = raw.split_once('?').unwrap_or((raw, ""));
    let (authority, path) = raw.split_once('/').unwrap_or((raw, ""));
    let (auth, hostport) = authority.rsplit_once('@').unwrap_or(("", authority));
    let (host, port) = endpoint(hostport)?;
    Ok(Uri {
        auth: decode_percent(auth, false),
        host,
        port,
        path: if path.is_empty() {
            "/".into()
        } else {
            format!("/{path}")
        },
        params: query(qs),
        name: decode_percent(name, true),
    })
}
pub(super) fn endpoint(raw: &str) -> Result<(String, String)> {
    let raw = raw.trim();
    if let Some(raw) = raw.strip_prefix('[') {
        let (host, rest) = raw
            .split_once(']')
            .ok_or_else(|| anyhow!("无效的 IPv6 地址"))?;
        if !rest.is_empty() && !rest.starts_with(':') {
            bail!("无效的服务器地址");
        }
        return Ok((host.to_owned(), rest.trim_start_matches(':').to_owned()));
    }
    if let Some((host, port)) = raw.rsplit_once(':') {
        if host.contains(':') {
            bail!("IPv6 地址必须使用方括号");
        }
        Ok((host.to_owned(), port.to_owned()))
    } else {
        Ok((raw.to_owned(), String::new()))
    }
}
fn set_text(node: &mut Value, field: &str, params: &Params, keys: &[&str]) {
    let value = pick(params, keys);
    if !value.is_empty() {
        node[field] = json!(value);
    }
}
fn set_bool(node: &mut Value, field: &str, params: &Params, keys: &[&str]) {
    if let Some(value) = keys
        .iter()
        .find_map(|key| params.get(*key).and_then(|s| boolish(&json!(s))))
    {
        node[field] = json!(value);
    }
}
fn set_integer(node: &mut Value, field: &str, params: &Params, keys: &[&str]) -> Result<()> {
    let raw = pick(params, keys);
    if !raw.is_empty() {
        let n = integer(&json!(raw))
            .filter(|n| *n <= 9_007_199_254_740_991)
            .ok_or_else(|| anyhow!("{field} 需要非负整数"))?;
        node[field] = json!(n);
    }
    Ok(())
}
fn password(auth: &str, params: &Params) -> String {
    if !auth.is_empty() {
        auth.to_owned()
    } else {
        pick(params, &["password", "auth", "auth_str", "auth-str", "psk"]).to_owned()
    }
}

pub(super) fn parse_link(input: &str) -> Result<Option<Value>> {
    let Some((scheme, raw)) = input.split_once("://") else {
        if input.contains(':')
            && (input.contains('@')
                || input
                    .split(':')
                    .nth(1)
                    .is_some_and(|v| v.parse::<u16>().is_ok()))
        {
            bail!("无法识别的代理格式，请添加协议前缀 (如 socks5://, http://)");
        }
        return Ok(None);
    };
    let scheme = scheme.to_ascii_lowercase();
    if scheme == "tg" || ((scheme == "https" || scheme == "http") && raw.starts_with("t.me/")) {
        let url = url::Url::parse(input)?;
        let kind = if scheme == "tg" {
            url.host_str().unwrap_or("")
        } else {
            url.path()
                .trim_start_matches('/')
                .split('/')
                .next()
                .unwrap_or("")
        };
        if !["socks", "http", "https"].contains(&kind) {
            bail!("无效的 Telegram 代理链接");
        }
        let params = query(url.query().unwrap_or(""));
        let server = pick(&params, &["server"]);
        let p = port(&json!(pick(&params, &["port"])))?;
        if server.is_empty() {
            bail!("缺少服务器地址");
        }
        let name = pick(&params, &["remark", "remarks"]);
        let mut node = json!({"name":if name.is_empty() {format!("{}-{server}:{p}",kind.to_uppercase())} else {decode_percent(name,true)},"type":if kind=="socks" {"socks5"} else {kind},"server":server,"port":p});
        for (field, key) in [("username", "user"), ("password", "pass")] {
            let raw = pick(&params, &[key]);
            if !raw.is_empty() {
                node[field] = json!(decode_percent(raw, false));
            }
        }
        if kind == "https" {
            node["tls"] = json!(true);
        }
        if kind == "socks" {
            node["udp"] = json!(true);
        }
        return Ok(Some(node));
    }
    if scheme == "ss" {
        return ss(raw).map(Some);
    }
    if scheme == "ssr" {
        return ssr(raw).map(Some);
    }
    if scheme == "vmess" || scheme == "vmess1" {
        return vmess(raw, scheme == "vmess1").map(Some);
    }
    if scheme == "netch" {
        return netch(&serde_json::from_str::<Value>(&decode_base64(raw)?)?).map(Some);
    }
    let kind = match scheme.as_str() {
        "hy" => "hysteria",
        "hy2" => "hysteria2",
        "wg" => "wireguard",
        "socks" | "socks5+tls" => "socks5",
        other => other,
    };
    if ![
        "vless",
        "trojan",
        "anytls",
        "hysteria",
        "hysteria2",
        "tuic",
        "wireguard",
        "snell",
        "http",
        "https",
        "socks5",
        "socks4",
        "ssh",
    ]
    .contains(&kind)
    {
        bail!("不支持的协议: {scheme}");
    }
    let mut raw = raw.to_owned();
    let mut encoded = false;
    let mut suffix_name = None;
    if ["http", "https", "socks5", "socks4", "ssh"].contains(&kind) && raw.ends_with('}') {
        if let Some(index) = raw.rfind('{') {
            suffix_name = Some(decode_percent(&raw[index + 1..raw.len() - 1], true));
            raw.truncate(index);
        }
    }
    if ["http", "https", "socks5", "socks4", "ssh"].contains(&kind)
        && raw.ends_with(']')
        && !raw.starts_with('[')
    {
        if let Some(start) = raw.rfind('[') {
            if !raw[start + 1..raw.len() - 1].contains(':') {
                raw.truncate(start);
            }
        }
    }
    if ["http", "https", "socks5", "socks4", "ssh"].contains(&kind) {
        let end = raw.find(['?', '#', '/']).unwrap_or(raw.len());
        let authority = &raw[..end];
        if !authority.starts_with('[') && !authority.contains('@') {
            let parts: Vec<_> = authority.splitn(4, ':').collect();
            if parts.len() == 4 {
                raw = format!(
                    "{}:{}@{}:{}{}",
                    &parts[2],
                    &parts[3],
                    parts[0],
                    parts[1],
                    &raw[end..]
                );
            }
        }
    }
    let authority_end = raw.find(['?', '#']).unwrap_or(raw.len());
    if !raw[..authority_end].contains('@') && !raw[..authority_end].contains(':') {
        let token = raw[..authority_end].trim_end_matches('/');
        if let Ok(decoded) = decode_base64(token) {
            if decoded.contains('@')
                || decoded
                    .rsplit_once(':')
                    .is_some_and(|(_, p)| p.parse::<u16>().is_ok())
            {
                raw = format!("{decoded}{}", &raw[authority_end..]);
                encoded = true;
            }
        }
    }
    let uri = uri(&raw)?;
    let params = &uri.params;
    if uri.host.trim().is_empty() {
        bail!("缺少服务器地址");
    }
    let default_port = match kind {
        "http" => 80,
        "socks5" | "socks4" => 1080,
        "ssh" => 22,
        "wireguard" => 51820,
        _ => 443,
    };
    let mut ports = None;
    let p = if kind == "hysteria2" {
        let spec = pick(params, &["ports", "mport"]);
        let spec = if spec.is_empty() && !uri.port.bytes().all(|b| b.is_ascii_digit()) {
            &uri.port
        } else {
            spec
        };
        if !spec.is_empty() {
            ports = Some(port_spec(spec)?);
        }
        if !uri.port.is_empty() && uri.port.bytes().all(|b| b.is_ascii_digit()) {
            port(&json!(uri.port))?
        } else {
            ports.as_ref().map(|p| p.1).unwrap_or(default_port)
        }
    } else if uri.port.is_empty() {
        default_port
    } else {
        port(&json!(uri.port))?
    };
    let name = suffix_name.filter(|s| !s.is_empty()).unwrap_or_else(|| {
        if !uri.name.is_empty() {
            uri.name.clone()
        } else if !pick(params, &["remarks", "remark"]).is_empty() {
            decode_percent(pick(params, &["remarks", "remark"]), true)
        } else {
            match kind {
                "vless" => "VLESS 节点".into(),
                "trojan" => "Trojan 节点".into(),
                "hysteria2" => "Hysteria2 节点".into(),
                _ => format!("{}-{}:{p}", kind.to_uppercase(), uri.host),
            }
        }
    });
    let mut node = json!({"name":name,"type":kind,"server":uri.host,"port":p});
    common(&mut node, params);
    if let Some((ports, _)) = ports {
        node["ports"] = json!(ports);
    }
    match kind {
        "vless" => {
            let uuid = if encoded {
                uri.auth.rsplit(':').next().unwrap_or("")
            } else {
                uri.auth.split(':').next().unwrap_or("")
            };
            if uuid.is_empty() {
                bail!("VLESS 配置缺少必要字段");
            }
            node["uuid"] = json!(uuid);
            node["udp"] = json!(true);
            let pbk = pick(params, &["pbk", "public-key", "public_key", "publicKey"]);
            let security = pick(params, &["security"]);
            let security = if security.is_empty() {
                if !pbk.is_empty() {
                    "reality"
                } else if encoded && boolish(&json!(pick(params, &["tls"]))) == Some(true) {
                    "tls"
                } else {
                    "none"
                }
            } else {
                security
            };
            node["tls"] = json!(["tls", "reality"].contains(&security));
            if security == "reality" {
                let mut reality = json!({});
                if !pbk.is_empty() {
                    reality["public-key"] = json!(pbk);
                }
                if let Some(sid) = validation::short_id(&json!(pick(
                    params,
                    &["sid", "short-id", "short_id", "shortId"]
                ))) {
                    reality["short-id"] = json!(sid);
                }
                set_text(&mut reality, "_spider-x", params, &["spx"]);
                set_bool(
                    &mut reality,
                    "support-x25519mlkem768",
                    params,
                    &[
                        "support-x25519mlkem768",
                        "support_x25519mlkem768",
                        "supportX25519MLKEM768",
                    ],
                );
                if reality.get("support-x25519mlkem768").is_none()
                    && validation::reality_key(pbk)
                    && validation::valid_mlkem(pick(params, &["encryption", "flow-encryption"]))
                {
                    reality["support-x25519mlkem768"] = json!(true);
                }
                if reality.as_object().is_some_and(|m| !m.is_empty()) {
                    node["reality-opts"] = reality;
                }
                if node.get("client-fingerprint").is_none() {
                    node["client-fingerprint"] = json!("chrome");
                }
            }
            set_text(
                &mut node,
                "encryption",
                params,
                &["encryption", "flow-encryption"],
            );
            set_text(&mut node, "flow", params, &["flow"]);
            if encoded && node.get("flow").is_none() {
                match pick(params, &["xtls"]) {
                    "1" => node["flow"] = json!("xtls-rprx-direct"),
                    "2" => node["flow"] = json!("xtls-rprx-vision"),
                    _ => {}
                }
            }
            if params.contains_key("ech") {
                if security != "tls" {
                    bail!("VLESS 启用 ECH 需要 security=tls（不支持 Reality）");
                }
                node["ech-opts"] = ech(pick(params, &["ech"]));
            }
            transport(&mut node, params)?;
        }
        "trojan" | "anytls" => {
            let mut auth = password(&uri.auth, params);
            if encoded && kind == "anytls" {
                auth = auth.rsplit(':').next().unwrap_or("").to_owned();
            }
            if auth.is_empty() {
                bail!(
                    "{} 配置缺少必要字段",
                    if kind == "anytls" { "AnyTLS" } else { "Trojan" }
                );
            }
            node["password"] = json!(auth);
            if node.get("udp").is_none() {
                node["udp"] = json!(true);
            }
            if node.get("sni").is_none() {
                node["sni"] = json!(uri.host);
            }
            if params.contains_key("ech") {
                node["ech-opts"] = ech(pick(params, &["ech"]));
            }
            if kind == "anytls" {
                let security = pick(params, &["security"]);
                if !["", "none", "tls"].contains(&security) {
                    bail!("AnyTLS 不支持 security={security} / reality-opts（Mihomo 不支持）");
                }
                let network = pick(params, &["type", "network", "transport"]);
                if !["", "none", "tcp"].contains(&network) {
                    bail!("AnyTLS 不支持 type={network}（Mihomo 仅支持纯 TCP）");
                }
                for key in [
                    "pbk",
                    "sid",
                    "spx",
                    "public-key",
                    "public_key",
                    "publicKey",
                    "short-id",
                    "short_id",
                    "shortId",
                    "serviceName",
                    "service_name",
                    "service-name",
                    "authority",
                    "host",
                    "path",
                    "seed",
                    "mode",
                ] {
                    if params.contains_key(key) {
                        bail!("AnyTLS 不支持 Reality 或传输层参数: {key}（Mihomo 仅支持纯 TCP）");
                    }
                }
                let header = pick(params, &["headerType", "header_type", "header-type"]);
                if !["", "none"].contains(&header) {
                    bail!("AnyTLS 不支持 headerType={header}（仅支持纯 TCP）");
                }
                for (field, aliases) in [
                    (
                        "idle-session-check-interval",
                        &["idle-session-check-interval", "idleSessionCheckInterval"][..],
                    ),
                    (
                        "idle-session-timeout",
                        &["idle-session-timeout", "idleSessionTimeout"][..],
                    ),
                    (
                        "min-idle-session",
                        &["min-idle-session", "minIdleSession"][..],
                    ),
                ] {
                    set_integer(&mut node, field, params, aliases)?;
                }
                set_text(
                    &mut node,
                    "padding-scheme",
                    params,
                    &["padding-scheme", "paddingScheme", "padding_scheme"],
                );
            } else {
                transport(&mut node, params)?;
            }
        }
        "hysteria" | "hysteria2" => {
            if kind == "hysteria2" {
                let auth = password(&uri.auth, params);
                if auth.is_empty() {
                    bail!("Hysteria2 配置缺少必要字段");
                }
                node["password"] = json!(auth);
                if node.get("sni").is_none() {
                    node["sni"] = json!(uri.host);
                }
                let obfs = pick(params, &["obfs"]).to_ascii_lowercase();
                if !["", "none", "salamander", "gecko"].contains(&obfs.as_str()) {
                    bail!("不支持的 Hysteria2 obfs: {obfs}");
                }
                if ["salamander", "gecko"].contains(&obfs.as_str()) {
                    let secret = pick(params, &["obfs-password", "obfs_password", "obfsPassword"]);
                    if secret.is_empty() {
                        bail!("Hysteria2 {obfs} obfs 缺少 obfs-password");
                    }
                    node["obfs"] = json!(obfs);
                    node["obfs-password"] = json!(secret);
                }
                for key in ["obfs-min-packet-size", "obfs-max-packet-size"] {
                    set_integer(&mut node, key, params, &[key, &key.replace('-', "_")])?;
                }
                if params.contains_key("ech") {
                    node["ech-opts"] = ech(pick(params, &["ech"]));
                }
                set_text(
                    &mut node,
                    "fingerprint",
                    params,
                    &["fp", "fingerprint", "pinSHA256"],
                );
                set_text(
                    &mut node,
                    "mldsa65-seed",
                    params,
                    &["mldsa65-seed", "mldsa65_seed", "mldsa65Seed"],
                );
                let hop = pick(params, &["hop-interval", "hop_interval", "hopInterval"]);
                if !hop.is_empty() {
                    if let Ok(n) = hop.parse::<u64>() {
                        if n > 0 {
                            node["hop-interval"] = json!(n);
                        }
                    } else {
                        let parts: Vec<_> = hop.split('-').collect();
                        if parts.len() == 2
                            && parts[0].parse::<u64>().is_ok_and(|n| n > 0)
                            && parts[1]
                                .parse::<u64>()
                                .is_ok_and(|n| n >= parts[0].parse().unwrap_or(u64::MAX))
                        {
                            node["hop-interval"] = json!(hop);
                        }
                    }
                }
            } else {
                node["protocol"] = json!(if pick(params, &["protocol"]).is_empty() {
                    "udp"
                } else {
                    pick(params, &["protocol"])
                });
                set_text(
                    &mut node,
                    "auth-str",
                    params,
                    &["auth", "auth_str", "auth-str", "password"],
                );
                set_text(&mut node, "ports", params, &["mport", "ports"]);
                set_text(&mut node, "obfs", params, &["obfsParam", "obfs-param"]);
                set_text(&mut node, "_obfs", params, &["obfs"]);
            }
            for (field, keys) in [
                ("up", &["up", "upmbps"][..]),
                ("down", &["down", "downmbps"][..]),
            ] {
                let raw = pick(params, keys);
                if !raw.is_empty() {
                    node[field] = json!(if kind == "hysteria2"
                        && !raw.to_ascii_lowercase().ends_with("bps")
                    {
                        format!("{raw} mbps")
                    } else {
                        raw.to_owned()
                    });
                }
            }
            set_bool(
                &mut node,
                "fast-open",
                params,
                &["fast-open", "fast_open", "fastOpen"],
            );
        }
        "tuic" => {
            let (user, pass) = uri.auth.split_once(':').unwrap_or((&uri.auth, ""));
            let token = pick(params, &["token", "auth", "auth_str", "auth-str"]);
            let pass = if pass.is_empty() {
                pick(params, &["password", "passwd"])
            } else {
                pass
            };
            let uuid = if pass.is_empty() {
                pick(params, &["uuid", "user", "username"])
            } else {
                user
            };
            if !token.is_empty() || (!user.is_empty() && pass.is_empty()) {
                node["token"] = json!(if token.is_empty() { user } else { token });
            } else if !uuid.is_empty() && !pass.is_empty() {
                node["uuid"] = json!(uuid);
                node["password"] = json!(pass);
            } else {
                bail!("TUIC 配置缺少必要字段");
            }
            set_text(
                &mut node,
                "congestion-controller",
                params,
                &[
                    "congestion_control",
                    "congestion-control",
                    "congestionControl",
                    "congestion-controller",
                ],
            );
            set_text(
                &mut node,
                "udp-relay-mode",
                params,
                &["udp_relay_mode", "udp-relay-mode", "udpRelayMode"],
            );
            set_bool(
                &mut node,
                "disable-sni",
                params,
                &["disable_sni", "disable-sni", "disableSni"],
            );
            set_bool(
                &mut node,
                "reduce-rtt",
                params,
                &["reduce_rtt", "reduce-rtt", "reduceRtt"],
            );
            set_bool(
                &mut node,
                "fast-open",
                params,
                &["fast-open", "fast_open", "fastOpen", "tfo"],
            );
            for (field, camel) in [
                ("request-timeout", "requestTimeout"),
                ("heartbeat-interval", "heartbeatInterval"),
                ("max-open-streams", "maxOpenStreams"),
                ("max-idle-time", "maxIdleTime"),
            ] {
                set_integer(
                    &mut node,
                    field,
                    params,
                    &[field, &field.replace('-', "_"), camel],
                )?;
            }
        }
        "wireguard" => {
            let private = if uri.auth.is_empty() {
                pick(params, &["private-key", "private_key", "privateKey"])
            } else {
                &uri.auth
            };
            if private.is_empty() {
                bail!("WireGuard 配置缺少必要字段");
            }
            node["private-key"] = json!(private);
            node["udp"] = json!(true);
            for (key, value) in params {
                let key = key.replace('_', "-");
                if value.is_empty() {
                    continue;
                }
                match key.as_str() {
                    "address" | "ip" => {
                        for address in value.split(',') {
                            let address = address
                                .split('/')
                                .next()
                                .unwrap_or("")
                                .trim_matches(['[', ']']);
                            node[if address.contains(':') { "ipv6" } else { "ip" }] =
                                json!(address);
                        }
                    }
                    "mtu" => {
                        set_integer(&mut node, "mtu", params, &["mtu"])?;
                    }
                    "flag" => {}
                    _ => {
                        if node.get(&key).is_none() {
                            node[&key] = json!(value);
                        }
                    }
                }
            }
            set_bool(&mut node, "udp", params, &["udp"]);
        }
        "snell" => {
            let psk = password(&uri.auth, params);
            if psk.is_empty() {
                bail!("Snell 配置缺少必要字段");
            }
            node["psk"] = json!(psk);
            set_integer(&mut node, "version", params, &["version"])?;
            let obfs = pick(params, &["obfs"]);
            if ["http", "tls"].contains(&obfs) {
                let mut opts = json!({"mode":obfs});
                set_text(&mut opts, "host", params, &["obfs-host"]);
                set_text(&mut opts, "path", params, &["obfs-uri"]);
                node["obfs-opts"] = opts;
            }
            set_integer(
                &mut node,
                "shadow-tls-version",
                params,
                &["shadow-tls-version"],
            )?;
            set_text(&mut node, "shadow-tls-sni", params, &["shadow-tls-sni"]);
            set_text(
                &mut node,
                "shadow-tls-password",
                params,
                &["shadow-tls-password"],
            );
            set_bool(&mut node, "reuse", params, &["reuse"]);
            set_bool(&mut node, "tfo", params, &["fast-open", "tfo"]);
        }
        _ => {
            let (user, pass) = uri.auth.split_once(':').unwrap_or((&uri.auth, ""));
            if !user.is_empty() {
                node["username"] = json!(user);
            }
            if !pass.is_empty() {
                node["password"] = json!(pass);
            }
            if kind == "https" || scheme == "socks5+tls" {
                node["tls"] = json!(true);
            }
            if ["socks5", "socks4"].contains(&kind) && node.get("udp").is_none() {
                node["udp"] = json!(kind == "socks5");
            }
            if ["http", "https"].contains(&kind) {
                if let Some(headers) = headers(pick(params, &["headers", "header"])) {
                    node["headers"] = headers;
                }
            }
            if kind == "ssh" {
                set_text(
                    &mut node,
                    "private-key",
                    params,
                    &["private-key", "private_key", "privateKey"],
                );
                set_text(
                    &mut node,
                    "server-fingerprint",
                    params,
                    &["server-fingerprint", "server_fingerprint"],
                );
                set_integer(
                    &mut node,
                    "idle-timeout",
                    params,
                    &["idle-timeout", "idle_timeout"],
                )?;
                for (field, keys) in [
                    ("host-key", &["host-key", "host_key", "hostKey"][..]),
                    (
                        "host-key-algorithms",
                        &["host-key-algorithms", "host_key_algorithms"][..],
                    ),
                ] {
                    let raw = pick(params, keys);
                    if !raw.is_empty() {
                        node[field] = json!(
                            raw.split(['|', ',', ';'])
                                .map(str::trim)
                                .filter(|s| !s.is_empty())
                                .collect::<Vec<_>>()
                        );
                    }
                }
            }
        }
    }
    Ok(Some(node))
}

pub(super) fn common(node: &mut Value, params: &Params) {
    let kind = text(node, "type").to_owned();
    set_text(
        node,
        if ["vmess", "vless"].contains(&kind.as_str()) {
            "servername"
        } else {
            "sni"
        },
        params,
        &[
            "sni",
            "peer",
            "serverName",
            "servername",
            "server-name",
            "tls-name",
            "tls_name",
        ],
    );
    set_text(
        node,
        "client-fingerprint",
        params,
        &[
            "fp",
            "fingerprint",
            "client-fingerprint",
            "clientFingerprint",
        ],
    );
    set_text(
        node,
        "packet-encoding",
        params,
        &["packet-encoding", "packet_encoding", "packetEncoding"],
    );
    for key in ["pcs", "pqv"] {
        set_text(node, key, params, &[key]);
    }
    set_bool(
        node,
        "skip-cert-verify",
        params,
        &[
            "skip-cert-verify",
            "skip_cert_verify",
            "allowInsecure",
            "allow_insecure",
            "allow-insecure",
            "insecure",
        ],
    );
    if let Some(value) = params
        .get("tls-verification")
        .and_then(|s| boolish(&json!(s)))
    {
        node["skip-cert-verify"] = json!(!value);
    }
    if ["hysteria", "hysteria2", "tuic"].contains(&kind.as_str()) {
        set_bool(node, "tfo", params, &["tfo"]);
    } else {
        set_bool(
            node,
            "tfo",
            params,
            &["tfo", "fast-open", "fast_open", "fastOpen"],
        );
    }
    set_bool(node, "udp", params, &["udp", "udp-relay", "udp_relay"]);
    set_bool(
        node,
        "authenticated-length",
        params,
        &[
            "authenticated-length",
            "authenticated_length",
            "authenticatedLength",
        ],
    );
    set_bool(
        node,
        "global-padding",
        params,
        &["global-padding", "global_padding", "globalPadding"],
    );
    let alpn = pick(params, &["alpn"]);
    if !alpn.is_empty() {
        node["alpn"] = json!(list(alpn));
    }
}

pub(super) fn transport(node: &mut Value, params: &Params) -> Result<()> {
    let kind = text(node, "type").to_owned();
    let mut network = pick(params, &["type", "network", "transport", "net"]).to_ascii_lowercase();
    if network.is_empty() {
        network = if boolish(&json!(pick(params, &["ws"]))) == Some(true) {
            "ws"
        } else if boolish(&json!(pick(params, &["h2"]))) == Some(true) {
            "h2"
        } else {
            "tcp"
        }
        .into();
    }
    if ["websocket", "wss"].contains(&network.as_str()) {
        network = "ws".into();
    }
    if network == "none" {
        network = "tcp".into();
    }
    if network == "tcp" && pick(params, &["headerType", "header_type", "header-type"]) == "http" {
        network = "http".into();
    }
    let upgrade = network == "httpupgrade";
    if upgrade {
        network = "ws".into();
    }
    if !["tcp", "ws", "h2", "http", "grpc", "xhttp"].contains(&network.as_str())
        || (network == "xhttp" && kind != "vless")
    {
        bail!("不支持的 {} 传输层: {network}", kind.to_uppercase());
    }
    node["network"] = json!(network);
    let path = pick(
        params,
        &[
            "path",
            "wspath",
            "ws-path",
            "ws_path",
            "obfs-uri",
            "obfs-path",
        ],
    );
    let path = if path.is_empty() { "/" } else { path };
    let host = pick(
        params,
        &[
            "host",
            "ws-host",
            "wsHost",
            "ws.host",
            "obfs-host",
            "obfsParam",
        ],
    );
    match network.as_str() {
        "ws" => {
            let mut ws = json!({"path":path});
            let parsed = headers(pick(params, &["ws-headers", "headers"])).or_else(|| {
                if host.trim_start().starts_with('{') {
                    headers(host)
                } else {
                    None
                }
            });
            if let Some(headers) = parsed {
                ws["headers"] = headers;
            } else if !host.is_empty() {
                ws["headers"] = json!({"Host":host.split(',').next().unwrap_or(host)});
            }
            if upgrade {
                ws["v2ray-http-upgrade"] = json!(true);
                ws["v2ray-http-upgrade-fast-open"] = json!(true);
            }
            validation::normalize_ws(&mut ws, false);
            node["ws-opts"] = ws;
        }
        "grpc" => {
            let service = pick(
                params,
                &[
                    "serviceName",
                    "service_name",
                    "service-name",
                    "grpc-service-name",
                ],
            );
            let mut opts = json!({"grpc-service-name":if service.is_empty() { path.trim_start_matches('/') } else { service }});
            set_text(&mut opts, "_grpcType", params, &["mode"]);
            set_text(&mut opts, "_grpcAuthority", params, &["authority"]);
            node["grpc-opts"] = opts;
        }
        "http" => {
            let mut opts = json!({"method":if pick(params,&["method"]).is_empty() { "GET".into() } else { pick(params,&["method"]).to_ascii_uppercase() },"path":list(path)});
            if !host.is_empty() {
                opts["headers"] = json!({"Host":list(host)});
            }
            node["http-opts"] = opts;
        }
        "h2" => {
            let mut opts = json!({"path":path});
            if !host.is_empty() {
                opts["host"] = json!(list(host));
            }
            node["h2-opts"] = opts;
        }
        "xhttp" => {
            node["xhttp-opts"] = xhttp(params, path, host)?;
        }
        _ => {}
    }
    Ok(())
}

fn ss(raw: &str) -> Result<Value> {
    let (base, name) = raw.split_once('#').unwrap_or((raw, ""));
    let (base, qs) = base.split_once('?').unwrap_or((base, ""));
    let base = base.trim_end_matches('/');
    let params = query(qs);
    let decoded;
    let (auth, hostport) = if let Some((auth, host)) = base.rsplit_once('@') {
        (auth, host)
    } else {
        decoded = decode_base64(base)?;
        decoded
            .rsplit_once('@')
            .ok_or_else(|| anyhow!("SS 配置缺少必要字段"))?
    };
    let auth = decode_percent(auth, false);
    let auth = if auth.contains(':') {
        auth
    } else {
        decode_base64(&auth)?
    };
    let (cipher, password) = auth
        .split_once(':')
        .ok_or_else(|| anyhow!("SS 配置缺少加密方法或密码"))?;
    let (server, p) = endpoint(hostport)?;
    let p = port(&json!(p))?;
    if cipher.is_empty() || password.is_empty() || server.is_empty() {
        bail!("SS 配置缺少必要字段");
    }
    let name = decode_percent(name, true);
    let mut node = json!({"name":if name.is_empty() { "SS 节点" } else { &name },"type":"ss","server":server,"port":p,"cipher":cipher,"password":password,"udp":true});
    common(&mut node, &params);
    set_bool(&mut node, "udp-over-tcp", &params, &["uot", "udp-over-tcp"]);
    for (param, field) in [("uot", "udp-over-tcp"), ("tfo", "tfo")] {
        if params.get(param).is_some_and(String::is_empty) {
            node[field] = json!(true);
        }
    }
    let plugin = pick(&params, &["plugin"]);
    if !plugin.is_empty() {
        ss_plugin(&mut node, plugin)?;
    }
    if plugin.is_empty() {
        if let Some(raw) = params.get("v2ray-plugin") {
            let decoded = decode_base64(raw).unwrap_or_else(|_| raw.clone());
            if let Ok(opts) = serde_json::from_str::<Value>(&decoded) {
                if opts.is_object() {
                    node["plugin"] = json!("v2ray-plugin");
                    node["plugin-opts"] = opts;
                    if let Some(map) = node.as_object_mut() {
                        normalize_ss_plugin(map);
                    }
                }
            }
        }
    }
    Ok(node)
}
pub(super) fn ss_plugin(node: &mut Value, raw: &str) -> Result<()> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut escape = false;
    for c in raw.chars() {
        if escape {
            current.push(c);
            escape = false;
        } else if c == '\\' {
            escape = true;
        } else if c == ';' {
            parts.push(std::mem::take(&mut current));
        } else {
            current.push(c);
        }
    }
    if escape {
        current.push('\\');
    }
    parts.push(current);
    let plugin = parts.first().cloned().unwrap_or_default();
    if plugin.trim().is_empty() {
        return Ok(());
    }
    let mut opts = Map::new();
    for part in parts.into_iter().skip(1) {
        let (key, value) = part
            .split_once('=')
            .map(|(k, v)| (k.trim().to_owned(), json!(v)))
            .unwrap_or_else(|| (part.trim().to_owned(), json!(true)));
        if !key.is_empty() {
            opts.insert(key, value);
        }
    }
    node["plugin"] = json!(plugin);
    if !opts.is_empty() {
        node["plugin-opts"] = Value::Object(opts);
    }
    if let Some(map) = node.as_object_mut() {
        normalize_ss_plugin(map);
    }
    Ok(())
}
pub(super) fn normalize_ss_plugin(map: &mut Map<String, Value>) {
    let name = map
        .get("plugin")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if ["obfs", "obfs-local", "simple-obfs"].contains(&name.as_str()) {
        map.insert("plugin".into(), json!("obfs"));
        if let Some(opts) = map.get_mut("plugin-opts").and_then(Value::as_object_mut) {
            for (target, aliases) in [
                ("mode", &["obfs"][..]),
                ("host", &["obfs-host", "obfs_host"][..]),
            ] {
                if !opts.contains_key(target) {
                    if let Some(v) = aliases.iter().find_map(|key| opts.get(*key)).cloned() {
                        opts.insert(target.into(), v);
                    }
                }
                for alias in aliases {
                    opts.remove(*alias);
                }
            }
        }
    } else if ["v2ray-plugin", "xray-plugin"].contains(&name.as_str()) {
        map.insert("plugin".into(), json!(name));
        if let Some(opts) = map.get_mut("plugin-opts").and_then(Value::as_object_mut) {
            for key in ["tls", "mux", "skip-cert-verify"] {
                if let Some(v) = opts.get(key).and_then(boolish) {
                    opts.insert(key.into(), json!(v));
                }
            }
            opts.entry("mode").or_insert(json!("websocket"));
        }
    }
    if let Some(opts) = map.get_mut("plugin-opts").and_then(Value::as_object_mut) {
        for key in ["tls", "mux"] {
            if let Some(v) = opts.get(key).and_then(boolish) {
                opts.insert(key.into(), json!(v));
            }
        }
    }
}
fn ssr(raw: &str) -> Result<Value> {
    let decoded = decode_base64(raw)?;
    let (base, qs) = decoded.split_once("/?").unwrap_or((&decoded, ""));
    let fields: Vec<_> = base.rsplitn(6, ':').collect();
    if fields.len() != 6 {
        bail!("无效的 SSR 链接");
    }
    let server = fields[5].trim_matches(['[', ']']);
    let p = port(&json!(fields[4]))?;
    let mut params = Params::new();
    for item in qs.split('&') {
        if let Some((k, v)) = item.split_once('=') {
            params.insert(k.to_owned(), v.to_owned());
        }
    }
    let decode = |key: &str| {
        let raw = pick(&params, &[key]);
        decode_base64(raw).unwrap_or_else(|_| decode_percent(raw, true))
    };
    let name = decode("remarks");
    let mut node = json!({"name":if name.is_empty() { "SSR 节点" } else { &name },"type":"ssr","server":server,"port":p,"protocol":fields[3],"cipher":fields[2],"obfs":fields[1],"password":decode_base64(fields[0])?,"udp":true});
    for (field, key) in [
        ("protocol-param", "protoparam"),
        ("obfs-param", "obfsparam"),
    ] {
        let value = decode(key);
        if !value.is_empty() {
            node[field] = json!(value);
        }
    }
    Ok(node)
}

fn vmess(raw: &str, kitsunebi: bool) -> Result<Value> {
    let (payload, hash) = raw.split_once('#').unwrap_or((raw, ""));
    let (base, qs) = payload.split_once('?').unwrap_or((payload, ""));
    let mut params = query(qs);
    let mut name = decode_percent(hash, true);
    if let Ok(decoded) = decode_base64(base.trim_end_matches('/')) {
        if let Ok(config) = serde_json::from_str::<Value>(&decoded) {
            return vmess_json(&config, &name);
        }
        if decoded.contains("= vmess") || decoded.contains("=vmess") {
            return super::platform::parse_line(&decoded, &Default::default())?
                .ok_or_else(|| anyhow!("无效的 Quantumult VMess 配置"));
        }
        let parsed = uri(&decoded)?;
        let (cipher, uuid) = parsed
            .auth
            .split_once(':')
            .ok_or_else(|| anyhow!("无效的 Shadowrocket VMess 基础信息"))?;
        params.insert("encryption".into(), cipher.into());
        let obfs = pick(&params, &["obfs"]).to_owned();
        if ["websocket", "ws", "wss"].contains(&obfs.as_str()) {
            params.insert("type".into(), "ws".into());
            if obfs == "wss" {
                params.insert("security".into(), "tls".into());
            }
        }
        if name.is_empty() {
            name = decode_percent(pick(&params, &["remarks", "remark"]), true);
        }
        return vmess_uri(&parsed.host, &parsed.port, uuid, &name, &params);
    }
    // Standard VMess: transport[+tls]:uuid-alterId@host:port.
    if let Some((tag, rest)) = base.split_once(':') {
        let network = tag.split('+').next().unwrap_or("");
        if network.bytes().all(|c| c.is_ascii_alphabetic()) && rest.contains('@') {
            let parsed = uri(rest)?;
            let (uuid, aid) = parsed
                .auth
                .rsplit_once('-')
                .ok_or_else(|| anyhow!("无效的标准 VMess 链接"))?;
            params.insert("type".into(), network.into());
            params.insert("aid".into(), aid.into());
            if tag.contains('+') {
                params.insert("security".into(), "tls".into());
            }
            return vmess_uri(&parsed.host, &parsed.port, uuid, &name, &params);
        }
    }
    let parsed = uri(payload)?;
    if kitsunebi && parsed.path != "/" {
        params.insert("path".into(), parsed.path.clone());
    }
    let uuid = if parsed.auth.is_empty() {
        pick(&params, &["id"])
    } else {
        &parsed.auth
    };
    vmess_uri(&parsed.host, &parsed.port, uuid, &name, &params)
}
fn vmess_uri(server: &str, p: &str, uuid: &str, name: &str, params: &Params) -> Result<Value> {
    if server.is_empty() || uuid.is_empty() {
        bail!("VMess 配置缺少必要字段");
    }
    let tls = ["tls", "reality"].contains(&pick(params, &["security"]))
        || boolish(&json!(pick(params, &["tls"]))) == Some(true);
    let p = if p.is_empty() {
        if tls { 443 } else { 80 }
    } else {
        port(&json!(p))?
    };
    let cipher = pick(params, &["encryption", "scy", "cipher"]);
    let aid = pick(params, &["aid", "alterId", "alter_id", "alter-id"]);
    let mut node = json!({"name":if name.is_empty() { "VMess 节点" } else { name },"type":"vmess","server":server,"port":p,"uuid":uuid,"alterId":if aid.is_empty() {0} else {integer(&json!(aid)).ok_or_else(|| anyhow!("VMess alterId 无效"))?},"cipher":if cipher.is_empty() {"auto"} else {cipher},"udp":true,"tls":tls});
    common(&mut node, params);
    transport(&mut node, params)?;
    if node.get("servername").is_none() && !pick(params, &["host"]).is_empty() {
        node["servername"] = json!(pick(params, &["host"]).split(',').next().unwrap_or(""));
    }
    if params.contains_key("ech") {
        if !tls {
            bail!("VMess 启用 ECH 需要 TLS（security=tls）");
        }
        node["ech-opts"] = ech(pick(params, &["ech"]));
    }
    Ok(node)
}
fn vmess_json(config: &Value, remark: &str) -> Result<Value> {
    if !config.is_object() {
        bail!("无效的 VMess JSON 格式");
    }
    let mut params = Params::new();
    for (source, target) in [
        ("net", "type"),
        ("scy", "encryption"),
        ("encryption", "encryption"),
        ("aid", "aid"),
        ("host", "host"),
        ("path", "path"),
        ("sni", "sni"),
        ("fp", "fp"),
        ("alpn", "alpn"),
        ("allowInsecure", "allowInsecure"),
        ("mode", "mode"),
        ("authority", "authority"),
        ("serviceName", "serviceName"),
        ("packet-encoding", "packet-encoding"),
        ("packetEncoding", "packet-encoding"),
        ("authenticatedLength", "authenticated-length"),
        ("globalPadding", "global-padding"),
        ("ech", "ech"),
    ] {
        if let Some(v) = config.get(source) {
            if let Some(s) = v.as_str() {
                params.insert(target.into(), s.into());
            } else if v.is_number() || v.is_boolean() {
                params.insert(target.into(), v.to_string());
            } else {
                bail!("VMess {source} 字段无效");
            }
        }
    }
    if text(config, "tls") == "tls" || config.get("tls").and_then(boolish) == Some(true) {
        params.insert("security".into(), "tls".into());
    }
    if text(config, "type") == "http" && ["", "tcp"].contains(&pick(&params, &["type"])) {
        params.insert("headerType".into(), "http".into());
    }
    let port_value = config
        .get("port")
        .ok_or_else(|| anyhow!("VMess 配置缺少 port"))?;
    let p = port(port_value)?.to_string();
    let mut node = vmess_uri(
        text(config, "add"),
        &p,
        text(config, "id"),
        if remark.is_empty() {
            text(config, "ps")
        } else {
            remark
        },
        &params,
    )?;
    if let Some(headers) = config.get("headers") {
        let target = if text(&node, "network") == "http" {
            "http-opts"
        } else {
            "ws-opts"
        };
        if let Some(opts) = node.get_mut(target) {
            opts["headers"] = headers.clone();
        }
    }
    Ok(node)
}

pub(super) fn netch(config: &Value) -> Result<Value> {
    let kind = text(config, "Type").to_ascii_lowercase();
    if ![
        "ss", "ssr", "vmess", "trojan", "snell", "socks", "socks5", "http", "https",
    ]
    .contains(&kind.as_str())
    {
        bail!("不支持的 Netch 类型: {}", text(config, "Type"));
    }
    let kind = if kind == "socks" { "socks5" } else { &kind };
    let mut node = json!({"name":if text(config,"Remark").is_empty() { format!("{kind}-{}",text(config,"Hostname")) } else { text(config,"Remark").to_owned() },"type":kind,"server":text(config,"Hostname"),"port":port(&config["Port"])?});
    for (source, target) in [
        ("Password", "password"),
        ("Username", "username"),
        ("EncryptMethod", "cipher"),
        ("Protocol", "protocol"),
        ("ProtocolParam", "protocol-param"),
        ("OBFS", "obfs"),
        ("OBFSParam", "obfs-param"),
        ("UserID", "uuid"),
        ("AlterID", "alterId"),
        ("SnellVersion", "version"),
    ] {
        if let Some(value) = config.get(source) {
            node[target] = value.clone();
        }
    }
    for (source, target) in [
        ("EnableUDP", "udp"),
        ("EnableTFO", "tfo"),
        ("AllowInsecure", "skip-cert-verify"),
    ] {
        if let Some(v) = config.get(source).and_then(boolish) {
            node[target] = json!(v);
        }
    }
    if kind == "ss" && !text(config, "Plugin").is_empty() {
        ss_plugin(
            &mut node,
            &format!(
                "{};{}",
                text(config, "Plugin"),
                text(config, "PluginOption")
            ),
        )?;
    }
    if ["vmess", "trojan"].contains(&kind) {
        let mut params = Params::new();
        for (source, target) in [
            ("TransferProtocol", "type"),
            ("Host", "host"),
            ("Path", "path"),
            ("Edge", "edge"),
            ("ServerName", "sni"),
        ] {
            let v = text(config, source);
            if !v.is_empty() {
                params.insert(target.into(), v.into());
            }
        }
        if text(config, "FakeType") == "http" && ["", "tcp"].contains(&pick(&params, &["type"])) {
            params.insert("headerType".into(), "http".into());
        }
        common(&mut node, &params);
        transport(&mut node, &params)?;
        let tls = config
            .get("TLSSecure")
            .and_then(boolish)
            .unwrap_or(kind == "trojan");
        if kind == "trojan" && !tls {
            bail!("Netch Trojan 必须启用 TLS");
        }
        if kind == "vmess" {
            node["tls"] = json!(tls);
        }
    }
    if kind == "snell" {
        node["psk"] = node["password"].clone();
        if let Some(map) = node.as_object_mut() {
            map.remove("password");
        }
        let obfs = text(config, "OBFS");
        if ["http", "tls"].contains(&obfs) {
            node["obfs-opts"] = json!({"mode":obfs,"host":text(config,"Host")});
        }
    }
    if kind == "https" {
        node["tls"] = json!(true);
    }
    Ok(node)
}

pub(super) fn ech_hostname(raw: &str) -> bool {
    let raw = raw.trim_end_matches('.');
    raw.len() <= 253
        && raw.contains('.')
        && raw.bytes().any(|b| b.is_ascii_alphabetic())
        && raw.split('.').all(|s| {
            !s.is_empty()
                && s.len() <= 63
                && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                && !s.starts_with('-')
                && !s.ends_with('-')
        })
}
fn ech(raw: &str) -> Value {
    let raw = raw.trim();
    if !raw.is_empty() && raw.len() % 4 == 0 && STANDARD.decode(raw).is_ok() {
        json!({"enable":true,"config":raw})
    } else if ech_hostname(raw) {
        json!({"enable":true,"query-server-name":raw})
    } else if let Some((host, resolver)) = raw.split_once('+') {
        if ech_hostname(host) && resolver.contains("://") && url::Url::parse(resolver).is_ok() {
            json!({"enable":true,"query-server-name":host})
        } else {
            json!({"enable":true})
        }
    } else {
        json!({"enable":true})
    }
}

const XHTTP_FIELDS: &[(&str, &str)] = &[
    ("noGRPCHeader", "no-grpc-header"),
    ("xPaddingBytes", "x-padding-bytes"),
    ("xPaddingObfsMode", "x-padding-obfs-mode"),
    ("xPaddingKey", "x-padding-key"),
    ("xPaddingHeader", "x-padding-header"),
    ("xPaddingPlacement", "x-padding-placement"),
    ("xPaddingMethod", "x-padding-method"),
    ("uplinkHTTPMethod", "uplink-http-method"),
    ("sessionIDPlacement", "session-placement"),
    ("sessionIDKey", "session-key"),
    ("sessionIDTable", "session-table"),
    ("sessionIDLength", "session-length"),
    ("seqPlacement", "seq-placement"),
    ("seqKey", "seq-key"),
    ("uplinkDataPlacement", "uplink-data-placement"),
    ("uplinkDataKey", "uplink-data-key"),
    ("uplinkChunkSize", "uplink-chunk-size"),
    ("scMaxEachPostBytes", "sc-max-each-post-bytes"),
    ("scMinPostsIntervalMs", "sc-min-posts-interval-ms"),
];
const REUSE_FIELDS: &[(&str, &str)] = &[
    ("maxConcurrency", "max-concurrency"),
    ("maxConnections", "max-connections"),
    ("cMaxReuseTimes", "c-max-reuse-times"),
    ("hMaxRequestTimes", "h-max-request-times"),
    ("hMaxReusableSecs", "h-max-reusable-secs"),
    ("hKeepAlivePeriod", "h-keep-alive-period"),
];
fn range(value: &Value, field: &str, source: bool) -> Result<String> {
    let raw = if let Some(s) = value.as_str() {
        s.to_owned()
    } else if value.is_number() {
        value.to_string()
    } else {
        bail!("XHTTP {field} 需要非负整数或范围");
    };
    if raw.trim().is_empty() {
        return Ok(String::new());
    }
    let parts: Vec<_> = raw
        .split('-')
        .map(|s| s.trim().trim_start_matches('+').parse::<u64>())
        .collect();
    if parts.is_empty() || parts.len() > 2 || parts.iter().any(|value| value.is_err()) {
        bail!("XHTTP {field} 需要非负整数或范围");
    }
    let mut min = *parts[0].as_ref().unwrap();
    let mut max = *parts.last().unwrap().as_ref().unwrap();
    if source && min > max {
        std::mem::swap(&mut min, &mut max);
    }
    if max < min || max > 2_147_483_647 {
        bail!("XHTTP {field} 范围无效");
    }
    Ok(if min == max {
        min.to_string()
    } else {
        format!("{min}-{max}")
    })
}
pub(super) fn normalize_xhttp(opts: &mut Value) -> Result<()> {
    let map = opts
        .as_object_mut()
        .ok_or_else(|| anyhow!("XHTTP 需要对象"))?;
    if let Some(mode) = map.get("mode") {
        if !mode
            .as_str()
            .is_some_and(|s| ["", "auto", "packet-up", "stream-up", "stream-one"].contains(&s))
        {
            bail!("XHTTP mode 无效");
        }
    }
    for field in [
        "x-padding-bytes",
        "session-length",
        "uplink-chunk-size",
        "sc-max-each-post-bytes",
        "sc-min-posts-interval-ms",
    ] {
        if let Some(value) = map.get(field) {
            let normalized = range(value, field, false)?;
            if field.starts_with("sc-") && normalized == "0" {
                bail!("XHTTP {field} 必须大于零");
            }
            map.insert(
                field.into(),
                if field == "sc-max-each-post-bytes"
                    && !normalized.is_empty()
                    && !normalized.contains('-')
                {
                    json!(normalized.parse::<u64>()?)
                } else {
                    json!(normalized)
                },
            );
        }
    }
    for (field, options) in [
        (
            "x-padding-placement",
            &["queryInHeader", "cookie", "header", "query"][..],
        ),
        ("x-padding-method", &["repeat-x", "tokenish"][..]),
        (
            "session-placement",
            &["path", "query", "cookie", "header"][..],
        ),
        ("seq-placement", &["path", "query", "cookie", "header"][..]),
        (
            "uplink-data-placement",
            &["auto", "body", "cookie", "header"][..],
        ),
    ] {
        if let Some(value) = map.get(field) {
            if !value
                .as_str()
                .is_some_and(|s| s.is_empty() || options.contains(&s))
            {
                bail!("XHTTP {field} 无效");
            }
        }
    }
    for field in [
        "path",
        "host",
        "x-padding-key",
        "x-padding-header",
        "uplink-http-method",
        "session-key",
        "session-table",
        "seq-key",
        "uplink-data-key",
    ] {
        if map.get(field).is_some_and(|v| !v.is_string()) {
            bail!("XHTTP {field} 需要字符串");
        }
    }
    for field in ["no-grpc-header", "x-padding-obfs-mode"] {
        if let Some(value) = map.get(field) {
            let value = boolish(value).ok_or_else(|| anyhow!("XHTTP {field} 需要布尔值"))?;
            map.insert(field.into(), json!(value));
        }
    }
    if map.get("headers").is_some_and(|v| {
        !v.as_object()
            .is_some_and(|m| m.values().all(Value::is_string))
    }) {
        bail!("XHTTP headers 需要字符串映射");
    }
    if let Some(reuse) = map.get_mut("reuse-settings") {
        let reuse = reuse
            .as_object_mut()
            .ok_or_else(|| anyhow!("XHTTP reuse-settings 需要对象"))?;
        for (_, field) in REUSE_FIELDS {
            if let Some(value) = reuse.get(*field) {
                if *field == "h-keep-alive-period" {
                    let n = value
                        .as_i64()
                        .or_else(|| value.as_str().and_then(|s| s.parse().ok()))
                        .ok_or_else(|| anyhow!("XHTTP h-keep-alive-period 需要整数"))?;
                    reuse.insert((*field).into(), json!(n));
                } else {
                    let r = range(value, field, false)?;
                    reuse.insert((*field).into(), json!(r));
                }
            }
        }
    }
    if map.get("mode").and_then(Value::as_str) == Some("stream-one")
        && map.contains_key("download-settings")
    {
        bail!("XHTTP stream-one 不支持 downloadSettings");
    }
    if let Some(download) = map.get_mut("download-settings") {
        let download = download
            .as_object_mut()
            .ok_or_else(|| anyhow!("XHTTP download-settings 需要对象"))?;
        if let Some(ro) = download.get_mut("reality-opts") {
            let key = text(ro, "public-key");
            if !key.is_empty() && !validation::reality_key(key) {
                bail!("XHTTP 下行 Reality public-key 无效");
            }
        }
    }
    Ok(())
}
fn source_extra(source: &Value) -> Result<Value> {
    let source = source
        .as_object()
        .ok_or_else(|| anyhow!("XHTTP extra 需要 JSON 对象"))?;
    let mut opts = json!({});
    for (from, to) in XHTTP_FIELDS {
        if let Some(v) = source.get(*from) {
            opts[*to] = v.clone();
        }
    }
    if let Some(headers) = source.get("headers") {
        if !headers.as_object().is_some_and(|m| {
            m.values().all(Value::is_string) && m.keys().all(|k| !k.eq_ignore_ascii_case("host"))
        }) {
            bail!("XHTTP headers 需要字符串映射；Host 请使用 host 字段");
        }
        opts["headers"] = headers.clone();
    }
    if let Some(xmux) = source.get("xmux") {
        let xmux = xmux
            .as_object()
            .ok_or_else(|| anyhow!("XHTTP xmux 需要对象"))?;
        let mut reuse = json!({});
        for (from, to) in REUSE_FIELDS {
            if let Some(v) = xmux.get(*from) {
                reuse[*to] = if *to == "h-keep-alive-period" {
                    v.clone()
                } else {
                    json!(range(v, to, true)?)
                };
            }
        }
        opts["reuse-settings"] = reuse;
    }
    for field in [
        "x-padding-bytes",
        "session-length",
        "uplink-chunk-size",
        "sc-max-each-post-bytes",
        "sc-min-posts-interval-ms",
    ] {
        if let Some(value) = opts.get(field) {
            let r = range(value, field, true)?;
            opts[field] = json!(r);
        }
    }
    for (field, default) in [
        ("x-padding-bytes", "100-1000"),
        ("sc-max-each-post-bytes", "1000000"),
        ("sc-min-posts-interval-ms", "30"),
    ] {
        if opts.get(field) == Some(&json!("0")) {
            opts[field] = json!(default);
        }
    }
    if text(&opts, "session-table") == "uuid" {
        bail!("XHTTP sessionIDTable 字符表 uuid 无法由 Mihomo 同名保留字表达");
    }
    if text(&opts, "x-padding-bytes").split('-').any(|s| s == "0") {
        bail!("XHTTP xPaddingBytes 不能包含零端点");
    }
    Ok(opts)
}
fn source_defaults(opts: &mut Value) -> Result<()> {
    normalize_xhttp(opts)?;
    if opts.get("reuse-settings").is_none() {
        opts["reuse-settings"] = json!({});
    }
    let reuse = opts["reuse-settings"]
        .as_object_mut()
        .ok_or_else(|| anyhow!("XHTTP reuse-settings 需要对象"))?;
    let max = |key: &str| {
        reuse
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or("0")
            .split('-')
            .next_back()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0)
    };
    if max("max-concurrency") > 0 && max("max-connections") > 0 {
        bail!("XHTTP xmux maxConcurrency 与 maxConnections 不能同时大于零");
    }
    if REUSE_FIELDS
        .iter()
        .filter(|(_, k)| *k != "h-keep-alive-period")
        .all(|(_, k)| max(k) == 0)
        && reuse
            .get("h-keep-alive-period")
            .and_then(Value::as_i64)
            .unwrap_or(0)
            == 0
    {
        reuse.insert("max-connections".into(), json!("3"));
        reuse.insert("h-max-request-times".into(), json!("600-900"));
        reuse.insert("h-max-reusable-secs".into(), json!("1800-3000"));
    }
    if text(opts, "session-table").is_empty() {
        opts["session-table"] = json!("uuid");
    }
    if text(opts, "session-table") != "uuid"
        && text(opts, "session-length")
            .split('-')
            .next()
            .unwrap_or("0")
            .parse::<u64>()
            .unwrap_or(0)
            == 0
    {
        bail!("XHTTP sessionIDTable 需要正数 sessionIDLength");
    }
    if opts.get("uplink-http-method").is_some() {
        opts["uplink-http-method"] = json!(text(opts, "uplink-http-method").to_ascii_uppercase());
    }
    let placement = text(opts, "uplink-data-placement").to_owned();
    if text(opts, "uplink-data-key").is_empty() && placement != "body" {
        opts["uplink-data-key"] = json!(if placement == "cookie" {
            "x_data"
        } else {
            "X-Data"
        });
    }
    if placement == "header" && ["", "0"].contains(&text(opts, "uplink-chunk-size")) {
        opts["uplink-chunk-size"] = json!("3000-4000");
    }
    Ok(())
}
fn xhttp(params: &Params, path: &str, host: &str) -> Result<Value> {
    let extra = if let Some(raw) = params.get("extra") {
        serde_json::from_str::<Value>(raw).map_err(|_| anyhow!("XHTTP extra 需要有效 JSON 对象"))?
    } else {
        json!({})
    };
    let source = params.contains_key("extra") || params.contains_key("downloadSettings");
    let mut opts = if source {
        source_extra(&extra)?
    } else {
        json!({})
    };
    opts["path"] = json!(
        if pick(params, &["xhttp-path", "xhttp_path", "xhttpPath"]).is_empty() {
            path
        } else {
            pick(params, &["xhttp-path", "xhttp_path", "xhttpPath"])
        }
    );
    let host = if pick(params, &["xhttp-host", "xhttp_host", "xhttpHost"]).is_empty() {
        host
    } else {
        pick(params, &["xhttp-host", "xhttp_host", "xhttpHost"])
    };
    if !host.is_empty() {
        opts["host"] = json!(host);
    }
    set_text(
        &mut opts,
        "mode",
        params,
        &["xhttp-mode", "xhttp_mode", "xhttpMode", "mode"],
    );
    if let Some(headers) = headers(pick(
        params,
        &["xhttp-headers", "xhttp_headers", "xhttpHeaders", "headers"],
    )) {
        let mut merged = opts
            .get("headers")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        merged.extend(headers.as_object().cloned().unwrap_or_default());
        opts["headers"] = Value::Object(merged);
    }
    for (camel, key) in XHTTP_FIELDS {
        let underscore = key.replace('-', "_");
        let raw = pick(params, &[key, &underscore, camel]);
        if !raw.is_empty() {
            opts[*key] = json!(raw);
        }
    }
    for (camel, key) in REUSE_FIELDS {
        let underscore = key.replace('-', "_");
        let raw = pick(params, &[key, &underscore, camel]);
        if !raw.is_empty() {
            if opts.get("reuse-settings").is_none() {
                opts["reuse-settings"] = json!({});
            }
            opts["reuse-settings"][*key] = json!(raw);
        }
    }
    set_bool(
        &mut opts,
        "no-grpc-header",
        params,
        &[
            "no-grpc-header",
            "no_grpc_header",
            "noGrpcHeader",
            "noGRPCHeader",
        ],
    );
    if source {
        source_defaults(&mut opts)?;
    }
    if params.contains_key("extra") {
        if text(&opts, "path").contains('?') {
            bail!("XHTTP path 查询参数无法由 Mihomo 表达");
        }
        if text(&opts, "mode") != "packet-up"
            && (["header", "cookie"].contains(&text(&opts, "uplink-data-placement"))
                || text(&opts, "uplink-http-method") == "GET")
        {
            bail!("XHTTP header/cookie 上传及 GET 需要显式 packet-up 模式");
        }
    }
    let download = if let Some(raw) = params.get("downloadSettings") {
        Some(
            serde_json::from_str::<Value>(raw)
                .map_err(|_| anyhow!("XHTTP downloadSettings 需要有效 JSON 对象"))?,
        )
    } else {
        extra.get("downloadSettings").cloned()
    };
    if let Some(download) = download {
        opts["download-settings"] = xhttp_download(&download, &opts)?;
    }
    let mut legacy = json!({});
    set_text(
        &mut legacy,
        "path",
        params,
        &["download-path", "download_path", "downloadPath"],
    );
    set_text(
        &mut legacy,
        "host",
        params,
        &["download-host", "download_host", "downloadHost"],
    );
    if let Some(h) = headers(pick(
        params,
        &["download-headers", "download_headers", "downloadHeaders"],
    )) {
        legacy["headers"] = h;
    }
    if legacy.as_object().is_some_and(|m| !m.is_empty()) {
        let mut map = opts
            .get("download-settings")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        map.extend(legacy.as_object().cloned().unwrap_or_default());
        opts["download-settings"] = Value::Object(map);
    }
    normalize_xhttp(&mut opts)?;
    Ok(opts)
}
fn xhttp_download(source: &Value, main: &Value) -> Result<Value> {
    let source = source
        .as_object()
        .ok_or_else(|| anyhow!("XHTTP downloadSettings 需要对象"))?;
    let get = |key: &str| source.get(key).and_then(Value::as_str).unwrap_or("");
    if !["", "xhttp", "splithttp"].contains(&get("network")) {
        bail!("XHTTP 下行仅支持 xhttp/splithttp 网络");
    }
    let server = get("address");
    let p = port(source.get("port").unwrap_or(&Value::Null))?;
    if server.is_empty() {
        bail!("XHTTP downloadSettings 需要有效 address 和 port");
    }
    let security = get("security");
    if !["", "none", "tls", "reality"].contains(&security) {
        bail!("XHTTP 下行 security 不受支持");
    }
    for field in ["sockopt", "finalmask"] {
        if source.contains_key(field) {
            bail!("XHTTP 下行 {field} 无法转换为 Mihomo 设置");
        }
    }
    let empty = json!({});
    let transport = source
        .get("xhttpSettings")
        .or_else(|| source.get("splithttpSettings"))
        .unwrap_or(&empty);
    if !transport.is_object() {
        bail!("XHTTP 下行 xhttpSettings 需要对象");
    }
    let effective = transport.get("extra").unwrap_or(transport);
    if effective.get("downloadSettings").is_some() {
        bail!("XHTTP 不支持嵌套 downloadSettings");
    }
    let mut mapped = source_extra(effective)?;
    source_defaults(&mut mapped)?;
    for (field, default) in [
        ("x-padding-bytes", json!("100-1000")),
        ("x-padding-obfs-mode", json!(false)),
        ("session-placement", json!("path")),
        ("seq-placement", json!("path")),
    ] {
        if mapped.get(field).unwrap_or(&default) != main.get(field).unwrap_or(&default) {
            bail!("XHTTP 下行独立 {field} 无法由 Mihomo 表达");
        }
    }
    for (field, default) in [
        ("x-padding-key", "x_padding"),
        ("x-padding-header", "X-Padding"),
        ("x-padding-placement", "queryInHeader"),
        ("x-padding-method", "repeat-x"),
        ("session-key", ""),
    ] {
        let effective = |opts: &Value| -> String {
            if field.starts_with("x-padding-")
                && opts.get("x-padding-obfs-mode") != Some(&json!(true))
            {
                default.into()
            } else if text(opts, field).is_empty() {
                default.into()
            } else {
                text(opts, field).to_owned()
            }
        };
        if effective(&mapped) != effective(main) {
            bail!("XHTTP 下行独立 {field} 无法由 Mihomo 表达");
        }
    }
    let tls = source
        .get(if security == "reality" {
            "realitySettings"
        } else {
            "tlsSettings"
        })
        .unwrap_or(&empty);
    if !tls.is_object() {
        bail!("XHTTP 下行安全设置需要对象");
    }
    for field in [
        "echConfigList",
        "echForceQuery",
        "echServerKeys",
        "echSockopt",
        "mldsa65Verify",
        "pinnedPeerCertSha256",
        "verifyPeerCertByName",
        "pinnedPeerCertificateChainSha256",
        "certificates",
        "verifyPeerCertInNames",
        "disableSystemRoot",
        "useSystemCA",
        "minVersion",
        "maxVersion",
        "cipherSuites",
        "curvePreferences",
        "enableSessionResumption",
    ] {
        if tls.get(field).is_some_and(|v| {
            !v.is_null() && v != false && v != "" && v.as_array().is_none_or(|a| !a.is_empty())
        }) {
            bail!("XHTTP 下行 {field} 无法转换为 Mihomo 设置");
        }
    }
    let path = if text(transport, "path").is_empty() {
        "/"
    } else {
        text(transport, "path")
    };
    if path.contains('?') {
        bail!("XHTTP 下行 path 查询参数无法由 Mihomo 表达");
    }
    if tls
        .get("alpn")
        .is_some_and(|v| !v.as_array().is_some_and(|a| a.iter().all(Value::is_string)))
    {
        bail!("XHTTP 下行 alpn 需要字符串数组");
    }
    if tls.get("allowInsecure").is_some_and(|v| !v.is_boolean()) {
        bail!("XHTTP 下行 allowInsecure 需要布尔值");
    }
    let mut out = json!({"server":server,"port":p,"tls":(["tls","reality"].contains(&security)),"servername":text(tls,"serverName"),"client-fingerprint":if text(tls,"fingerprint").is_empty() && security=="reality" { "chrome" } else { text(tls,"fingerprint") },"skip-cert-verify":tls.get("allowInsecure")==Some(&json!(true)),"alpn":tls.get("alpn").cloned().unwrap_or(json!([])),"ech-opts":{"enable":false},"path":path,"host":text(transport,"host"),"headers":mapped.get("headers").cloned().unwrap_or(json!({})),"reuse-settings":mapped["reuse-settings"],"reality-opts":{"public-key":""}});
    if security == "reality" {
        let key = if text(tls, "password").is_empty() {
            text(tls, "publicKey")
        } else {
            text(tls, "password")
        };
        if !validation::reality_key(key) {
            bail!("XHTTP 下行 Reality 缺少有效 publicKey/password");
        }
        out["reality-opts"]["public-key"] = json!(key);
        if let Some(sid) = validation::short_id(&tls["shortId"]) {
            out["reality-opts"]["short-id"] = json!(sid);
        }
        if let Some(v) = tls.get("support-x25519mlkem768") {
            if !v.is_boolean() {
                bail!("XHTTP 下行 Reality support-x25519mlkem768 需要布尔值");
            }
            out["reality-opts"]["support-x25519mlkem768"] = v.clone();
        }
    }
    Ok(out)
}
