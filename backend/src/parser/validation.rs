use super::*;

pub(super) fn stable_key(value: &Value) -> String {
    fn sort(value: &Value) -> Value {
        match value {
            Value::Object(map) => {
                let mut keys: Vec<_> = map.keys().collect();
                keys.sort();
                Value::Object(
                    keys.into_iter()
                        .map(|k| (k.clone(), sort(&map[k])))
                        .collect(),
                )
            }
            Value::Array(items) => Value::Array(items.iter().map(sort).collect()),
            other => other.clone(),
        }
    }
    sort(value).to_string()
}
pub(super) fn strip_control(node: &mut Value) {
    if let Some(map) = node.as_object_mut() {
        for key in [
            "_meta",
            "_id",
            "_source",
            "_sourceId",
            "_sourceUrl",
            "_sourceName",
            "_subboost-invalid-mihomo-node",
            "dialer-proxy",
            "dialer_proxy",
        ] {
            map.remove(key);
        }
    }
}
pub(super) fn info_name(name: &str) -> bool {
    [
        "剩余",
        "流量",
        "到期",
        "过期",
        "重置",
        "套餐",
        "余额",
        "会员",
        "官网",
        "剩餘",
        "到期",
        "expire",
        "traffic",
        "remaining",
        "reset",
        "balance",
    ]
    .iter()
    .any(|s| name.to_lowercase().contains(s))
}
pub(super) fn placeholder(node: &Value) -> bool {
    let server = text(node, "server")
        .trim()
        .trim_end_matches('.')
        .to_ascii_lowercase();
    let name = text(node, "name").to_lowercase();
    ["127.0.0.1", "localhost", "0.0.0.0", "::1", "[::1]"].contains(&server.as_str())
        && integer(&node["port"]).is_some_and(|p| p <= 1)
        && [
            "v2rayn", "clash", "版本", "太旧", "过旧", "更新", "upgrade", "update",
        ]
        .iter()
        .any(|s| name.contains(s))
}
fn alias(map: &mut Map<String, Value>, target: &str, aliases: &[&str]) {
    if !map.get(target).is_some_and(|v| !v.is_null() && v != "") {
        if let Some(v) = aliases
            .iter()
            .filter_map(|key| map.get(*key))
            .find(|v| !v.is_null() && *v != "")
            .cloned()
        {
            map.insert(target.into(), v);
        }
    }
    for key in aliases {
        map.remove(*key);
    }
}
fn scalar(value: &Value) -> Result<String> {
    match value {
        Value::String(s) => Ok(s.clone()),
        Value::Number(n)
            if n.as_i64()
                .is_some_and(|v| v.unsigned_abs() <= 9_007_199_254_740_991)
                || n.as_u64().is_some_and(|v| v <= 9_007_199_254_740_991) =>
        {
            Ok(n.to_string())
        }
        _ => bail!("必须是字符串或可无损表示的整数"),
    }
}
pub(super) fn reserved(value: &Value) -> Option<Value> {
    if let Some(s) = value.as_str() {
        if s.trim().len() == 4 {
            if let Ok(bytes) = STANDARD.decode(s.trim()) {
                if bytes.len() == 3 {
                    return Some(json!(bytes));
                }
            }
        }
    }
    let values: Vec<Value> = match value {
        Value::Array(items) => items.clone(),
        Value::String(s) => s
            .trim_matches(['[', ']'])
            .split([',', '/'])
            .map(|v| json!(v.trim()))
            .collect(),
        _ => return None,
    };
    if values.len() != 3 {
        return None;
    }
    values
        .iter()
        .map(|v| integer(v).filter(|v| *v <= 255))
        .collect::<Option<Vec<_>>>()
        .map(|v| json!(v))
}
pub(super) fn short_id(value: &Value) -> Option<String> {
    let raw = value
        .as_str()
        .map(str::to_owned)
        .or_else(|| value.as_u64().map(|n| n.to_string()))?;
    let raw = raw.trim().trim_start_matches("0x");
    if raw.is_empty()
        || raw.len() > 16
        || raw.len() % 2 != 0
        || !raw.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return None;
    }
    Some(raw.to_ascii_lowercase())
}
pub(super) fn reality_key(raw: &str) -> bool {
    raw.len() == 43
        && raw
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ValidationMode {
    Imported,
    Persisted,
}

pub(super) fn normalize(node: &mut Value, mode: ValidationMode) -> Result<()> {
    let imported = mode == ValidationMode::Imported;
    let map = node.as_object_mut().ok_or_else(|| anyhow!("必须是对象"))?;
    let kind = map
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if kind.is_empty() || kind.chars().any(char::is_control) {
        bail!("缺少有效类型");
    }
    let kind = match kind.as_str() {
        "hy2" => "hysteria2",
        "hy" => "hysteria",
        "socks" => "socks5",
        other => other,
    }
    .to_owned();
    let name = map.get("name").and_then(Value::as_str).unwrap_or("").trim();
    if name.is_empty() && !imported {
        bail!("缺少有效名称");
    }
    if name.chars().any(char::is_control) {
        bail!("名称不能包含控制字符");
    }
    let name = if name.is_empty() {
        "未命名节点".to_owned()
    } else {
        name.to_owned()
    };
    map.insert("name".into(), json!(name));
    map.insert("type".into(), json!(kind));
    for field in STRING_FIELDS {
        if let Some(value) = map.get(*field).filter(|v| !v.is_null()) {
            let normalized = scalar(value).map_err(|e| anyhow!("{field} {e}"))?;
            map.insert((*field).into(), json!(normalized));
        }
    }
    if ["vmess", "vless", "trojan", "anytls", "trusttunnel"].contains(&kind.as_str()) {
        if map
            .get("fingerprint")
            .and_then(Value::as_str)
            .and_then(certificate_fingerprint)
            .is_some()
        {
            alias(map, "client-fingerprint", &["fp", "clientFingerprint"]);
        } else {
            alias(
                map,
                "client-fingerprint",
                &["fp", "fingerprint", "clientFingerprint"],
            );
        }
    }
    if ["vmess", "vless"].contains(&kind.as_str()) {
        alias(
            map,
            "packet-encoding",
            &["packetEncoding", "packet_encoding", "packetencoding"],
        );
    }
    for field in [
        "skip-cert-verify",
        "skip_cert_verify",
        "allowInsecure",
        "allow_insecure",
        "allow-insecure",
        "insecure",
    ] {
        if let Some(value) = map.get(field).and_then(boolish) {
            if field == "skip-cert-verify" || !map.contains_key("skip-cert-verify") {
                map.insert("skip-cert-verify".into(), json!(value));
            }
        }
        if field != "skip-cert-verify" {
            map.remove(field);
        }
    }
    if let Some(value) = map.remove("tls-verification").as_ref().and_then(boolish) {
        map.insert("skip-cert-verify".into(), json!(!value));
    }
    if ["vmess", "vless", "trojan"].contains(&kind.as_str()) {
        let service = [
            "serviceName",
            "service_name",
            "service-name",
            "servicename",
            "grpc-service-name",
        ]
        .iter()
        .find_map(|key| map.remove(*key));
        if service.is_some() || map.contains_key("grpc-opts") {
            let grpc = map.entry("grpc-opts").or_insert_with(|| json!({}));
            if let Some(grpc) = grpc.as_object_mut() {
                alias(
                    grpc,
                    "grpc-service-name",
                    &["serviceName", "service_name", "service-name"],
                );
                if let Some(service) = service {
                    grpc.entry("grpc-service-name").or_insert(service);
                }
            }
        }
    }
    if kind == "vless" {
        let mut reality = map.remove("reality-opts");
        for (target, aliases) in [
            (
                "public-key",
                &["pbk", "public-key", "publicKey", "public_key"][..],
            ),
            (
                "short-id",
                &["sid", "short-id", "shortId", "short_id", "shortid"][..],
            ),
        ] {
            let value = aliases.iter().find_map(|key| map.remove(*key));
            if let Some(value) = value {
                if reality.is_none() {
                    reality = Some(json!({}));
                }
                if let Some(ro) = reality.as_mut().and_then(Value::as_object_mut) {
                    ro.entry(target).or_insert(value);
                }
            }
        }
        if let Some(mut reality) = reality {
            if let Some(ro) = reality.as_object_mut() {
                alias(ro, "public-key", &["pbk", "publicKey", "public_key"]);
                alias(ro, "short-id", &["sid", "shortId", "short_id"]);
                alias(
                    ro,
                    "support-x25519mlkem768",
                    &["supportX25519MLKEM768", "support_x25519mlkem768"],
                );
                if let Some(value) = ro.get("public-key").and_then(Value::as_str) {
                    ro.insert("public-key".into(), json!(value.trim()));
                }
                if let Some(value) = ro.get("short-id").and_then(short_id) {
                    ro.insert("short-id".into(), json!(value));
                } else {
                    ro.remove("short-id");
                }
                if let Some(value) = ro.get("support-x25519mlkem768").and_then(boolish) {
                    ro.insert("support-x25519mlkem768".into(), json!(value));
                }
            }
            map.insert("reality-opts".into(), reality);
        }
    }
    if kind == "wireguard" {
        alias(
            map,
            "private-key",
            &["privatekey", "privateKey", "private_key"],
        );
        alias(
            map,
            "public-key",
            &[
                "publickey",
                "publicKey",
                "public_key",
                "peer-public-key",
                "peerPublicKey",
            ],
        );
        alias(
            map,
            "pre-shared-key",
            &[
                "presharedkey",
                "preshared-key",
                "preSharedKey",
                "pre_shared_key",
                "psk",
            ],
        );
        alias(
            map,
            "persistent-keepalive",
            &["keepalive", "persistent_keepalive", "persistentKeepalive"],
        );
        if let Some(value) = map.get("persistent-keepalive") {
            let n = integer(value)
                .filter(|v| *v <= 65535)
                .ok_or_else(|| anyhow!("WireGuard persistent-keepalive 无效"))?;
            map.insert("persistent-keepalive".into(), json!(n));
        }
        if let Some(value) = map.get("reserved") {
            if let Some(v) = reserved(value) {
                map.insert("reserved".into(), v);
            } else {
                map.remove("reserved");
            }
        }
        if let Some(peers) = map.get_mut("peers").and_then(Value::as_array_mut) {
            for peer in peers {
                if let Some(peer) = peer.as_object_mut() {
                    if imported {
                        if let Some(p) = peer.get("port").and_then(integer) {
                            peer.insert("port".into(), json!(p));
                        }
                    }
                    if let Some(value) = peer.get("reserved") {
                        if let Some(value) = reserved(value) {
                            peer.insert("reserved".into(), value);
                        } else {
                            peer.remove("reserved");
                        }
                    }
                }
            }
        }
    }
    if kind == "snell" {
        let mut aliases = Map::new();
        for (key, target) in [
            ("shadow-tls-version", "version"),
            ("shadow-tls-sni", "host"),
            ("shadow-tls-password", "password"),
        ] {
            if let Some(value) = map.remove(key) {
                aliases.insert(target.into(), value);
            }
        }
        if !aliases.is_empty()
            && (!map.contains_key("obfs-opts")
                || map
                    .get("obfs-opts")
                    .is_some_and(|v| text(v, "mode") == "shadow-tls"))
        {
            let existing = map
                .remove("obfs-opts")
                .and_then(|v| v.as_object().cloned())
                .unwrap_or_default();
            aliases.insert("mode".into(), json!("shadow-tls"));
            aliases.extend(existing);
            map.insert("obfs-opts".into(), Value::Object(aliases));
        }
    }
    if kind == "tailscale" {
        for field in [
            "hostname",
            "auth-key",
            "control-url",
            "state-dir",
            "exit-node",
        ] {
            if let Some(value) = map.get(field) {
                if value.is_null() {
                    map.remove(field);
                } else {
                    let value =
                        scalar(value).map_err(|_| anyhow!("Tailscale {field} 必须是字符串"))?;
                    map.insert(field.into(), json!(value));
                }
            }
        }
        if let Some(raw) = map
            .get("control-url")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            let url = url::Url::parse(raw)
                .map_err(|_| anyhow!("Tailscale control-url 必须是有效的 HTTP 或 HTTPS 地址"))?;
            if !["http", "https"].contains(&url.scheme()) || url.host_str().is_none() {
                bail!("Tailscale control-url 必须是有效的 HTTP 或 HTTPS 地址");
            }
        }
        for field in [
            "ephemeral",
            "udp",
            "accept-routes",
            "exit-node-allow-lan-access",
        ] {
            if let Some(value) = map.get(field) {
                if value.is_null() {
                    map.remove(field);
                } else {
                    let value =
                        boolish(value).ok_or_else(|| anyhow!("Tailscale {field} 必须是布尔值"))?;
                    map.insert(field.into(), json!(value));
                }
            }
        }
    }
    if imported {
        if let Some(value) = map.get("port") {
            let p = port(value)?;
            map.insert("port".into(), json!(p));
        }
        if ["hysteria", "hysteria2"].contains(&kind.as_str()) {
            if let Some(raw) = map.get("ports").and_then(|v| {
                v.as_str()
                    .map(str::to_owned)
                    .or_else(|| v.as_u64().map(|n| n.to_string()))
            }) {
                let (ports, p) = port_spec(&raw)?;
                map.insert("ports".into(), json!(ports));
                if kind == "hysteria2" && !map.contains_key("port") {
                    map.insert("port".into(), json!(p));
                }
            }
        }
        match kind.as_str() {
            "ss" => {
                map.entry("cipher").or_insert(json!("aes-256-gcm"));
                map.entry("password").or_insert(json!(""));
            }
            "ssr" => {
                map.entry("cipher").or_insert(json!("aes-256-cfb"));
                map.entry("protocol").or_insert(json!("origin"));
                map.entry("obfs").or_insert(json!("plain"));
            }
            "vmess" => {
                map.entry("cipher").or_insert(json!("auto"));
                let aid = map.get("alterId").and_then(integer).unwrap_or(0);
                map.insert("alterId".into(), json!(aid));
            }
            "hysteria" => {
                map.entry("protocol").or_insert(json!("udp"));
            }
            _ => {}
        }
        if kind == "ss" {
            super::links::normalize_ss_plugin(map);
        }
        if let Some(ws) = map.get_mut("ws-opts") {
            normalize_ws(ws, false);
        }
    }
    endpoint(node)
}

