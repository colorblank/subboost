use super::links::{Params, pick};
use super::*;
use std::collections::BTreeMap;

type Sections = BTreeMap<String, Vec<String>>;

pub(super) fn tokens(raw: &str) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut escaped = false;
    let mut depth = 0usize;
    for c in raw.chars() {
        if escaped {
            current.push(c);
            escaped = false;
            continue;
        }
        if c == '\\' && quote.is_some() {
            current.push(c);
            escaped = true;
            continue;
        }
        if let Some(q) = quote {
            current.push(c);
            if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '\'' | '"' => {
                quote = Some(c);
                current.push(c);
            }
            '[' | '{' | '(' => {
                depth += 1;
                if depth > 100 {
                    return Err(limited("配置行嵌套超过上限"));
                }
                current.push(c);
            }
            ']' | '}' | ')' => {
                if depth == 0 {
                    bail!("配置行括号不匹配");
                }
                depth -= 1;
                current.push(c);
            }
            ',' if depth == 0 => {
                out.push(current.trim().to_owned());
                current.clear();
                check_items(out.len())?;
            }
            _ => current.push(c),
        }
    }
    if quote.is_some() || depth != 0 {
        bail!("配置行引号或括号未闭合");
    }
    if !current.trim().is_empty() {
        out.push(current.trim().to_owned());
    }
    Ok(out)
}
fn unquote(raw: &str) -> String {
    let raw = raw.trim();
    if raw.starts_with('"') && raw.ends_with('"') {
        serde_json::from_str::<String>(raw).unwrap_or_else(|_| raw[1..raw.len() - 1].to_owned())
    } else if raw.starts_with('\'') && raw.ends_with('\'') {
        raw[1..raw.len() - 1].replace("''", "'")
    } else {
        raw.to_owned()
    }
}
fn assignments(items: &[String]) -> (Params, Vec<String>) {
    let mut params = Params::new();
    let mut extras = Vec::new();
    for item in items {
        if let Some((key, value)) = item.split_once('=') {
            params.insert(key.trim().to_ascii_lowercase(), unquote(value));
        } else if !item.is_empty() {
            extras.push(unquote(item));
        }
    }
    (params, extras)
}
fn aliases(params: &mut Params, target: &str, keys: &[&str]) {
    if !params.contains_key(target) {
        let value = pick(params, keys).to_owned();
        if !value.is_empty() {
            params.insert(target.into(), value);
        }
    }
}
fn node_endpoint(raw: &str) -> Result<(String, u16)> {
    let (server, p) = links::endpoint(&unquote(raw))?;
    if server.is_empty() {
        bail!("缺少服务器地址");
    }
    Ok((server, port(&json!(p))?))
}

pub(super) fn parse_config(content: &str, parsed: &mut Parsed) -> Result<()> {
    let mut sections = Sections::new();
    let mut section = String::new();
    for line in content.split(['\r', '\n']) {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            section = trimmed[1..trimmed.len() - 1].trim().to_ascii_lowercase();
            sections.entry(section.clone()).or_insert_with(Vec::new);
        } else if !section.is_empty() {
            sections
                .entry(section.clone())
                .or_default()
                .push(line.to_owned());
        }
    }
    let mut count = 0;
    for (name, lines) in &sections {
        if !["proxy", "server_local", "server_remote"].contains(&name.as_str()) {
            continue;
        }
        for line in lines
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty() && !s.starts_with(['#', ';']))
        {
            if line
                .split_once('=')
                .is_some_and(|(_, r)| r.trim().eq_ignore_ascii_case("direct"))
            {
                continue;
            }
            count += 1;
            check_items(count)?;
            let result = if line.contains("://")
                && !line
                    .split_once('=')
                    .is_some_and(|(l, _)| !l.contains("://"))
            {
                links::parse_link(line)
            } else {
                parse_line(line, &sections)
            };
            parsed.node_result(line, result)?;
        }
    }
    Ok(())
}

