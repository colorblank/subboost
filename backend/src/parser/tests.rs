use super::*;

fn nodes(input: &str) -> Vec<Value> {
    let result = parse(input).unwrap();
    assert_eq!(result["errors"], json!([]), "{result}");
    result["nodes"].as_array().unwrap().clone()
}

#[test]
fn share_links_keep_credentials_reality_and_early_data() {
    let result = nodes(&format!(
        "vless://11111111-1111-4111-8111-111111111111@edge.example:443?security=reality&type=ws&path=%2Fws%3Fa%3D1%26ed%3D1024%26b%3D2&host=cdn.example&pbk={}&sid=0x7250&support-x25519mlkem768=false#VLESS%23Remark",
        "A".repeat(43)
    ));
    assert_eq!(result[0]["name"], "VLESS#Remark");
    assert_eq!(result[0]["ws-opts"]["path"], "/ws?a=1&b=2");
    assert_eq!(result[0]["ws-opts"]["max-early-data"], 1024);
    assert_eq!(result[0]["reality-opts"]["short-id"], "7250");
    assert_eq!(result[0]["reality-opts"]["support-x25519mlkem768"], false);
    let trojan =
        nodes("trojan://pa%3Ass%40secret@edge.example?type=httpupgrade&path=%2Fws%3Fed%3D2048#one");
    assert_eq!(trojan[0]["password"], "pa:ss@secret");
    assert_eq!(trojan[0]["ws-opts"]["max-early-data"], 2048);
    assert!(
        sanitize_for_generation(&trojan[0]).unwrap()["ws-opts"]
            .get("max-early-data")
            .is_none()
    );
}

#[test]
fn subscriptions_decode_unicode_and_normalize_duplicates() {
    let credential = STANDARD.encode("aes-128-gcm:p@ss");
    let link = format!(
        "ss://{credential}@edge.example:8388?plugin=obfs-local%3Bobfs%3Dtls%3Bobfs-host%3Dcdn.example#香港"
    );
    let result = nodes(&URL_SAFE_NO_PAD.encode(format!("{link}\n{link}")));
    assert_eq!(result.len(), 1);
    assert_eq!(result[0]["name"], "香港");
    assert_eq!(result[0]["password"], "p@ss");
    assert_eq!(result[0]["plugin"], "obfs");
    assert_eq!(result[0]["plugin-opts"]["host"], "cdn.example");
    let payload = STANDARD.encode(format!(
        "edge.example:443:auth_chain_a:aes-128-gcm:tls1.2_ticket_auth:{}/?remarks={}&protoparam={}",
        STANDARD.encode("secret"),
        STANDARD.encode("SSR"),
        STANDARD.encode("32")
    ));
    let ssr = nodes(&format!("ssr://{payload}"));
    assert_eq!(ssr[0]["protocol-param"], "32");
}

#[test]
fn yaml_retains_new_protocol_fields_and_exact_integer_credentials() {
    let result = nodes(
        "proxies:\n - {name: first, type: new-protocol, server: example.test, port: '443', password: 000123, username: 9007199254740993, extra: {nested: [false, 9]}}\n - {name: ts, type: tailscale, auth-key: 00123, accept-routes: 'false', custom: yes}\n - {name: direct, type: direct}\n",
    );
    assert_eq!(result[0]["password"], "000123");
    assert_eq!(result[0]["username"], "9007199254740993");
    assert_eq!(result[0]["extra"]["nested"], json!([false, 9]));
    assert_eq!(result[1]["auth-key"], "00123");
    assert_eq!(result[1]["accept-routes"], false);
    assert!(result[2].get("server").is_none());
    let result = nodes(
        "{name: hex, type: http, server: example.test, port: 443, password: -0xff, headers: {'123': token}, note: 'password: 00123'}",
    );
    assert_eq!(result[0]["password"], "-255");
    assert_eq!(result[0]["note"], "password: 00123");
}