fn endpoint(node: &Value) -> Result<()> {
    let kind = text(node, "type");
    if kind == "relay" {
        if !node
            .get("proxies")
            .and_then(Value::as_array)
            .is_some_and(|items| {
                !items.is_empty()
                    && items
                        .iter()
                        .all(|v| v.as_str().is_some_and(|s| !s.trim().is_empty()))
            })
        {
            bail!("relay proxies 必须是非空字符串数组");
        }
        return Ok(());
    }
    if ["direct", "dns", "reject", "tailscale"].contains(&kind) {
        return Ok(());
    }
    if kind == "wireguard" {
        if let Some(peers) = node.get("peers") {
            let peers = peers
                .as_array()
                .filter(|p| !p.is_empty())
                .ok_or_else(|| anyhow!("WireGuard peers 必须是非空数组"))?;
            check_items(peers.len())?;
            for (i, peer) in peers.iter().enumerate() {
                if !peer.is_object()
                    || text(peer, "server").trim().is_empty()
                    || port(&peer["port"]).is_err()
                    || !peer["port"].is_number()
                    || text(peer, "public-key").trim().is_empty()
                {
                    bail!(
                        "WireGuard peer #{} 缺少有效 server、port 或 public-key",
                        i + 1
                    );
                }
            }
            if node.get("server").is_none() && node.get("port").is_none() {
                return Ok(());
            }
        }
    }
    let server = text(node, "server").trim();
    if server.is_empty() || server.chars().any(|c| c.is_control() || c.is_whitespace()) {
        bail!("缺少有效服务器地址");
    }
    if kind == "mieru" && node.get("port-range").is_some() {
        if node.get("port").is_some() {
            bail!("Mieru port 与 port-range 不能同时设置");
        }
        let range = text(node, "port-range");
        if !range.contains('-') || range.contains(',') || port_spec(range).is_err() {
            bail!("Mieru port-range 必须是 1 到 65535 内的递增范围");
        }
        return Ok(());
    }
    if !node["port"].is_number() {
        bail!("端口必须是 1 到 65535 的整数");
    }
    port(&node["port"])?;
    Ok(())
}