pub(super) fn parse_line(line: &str, sections: &Sections) -> Result<Option<Value>> {
    let Some((left, right)) = line.split_once('=') else {
        return Ok(None);
    };
    let left = left.trim();
    let right = right.trim();
    let qx_type = left.to_ascii_lowercase();
    let qx = [
        "shadowsocks",
        "shadowsocksr",
        "vmess",
        "vless",
        "trojan",
        "anytls",
        "hysteria",
        "hysteria2",
        "tuic",
        "http",
        "https",
        "socks5",
        "socks",
        "socks4",
        "ssh",
    ]
    .contains(&qx_type.as_str())
        && right.split(',').next().is_some_and(|v| v.contains(':'));
    let parts = tokens(right)?;
    if parts.is_empty() {
        return Ok(None);
    }
    let original = if qx {
        qx_type.as_str()
    } else {
        parts[0].trim()
    };
    let kind = match original.to_ascii_lowercase().as_str() {
        "shadowsocks" => "ss",
        "shadowsocksr" => "ssr",
        "socks" | "socks5-tls" | "socks5+tls" => "socks5",
        "hy" => "hysteria",
        "hy2" => "hysteria2",
        "tuic-v4" | "tuic-v5" => "tuic",
        other => other,
    }
    .to_owned();
    if kind == "direct" {
        return Ok(None);
    }
    if ![
        "ss",
        "ssr",
        "vmess",
        "vless",
        "trojan",
        "anytls",
        "snell",
        "hysteria",
        "hysteria2",
        "tuic",
        "wireguard",
        "http",
        "https",
        "socks4",
        "socks5",
        "ssh",
    ]
    .contains(&kind.as_str())
    {
        bail!("不支持的平台代理类型: {kind}");
    }
    let (mut params, extras) = assignments(if qx {
        &parts[1..]
    } else if kind == "wireguard" && parts.get(1).is_some_and(|v| v.contains('=')) {
        &parts[1..]
    } else if parts.len() >= 3 {
        &parts[3..]
    } else {
        &[]
    });
    let name = if qx {
        let name = pick(&params, &["tag", "remarks", "remark"]);
        if name.is_empty() {
            format!("{} {}", kind, parts[0])
        } else {
            name.to_owned()
        }
    } else {
        unquote(left)
    };
    if kind == "wireguard"
        && (params.contains_key("section-name") || params.contains_key("peers") || parts.len() < 3)
    {
        return wireguard(&name, &params, sections).map(Some);
    }
    let (server, p) = if qx {
        node_endpoint(&parts[0])?
    } else {
        if parts.len() < 3 {
            bail!("{kind} 配置行缺少 server/port");
        }
        let server = unquote(&parts[1]).trim_matches(['[', ']']).to_owned();
        let p = port(&json!(unquote(&parts[2])))?;
        (server, p)
    };
    let mut node = json!({"name":name,"type":kind,"server":server,"port":p});
    aliases(
        &mut params,
        "sni",
        &["tls-name", "tls-host", "server-name", "servername", "peer"],
    );
    aliases(
        &mut params,
        "fp",
        &[
            "client-fingerprint",
            "fingerprint",
            "tls-fingerprint",
            "server-cert-fingerprint-sha256",
            "tls-cert-sha256",
        ],
    );
    aliases(&mut params, "allow-insecure", &["skip-cert-verify"]);
    aliases(&mut params, "type", &["transport", "network"]);
    aliases(&mut params, "path", &["ws-path", "obfs-uri", "obfs-path"]);
    aliases(&mut params, "host", &["ws-host", "obfs-host"]);
    let obfs = pick(&params, &["obfs"]).to_ascii_lowercase();
    if kind != "ss" && kind != "ssr" && ["ws", "wss", "websocket", "http"].contains(&obfs.as_str())
    {
        params.insert(
            "type".into(),
            if obfs == "websocket" {
                "ws".into()
            } else if obfs == "wss" {
                "ws".into()
            } else {
                obfs.clone()
            },
        );
        if obfs == "wss" {
            params.insert("tls".into(), "true".into());
        }
    }
    links::common(&mut node, &params);
    let tls = if kind == "https" || original.contains("tls") {
        Some(true)
    } else {
        ["tls", "over-tls"]
            .iter()
            .find_map(|key| params.get(*key).and_then(|s| boolish(&json!(s))))
    };
    if let Some(tls) = tls {
        node["tls"] = json!(tls);
    }
    if qx && kind == "http" && tls == Some(true) {
        node["type"] = json!("https");
    }
    if node.get("udp").is_none()
        && ![
            "hysteria",
            "hysteria2",
            "tuic",
            "http",
            "https",
            "ssh",
            "snell",
        ]
        .contains(&kind.as_str())
    {
        node["udp"] = json!(kind != "socks4");
    }
    let extra = |i: usize| extras.get(i).map(String::as_str).unwrap_or("");
    let pass = pick(
        &params,
        &["password", "passwd", "auth", "auth-str", "auth_str"],
    );
    match kind.as_str() {
        "ss" | "ssr" => {
            let cipher = pick(
                &params,
                &["encrypt-method", "method", "cipher", "encryption"],
            );
            let cipher = if cipher.is_empty() && !extra(0).is_empty() {
                extra(0)
            } else if cipher.is_empty() {
                if kind == "ss" {
                    "aes-256-gcm"
                } else {
                    "aes-256-cfb"
                }
            } else {
                cipher
            };
            node["cipher"] = json!(cipher);
            node["password"] = json!(if pass.is_empty() { extra(1) } else { pass });
            if kind == "ssr" || params.contains_key("ssr-protocol") {
                node["type"] = json!("ssr");
                node["protocol"] =
                    json!(if pick(&params, &["protocol", "ssr-protocol"]).is_empty() {
                        "origin"
                    } else {
                        pick(&params, &["protocol", "ssr-protocol"])
                    });
                node["obfs"] = json!(if obfs.is_empty() { "plain" } else { &obfs });
                for (field, keys) in [
                    (
                        "protocol-param",
                        &["protocol-param", "ssr-protocol-param"][..],
                    ),
                    ("obfs-param", &["obfs-param", "obfs-host"][..]),
                ] {
                    let value = pick(&params, keys);
                    if !value.is_empty() {
                        node[field] = json!(value);
                    }
                }
            } else if !pick(&params, &["plugin"]).is_empty() {
                links::ss_plugin(
                    &mut node,
                    &format!(
                        "{};{}",
                        pick(&params, &["plugin"]),
                        pick(&params, &["plugin-opts", "plugin-options"])
                    ),
                )?;
            } else if ["http", "tls"].contains(&obfs.as_str()) {
                node["plugin"] = json!("obfs");
                let mut opts = json!({"mode":obfs});
                let host = pick(&params, &["obfs-host"]);
                if !host.is_empty() {
                    opts["host"] = json!(host);
                }
                let path = pick(&params, &["obfs-uri"]);
                if !path.is_empty() {
                    opts["path"] = json!(path);
                }
                node["plugin-opts"] = opts;
            }
        }
        "vmess" | "vless" => {
            let uuid = pick(&params, &["uuid", "id", "username", "user", "password"]);
            let uuid = if !uuid.is_empty() {
                uuid
            } else {
                extras
                    .iter()
                    .find(|s| s.len() >= 32 && s.contains('-'))
                    .map(String::as_str)
                    .unwrap_or(extra(0))
            };
            if uuid.is_empty() {
                bail!("{kind} 配置行缺少 uuid");
            }
            node["uuid"] = json!(uuid);
            if kind == "vmess" {
                let cipher = pick(
                    &params,
                    &["cipher", "encrypt-method", "method", "encryption"],
                );
                node["cipher"] = json!(if boolish(&json!(pick(&params, &["vmess-aead"])))
                    == Some(true)
                {
                    "none"
                } else if !cipher.is_empty() {
                    cipher
                } else if extras.len() > 1 {
                    extra(0)
                } else {
                    "auto"
                });
                let aid = pick(&params, &["alter-id", "alterid", "alter_id", "aid"]);
                node["alterId"] = json!(if aid.is_empty() {
                    u64::from(
                        qx && params.get("aead").and_then(|s| boolish(&json!(s))) == Some(false),
                    )
                } else {
                    integer(&json!(aid)).ok_or_else(|| anyhow!("VMess alter-id 无效"))?
                });
            } else {
                for (field, keys) in [
                    ("flow", &["flow"][..]),
                    ("encryption", &["encryption"][..]),
                    ("pbk", &["pbk", "public-key"][..]),
                    ("sid", &["sid", "short-id"][..]),
                ] {
                    let value = pick(&params, keys);
                    if !value.is_empty() {
                        node[field] = json!(value);
                    }
                }
                if node.get("pbk").is_some() {
                    node["tls"] = json!(true);
                    if node.get("client-fingerprint").is_none() {
                        node["client-fingerprint"] = json!("chrome");
                    }
                }
            }
            if params.contains_key("type") || boolish(&json!(pick(&params, &["ws"]))) == Some(true)
            {
                links::transport(&mut node, &params)?;
            }
        }
        "trojan" | "anytls" => {
            let pass = if pass.is_empty() { extra(0) } else { pass };
            if pass.is_empty() {
                bail!("{kind} 配置行缺少 password");
            }
            node["password"] = json!(pass);
            if kind == "anytls" {
                let transport = pick(&params, &["type"]);
                if !["", "none", "tcp"].contains(&transport) {
                    bail!("anytls 配置行不支持 transport={transport}");
                }
                if ["pbk", "sid", "public-key", "short-id"]
                    .iter()
                    .any(|k| params.contains_key(*k))
                {
                    bail!("anytls 配置行不支持 Reality 参数（Mihomo 不支持）");
                }
                for field in [
                    "idle-session-check-interval",
                    "idle-session-timeout",
                    "min-idle-session",
                ] {
                    if let Some(raw) = params.get(field) {
                        node[field] =
                            json!(integer(&json!(raw)).ok_or_else(|| anyhow!("{field} 需要整数"))?);
                    }
                }
                if let Some(raw) = params.get("padding-scheme") {
                    node["padding-scheme"] = json!(raw);
                }
            } else if params.contains_key("type")
                || boolish(&json!(pick(&params, &["ws"]))) == Some(true)
            {
                links::transport(&mut node, &params)?;
            }
        }
        "snell" => {
            let psk = pick(&params, &["psk", "password"]);
            node["psk"] = json!(if psk.is_empty() { extra(0) } else { psk });
            for field in ["version", "shadow-tls-version"] {
                if let Some(raw) = params.get(field) {
                    node[field] =
                        json!(integer(&json!(raw)).ok_or_else(|| anyhow!("{field} 无效"))?);
                }
            }
            for field in ["shadow-tls-sni", "shadow-tls-password"] {
                if let Some(raw) = params.get(field) {
                    node[field] = json!(raw);
                }
            }
            if ["http", "tls"].contains(&obfs.as_str()) {
                node["obfs-opts"] = json!({"mode":obfs,"host":pick(&params,&["obfs-host"])});
            }
        }
        "hysteria" | "hysteria2" => {
            node[if kind == "hysteria" {
                "auth-str"
            } else {
                "password"
            }] = json!(if pass.is_empty() { extra(0) } else { pass });
            let salamander = pick(&params, &["salamander-password", "obfs-password"]);
            if !salamander.is_empty() {
                node["obfs"] = json!(if obfs.is_empty() { "salamander" } else { &obfs });
                node["obfs-password"] = json!(salamander);
            }
            for (field, keys) in [
                ("up", &["up", "upload-bandwidth"][..]),
                ("down", &["down", "download-bandwidth"][..]),
                (
                    "fingerprint",
                    &["tls-fingerprint", "server-cert-fingerprint-sha256", "fp"][..],
                ),
            ] {
                let raw = pick(&params, keys);
                if !raw.is_empty() {
                    node[field] = json!(raw);
                }
            }
            let hop = pick(&params, &["hop-interval", "port-hopping-interval"]);
            if !hop.is_empty() {
                node["hop-interval"] = integer(&json!(hop))
                    .map(|v| json!(v))
                    .unwrap_or_else(|| json!(hop));
            }
        }
        "tuic" => {
            if original == "tuic-v5" {
                node["version"] = json!(5);
            } else if original == "tuic-v4" {
                node["version"] = json!(4);
            }
            let token = pick(&params, &["token"]);
            let uuid = pick(&params, &["uuid", "username"]);
            if !token.is_empty() || (uuid.is_empty() && pass.is_empty()) {
                node["token"] = json!(if token.is_empty() { extra(0) } else { token });
            } else {
                node["uuid"] = json!(uuid);
                node["password"] = json!(pass);
            }
            for (field, keys) in [
                (
                    "congestion-controller",
                    &["congestion-controller", "congestion-control"][..],
                ),
                ("udp-relay-mode", &["udp-relay-mode"][..]),
            ] {
                let raw = pick(&params, keys);
                if !raw.is_empty() {
                    node[field] = json!(raw);
                }
            }
        }
        "wireguard" => {
            for field in [
                "private-key",
                "public-key",
                "pre-shared-key",
                "ip",
                "ipv6",
                "mtu",
                "reserved",
                "keepalive",
                "persistent-keepalive",
            ] {
                if let Some(raw) = params.get(field) {
                    node[field] = json!(raw);
                }
            }
        }
        _ => {
            for (field, keys) in [
                ("username", &["username", "user"][..]),
                ("password", &["password", "passwd"][..]),
                ("private-key", &["private-key"][..]),
                ("private-key-passphrase", &["private-key-passphrase"][..]),
                ("tls-cert-sha256", &["tls-cert-sha256"][..]),
            ] {
                let raw = pick(&params, keys);
                if !raw.is_empty() {
                    node[field] = json!(raw);
                }
            }
            if let Some(h) = links::headers(pick(&params, &["headers", "header"])) {
                node["headers"] = h;
            }
            if kind == "ssh" && !pick(&params, &["server-fingerprint"]).is_empty() {
                node["tls-fingerprint"] = json!(pick(&params, &["server-fingerprint"]));
            }
        }
    }
    for field in ["ecn", "quic", "health-check", "remote-dns-resolve", "mptcp"] {
        if let Some(value) = params.get(field).and_then(|s| boolish(&json!(s))) {
            node[field] = json!(value);
        }
    }
    let ports = pick(&params, &["ports", "port-hopping"]);
    if !ports.is_empty() {
        node["ports"] = json!(port_spec(ports)?.0);
    }
    for field in [
        "block-quic",
        "disable-sni",
        "reduce-rtt",
        "reuse",
        "udp-over-tcp",
        "fast-open",
    ] {
        if let Some(value) = params.get(field).and_then(|s| boolish(&json!(s))) {
            node[field] = json!(value);
        }
    }
    for field in [
        "udp-port",
        "mtu",
        "request-timeout",
        "heartbeat-interval",
        "max-open-streams",
        "max-idle-time",
    ] {
        if let Some(raw) = params.get(field) {
            node[field] = json!(integer(&json!(raw)).ok_or_else(|| anyhow!("{field} 无效"))?);
        }
    }
    if ["hysteria", "hysteria2", "tuic"].contains(&kind.as_str()) && !params.contains_key("tfo") {
        node.as_object_mut().map(|m| m.remove("tfo"));
    }
    Ok(Some(node))
}