#[test]
fn yaml_aliases_merge_without_expansion_bombs() {
    let result = nodes(
        "defaults: &defaults {server: example.test, port: 443, password: 000123}\nproxies: [{<<: *defaults, name: merged, type: trojan}]",
    );
    assert_eq!(result[0]["password"], "000123");
    let mut bomb = String::from("a0: &a0 [x,x,x,x,x,x,x,x,x,x]\n");
    for i in 1..7 {
        bomb.push_str(&format!(
            "a{i}: &a{i} [{}]\n",
            vec![format!("*a{}", i - 1); 10].join(",")
        ));
    }
    bomb.push_str("proxies: [{name: evil, type: reject, extra: *a6}]");
    let result = parse(&bomb).unwrap();
    assert_eq!(result["nodes"], json!([]));
    assert!(
        result["errors"][0]
            .as_str()
            .unwrap()
            .starts_with("订阅解析受限:")
    );
    let cycle = parse("proxies: [{name: cycle, type: reject, extra: &a [*a]}]").unwrap();
    assert_eq!(cycle["nodes"], json!([]));
    assert_eq!(cycle["totalFailed"], 1);
}

#[test]
fn endpoint_variants_and_atomic_persistence_validation() {
    let key = STANDARD.encode([0u8; 32]);
    let accepted = json!([
        {"name":"tailscale","type":" Tailscale ","control-url":"https://control.example","udp":"yes","custom":{"keep":1}},
        {"name":"wg","type":"wireguard","private-key":key,"peers":[{"server":"example.test","port":443,"public-key":key,"reserved":"U4An"}],"keepalive":25,"persistent-keepalive":0},
        {"name":"range","type":"mieru","server":"example.test","port-range":"4000-4010"},
        {"name":"relay","type":"relay","proxies":["tailscale"]}
    ]);
    let admitted = validate_nodes(&accepted).unwrap();
    assert_eq!(admitted[0]["type"], "tailscale");
    assert_eq!(admitted[0]["custom"], json!({"keep":1}));
    assert_eq!(admitted[1]["persistent-keepalive"], 0);
    assert_eq!(admitted[1]["peers"][0]["reserved"], json!([83, 128, 39]));
    for bad in [
        json!({"name":"bad","type":"http","server":"example.test","port":"443"}),
        json!({"name":"bad","type":"http","server":"example.test","port":443,"password":9007199254740992u64}),
        json!({"name":"bad","type":"tailscale","control-url":"file:///tmp/key"}),
        json!({"name":"bad","type":"tailscale","accept-routes":"maybe"}),
    ] {
        let error = validate_nodes(&json!([{"name":"good","type":"reject"},bad])).unwrap_err();
        assert!(error.to_string().contains("#2"), "{error}");
    }
    assert!(
        validate_nodes(&json!(vec![
            json!({"name":"reject","type":"reject"});
            10_001
        ]))
        .is_err()
    );
}

#[test]
fn parses_platforms_referenced_wireguard_and_netch() {
    let result = nodes(
        "[General]\nloglevel = notify\n[WireGuard office]\nprivate-key = private\nself-ip = 10.0.0.2/32\nkeepalive = 25\npeer = (public-key = public, endpoint = \"wg.example:51820\", client-id = \"1/2/3\")\n[Proxy]\nWG = wireguard, section-name=office\nSS = ss, ss.example, 8388, encrypt-method=aes-128-gcm, password=secret\nLocal = direct\n[Rule]\nFINAL,Local\n",
    );
    assert_eq!(result.len(), 2);
    let wg = result.iter().find(|v| v["type"] == "wireguard").unwrap();
    assert_eq!(wg["persistent-keepalive"], 25);
    assert_eq!(wg["reserved"], json!([1, 2, 3]));
    let qx = nodes(
        "vmess=qx.example:443, password=11111111-1111-4111-8111-111111111111, method=none, obfs=wss, obfs-host=cdn.example, obfs-uri=/ws, tls-host=sni.example, aead=false, tag=QX",
    );
    assert_eq!(qx[0]["alterId"], 1);
    assert_eq!(qx[0]["tls"], true);
    assert_eq!(qx[0]["servername"], "sni.example");
    let netch = nodes(
        r#"{"Server":[{"Type":"VMess","Hostname":"example.test","Port":"443","UserID":"uuid","TransferProtocol":"ws","Path":"/ws","Host":"cdn.example","TLSSecure":true}]}"#,
    );
    assert_eq!(netch[0]["ws-opts"]["headers"]["Host"], "cdn.example");
}