pub(super) fn normalize_ws(value: &mut Value, generation: bool) {
    if !value.is_object() {
        return;
    }
    let upgrade = value.get("v2ray-http-upgrade").and_then(boolish) == Some(true);
    if let Some(path) = value.get("path").and_then(Value::as_str).map(str::to_owned) {
        let (path, early) = ws_path(&path);
        value["path"] = json!(path);
        if let Some(n) = early {
            if value.get("max-early-data").is_none() {
                value["max-early-data"] = json!(n);
                value["early-data-header-name"] = json!("Sec-WebSocket-Protocol");
            }
        }
    }
    if generation && upgrade {
        if let Some(map) = value.as_object_mut() {
            map.remove("max-early-data");
            map.remove("early-data-header-name");
            map.remove("_v2ray-http-upgrade-ed");
        }
    }
}
pub(super) fn ws_path(path: &str) -> (String, Option<u64>) {
    let Some((base, query)) = path.split_once('?') else {
        return (path.to_owned(), None);
    };
    let mut early = None;
    let mut kept = Vec::new();
    for part in query.split('&') {
        if let Some(("ed", n)) = part.split_once('=') {
            if let Ok(n) = n.parse::<u64>() {
                if n <= 9_007_199_254_740_991 {
                    early = Some(n);
                    continue;
                }
            }
        }
        kept.push(part);
    }
    (
        if kept.is_empty() {
            base.to_owned()
        } else {
            format!("{base}?{}", kept.join("&"))
        },
        early,
    )
}