fn wireguard(name: &str, params: &Params, sections: &Sections) -> Result<Value> {
    let section = pick(params, &["section-name"]);
    let mut values = params.clone();
    let mut peer_list = Vec::new();
    if !section.is_empty() {
        let key = format!("wireguard {}", section.to_ascii_lowercase());
        let lines = sections
            .get(&key)
            .filter(|lines| !lines.is_empty())
            .ok_or_else(|| anyhow!("未找到 WireGuard section: {section}"))?;
        for line in lines {
            if let Some((key, value)) = line.split_once('=') {
                let key = key.trim().to_ascii_lowercase();
                if key == "peer" {
                    peer_list.push(
                        value
                            .trim()
                            .trim_start_matches('(')
                            .trim_end_matches(')')
                            .to_owned(),
                    );
                } else {
                    values.insert(key, unquote(value));
                }
            }
        }
    } else if let Some(peers) = params.get("peers") {
        let mut depth = 0;
        let mut start = None;
        let mut quote = None;
        for (index, c) in peers.char_indices() {
            if let Some(q) = quote {
                if c == q {
                    quote = None;
                }
                continue;
            }
            if c == '\'' || c == '"' {
                quote = Some(c);
                continue;
            }
            if c == '{' {
                if depth == 0 {
                    start = Some(index + 1);
                }
                depth += 1;
            }
            if c == '}' {
                if depth == 0 {
                    bail!("WireGuard peers 格式无效");
                }
                depth -= 1;
                if depth == 0 {
                    if let Some(start) = start {
                        peer_list.push(peers[start..index].to_owned());
                    }
                }
            }
        }
        if depth != 0 {
            bail!("WireGuard peers 格式无效");
        }
    } else {
        bail!("Surge WireGuard 缺少 section-name");
    }
    check_items(peer_list.len())?;
    let mut peers = Vec::new();
    for peer in peer_list {
        let (peer, _) = assignments(&tokens(&peer)?);
        let endpoint = pick(&peer, &["endpoint"]);
        if endpoint.is_empty() {
            continue;
        }
        let (server, p) = node_endpoint(endpoint)?;
        let mut node = json!({"server":server,"port":p});
        for (target, keys) in [
            ("public-key", &["public-key"][..]),
            ("pre-shared-key", &["pre-shared-key", "preshared-key"][..]),
        ] {
            let raw = pick(&peer, keys);
            if !raw.is_empty() {
                node[target] = json!(raw);
            }
        }
        let allowed = pick(&peer, &["allowed-ips"]);
        if !allowed.is_empty() {
            node["allowed-ips"] = json!(list(allowed));
        }
        if let Some(reserved) =
            validation::reserved(&json!(pick(&peer, &["reserved", "client-id"])))
        {
            node["reserved"] = reserved;
        }
        peers.push(node);
    }
    if peers.is_empty() {
        bail!("WireGuard section {section} 缺少有效 endpoint");
    }
    let first = peers[0].clone();
    let mut node = json!({"name":name,"type":"wireguard","server":first["server"],"port":first["port"],"udp":true});
    for (target, keys) in [
        ("private-key", &["private-key"][..]),
        ("ip", &["interface-ip", "self-ip", "ip"][..]),
        ("ipv6", &["interface-ipv6", "self-ip-v6", "ipv6"][..]),
        (
            "persistent-keepalive",
            &["persistent-keepalive", "keepalive"][..],
        ),
    ] {
        let raw = pick(&values, keys);
        if !raw.is_empty() {
            node[target] = json!(raw);
        }
    }
    if let Some(raw) = values.get("mtu") {
        node["mtu"] = json!(integer(&json!(raw)).ok_or_else(|| anyhow!("WireGuard mtu 无效"))?);
    }
    for field in ["public-key", "pre-shared-key", "allowed-ips", "reserved"] {
        if let Some(v) = first.get(field) {
            node[field] = v.clone();
        }
    }
    if peers.len() > 1 {
        node["peers"] = json!(peers);
    }
    let dns = ["dns-server", "dns", "dnsv6"]
        .iter()
        .filter_map(|key| values.get(*key))
        .flat_map(|raw| list(raw))
        .collect::<Vec<_>>();
    if !dns.is_empty() {
        node["dns"] = json!(dns);
        node["remote-dns-resolve"] = json!(true);
    }
    Ok(node)
}