#[test]
fn port_hopping_tuic_fast_open_and_safe_anytls() {
    let hy = nodes(
        "hy2://secret@hy.example:8443,9443-9449?obfs=gecko&obfs-password=padding&obfs-min-packet-size=100&ech=QUJDRA%3D%3D#HY",
    );
    assert_eq!(hy[0]["port"], 8443);
    assert_eq!(hy[0]["ports"], "8443,9443-9449");
    assert_eq!(hy[0]["ech-opts"]["config"], "QUJDRA==");
    let tuic = nodes("tuic://token@tuic.example?fast-open=false&tfo=true&congestion-control=bbr");
    assert_eq!(tuic[0]["fast-open"], false);
    assert_eq!(tuic[0]["tfo"], true);
    let any =
        nodes("anytls://secret@any.example?idle-session-timeout=120&udp=false&ech=ech.example");
    assert_eq!(any[0]["idle-session-timeout"], 120);
    assert_eq!(any[0]["udp"], false);
    for bad in [
        "anytls://secret@any.example?security=reality",
        "anytls://secret@any.example?type=ws",
        "hy2://secret@hy.example?obfs=salamander",
        "hy2://secret@hy.example?obfs=gecko&obfs-password=padding&obfs-min-packet-size=12bad",
    ] {
        assert_eq!(parse(bad).unwrap()["totalFailed"], 1, "{bad}");
    }
}

#[test]
fn all_supported_link_protocols_are_native() {
    for link in [
        "trojan://secret@node.example",
        "vless://uuid@node.example",
        "anytls://secret@node.example",
        "hysteria://node.example?auth=secret",
        "hysteria2://secret@node.example",
        "tuic://token@node.example",
        "wg://private@node.example?public-key=public",
        "snell://secret@node.example",
        "socks5://user:pass@node.example",
        "socks4://node.example",
        "http://user:pass@node.example",
        "https://user:pass@node.example",
        "ssh://user:pass@node.example",
        "tg://socks?server=node.example&port=1080&user=user&pass=pass",
    ] {
        assert_eq!(nodes(link).len(), 1, "{link}");
    }
    let config = json!({"v":"2","ps":"VMess","add":"node.example","port":"443","id":"uuid","aid":"0","net":"ws","tls":"tls","host":"cdn.example","path":"/ws"});
    assert_eq!(
        nodes(&format!("vmess://{}", STANDARD.encode(config.to_string())))[0]["type"],
        "vmess"
    );
}

#[test]
fn limits_drop_partial_nodes_and_bound_errors() {
    let valid = "trojan://secret@node.example";
    let result = parse(&format!(
        "{valid}\n{}",
        vec!["ftp://unsupported"; 101].join("\n")
    ))
    .unwrap();
    assert_eq!(result["nodes"], json!([]));
    assert!(
        result["errors"][0]
            .as_str()
            .unwrap()
            .starts_with("订阅解析受限:")
    );
    let count = parse(&vec![valid; 10_001].join("|")).unwrap();
    assert_eq!(count["nodes"], json!([]));
    let long = parse(&format!("ftp://{}", "x".repeat(2000))).unwrap();
    assert!(long["errors"][0].as_str().unwrap().chars().count() <= 512);
    assert_eq!(
        parse(&"# comment\r".repeat(200_001)).unwrap()["nodes"],
        json!([])
    );
}

#[test]
fn xhttp_preserves_ranges_and_rejects_unrepresentable_settings() {
    let extra = json!({"noGRPCHeader":false,"scMaxEachPostBytes":"100000-200000","xmux":{"maxConnections":"2-4"}});
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("extra", &extra.to_string())
        .finish();
    let result = nodes(&format!("vless://uuid@node.example?type=xhttp&{query}"));
    assert_eq!(
        result[0]["xhttp-opts"]["sc-max-each-post-bytes"],
        "100000-200000"
    );
    assert_eq!(
        result[0]["xhttp-opts"]["reuse-settings"]["max-connections"],
        "2-4"
    );
    assert_eq!(result[0]["xhttp-opts"]["no-grpc-header"], false);
    let bad =
        parse("vless://uuid@node.example?type=xhttp&sc-max-each-post-bytes=2048-1024").unwrap();
    assert_eq!(bad["totalFailed"], 1);
}