pub fn sanitize_for_generation(node: &Value) -> Option<Value> {
    let mut node = node.clone();
    normalize(&mut node, ValidationMode::Persisted).ok()?;
    let kind = text(&node, "type").to_owned();
    if ["socks4", "h2-connect", "trust-tunnel", "external"].contains(&kind.as_str()) {
        return None;
    }
    for field in match kind.as_str() {
        "ss" => &["cipher", "password"][..],
        "ssr" => &["cipher", "password", "protocol", "obfs"],
        "vmess" | "vless" => &["uuid"],
        "trojan" | "anytls" | "hysteria2" => &["password"],
        "snell" => &["psk"],
        _ => &[],
    } {
        if text(&node, field).trim().is_empty() {
            return None;
        }
    }
    if kind == "wireguard" {
        let valid = |s: &str| {
            s.len() == 44 && s.ends_with('=') && STANDARD.decode(s).is_ok_and(|b| b.len() == 32)
        };
        if !valid(text(&node, "private-key")) {
            return None;
        }
        for key in ["public-key", "pre-shared-key"] {
            if node
                .get(key)
                .is_some_and(|v| !v.as_str().is_some_and(valid))
            {
                return None;
            }
        }
        if let Some(peers) = node.get("peers").and_then(Value::as_array) {
            for peer in peers {
                if !valid(text(peer, "public-key"))
                    || peer
                        .get("pre-shared-key")
                        .is_some_and(|v| !v.as_str().is_some_and(valid))
                {
                    return None;
                }
            }
        }
    }
    if kind == "ssh" {
        if text(&node, "password").is_empty() && !valid_pem(text(&node, "private-key")) {
            return None;
        }
        if let Some(keys) = node.get("host-key") {
            if !keys
                .as_array()
                .is_some_and(|a| a.iter().all(|v| v.as_str().is_some_and(ssh_host_key)))
            {
                return None;
            }
        }
    }
    let map = node.as_object_mut()?;
    for field in [
        "udp",
        "tls",
        "skip-cert-verify",
        "tfo",
        "fast-open",
        "mptcp",
        "reuse",
        "disable-sni",
        "reduce-rtt",
        "authenticated-length",
        "global-padding",
        "udp-over-tcp",
    ] {
        if let Some(value) = map.get(field) {
            if let Some(b) = boolish(value) {
                map.insert(field.into(), json!(b));
            } else {
                map.remove(field);
            }
        }
    }
    if let Some(value) = map.get("alpn") {
        let items = if let Some(s) = value.as_str() {
            list(s)
        } else {
            value
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(|s| json!(s))
                        .collect()
                })
                .unwrap_or_default()
        };
        if items.is_empty() {
            map.remove("alpn");
        } else {
            map.insert("alpn".into(), json!(items));
        }
    }
    if kind == "ss"
        && ["v2ray-plugin", "xray-plugin"]
            .contains(&map.get("plugin").and_then(Value::as_str).unwrap_or(""))
    {
        let mode = map
            .get("plugin-opts")
            .map(|v| text(v, "mode"))
            .unwrap_or("");
        if !["", "ws", "websocket"].contains(&mode) {
            return None;
        }
    }
    if let Some(ws) = map.get_mut("ws-opts") {
        if ws.is_object() {
            normalize_ws(ws, true);
        } else {
            map.remove("ws-opts");
        }
    }
    if kind == "https" {
        map.insert("type".into(), json!("http"));
        if map.get("tls") != Some(&json!(false)) {
            map.insert("tls".into(), json!(true));
        }
    }
    if kind == "vless" {
        if let Some(reality) = map.get_mut("reality-opts") {
            let ro = reality.as_object_mut()?;
            let key = ro
                .get("public-key")
                .and_then(Value::as_str)?
                .trim()
                .to_owned();
            if !reality_key(&key) {
                return None;
            }
            ro.insert("public-key".into(), json!(key));
            map.insert("tls".into(), json!(true));
            map.entry("client-fingerprint").or_insert(json!("chrome"));
        }
        if let Some(opts) = map.get_mut("xhttp-opts") {
            super::links::normalize_xhttp(opts).ok()?;
        }
        if let Some(s) = map.get("encryption").and_then(Value::as_str) {
            if s.starts_with("mlkem768x25519plus") && !valid_mlkem(s) {
                map.remove("encryption");
            }
        }
    }
    if kind == "ssh" {
        if !map
            .get("private-key")
            .and_then(Value::as_str)
            .is_some_and(valid_pem)
        {
            map.remove("private-key");
            map.remove("private-key-passphrase");
        }
        if let Some(fp) = map.get("server-fingerprint").and_then(Value::as_str) {
            if !fp.starts_with("SHA256:") || !(50..=51).contains(&fp.len()) {
                map.remove("server-fingerprint");
            }
        }
    }
    if let Some(fingerprint) = map.get("fingerprint").and_then(Value::as_str) {
        if let Some(fingerprint) = certificate_fingerprint(fingerprint) {
            map.insert("fingerprint".into(), json!(fingerprint));
        } else {
            map.remove("fingerprint");
        }
    }
    sanitize_ech(map);
    strip_control(&mut node);
    strip_internal(&mut node);
    Some(node)
}
fn strip_internal(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.retain(|key, _| !key.starts_with('_'));
            for v in map.values_mut() {
                strip_internal(v);
            }
        }
        Value::Array(items) => {
            for v in items {
                strip_internal(v);
            }
        }
        _ => {}
    }
}
fn sanitize_ech(map: &mut Map<String, Value>) {
    if let Some(ech) = map.get_mut("ech-opts") {
        if let Some(ech) = ech.as_object_mut() {
            if let Some(value) = ech.get("enable").and_then(boolish) {
                ech.insert("enable".into(), json!(value));
            } else {
                ech.remove("enable");
            }
            if let Some(config) = ech.get("config").and_then(Value::as_str).map(str::to_owned) {
                if STANDARD.decode(&config).is_err() {
                    ech.remove("config");
                    if super::links::ech_hostname(&config) && !ech.contains_key("query-server-name")
                    {
                        ech.insert("query-server-name".into(), json!(config));
                    }
                }
            }
        } else {
            map.remove("ech-opts");
        }
    }
    for key in ["xhttp-opts", "download-settings"] {
        if let Some(child) = map.get_mut(key).and_then(Value::as_object_mut) {
            sanitize_ech(child);
        }
    }
}
fn valid_pem(s: &str) -> bool {
    s.trim_start().starts_with("-----BEGIN ")
        && s.lines()
            .next()
            .is_some_and(|l| l.ends_with("PRIVATE KEY-----"))
}
fn ssh_host_key(s: &str) -> bool {
    let mut parts = s.split_whitespace();
    let kind = parts.next().unwrap_or("");
    let data = parts.next().unwrap_or("");
    ([
        "ssh-rsa",
        "ssh-dss",
        "ssh-ed25519",
        "ecdsa-sha2-nistp256",
        "ecdsa-sha2-nistp384",
        "ecdsa-sha2-nistp521",
    ]
    .contains(&kind)
        || kind.starts_with("ssh-ecdsa-"))
        && !data.is_empty()
        && data
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"+/=".contains(&c))
}
pub(super) fn valid_mlkem(s: &str) -> bool {
    s.split('.').count() >= 4
        && s.starts_with("mlkem768x25519plus.")
        && !s.chars().any(char::is_whitespace)
}

fn certificate_fingerprint(value: &str) -> Option<String> {
    use std::sync::LazyLock;
    static PREFIX: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"(?i)^(?:sha256\s+fingerprint\s*=\s*|sha256[:=]\s*)")
            .expect("certificate prefix expression")
    });
    let raw = PREFIX.replace(value.trim(), "");
    let compact: String = raw
        .chars()
        .filter(|character| *character != ':')
        .flat_map(char::to_lowercase)
        .collect();
    (compact.len() == 64 && compact.bytes().all(|byte| byte.is_ascii_hexdigit())).then_some(compact)
}
