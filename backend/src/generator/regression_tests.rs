use super::*;
fn node(name: &str) -> Value {
    json!({"name":name,"type":"ss","server":"example.com","port":443,"cipher":"aes-128-gcm","password":"secret"})
}
fn group<'a>(value: &'a Value, name: &str) -> &'a Value {
    array(value, "proxy-groups")
        .iter()
        .find(|g| text(g, "name") == name)
        .unwrap()
}

#[test]
fn information_nodes_only_appear_in_primary_selection() {
    let value = generate_config(
        &[node("Usable"), node("剩余流量: 20GB")],
        &json!({"template":"minimal"}),
    )
    .unwrap();
    assert!(
        strings(&group(&value, "🚀 节点选择")["proxies"]).contains(&"剩余流量: 20GB".to_owned())
    );
    assert_eq!(group(&value, "⚡ 自动选择")["proxies"], json!(["Usable"]));
    assert!(
        !strings(&group(&value, "🌍 非中国")["proxies"]).contains(&"剩余流量: 20GB".to_owned())
    );
}

#[test]
fn chain_uses_enabled_renamed_groups_and_first_target_owner() {
    let value=generate_config(&[node("Relay"),node("Target")],&json!({
        "enabledGroups":["auto","final"],"proxyGroupNameOverrides":{"auto":"Custom Auto"},
        "dialerProxyGroups":[
            {"id":"one","name":"Chain One","type":"url-test","testUrl":"https://example.com/204","relayNodes":["⚡ Custom Auto","Missing","DIRECT","Relay","Relay"],"targetNodes":["Target"]},
            {"id":"two","name":"Chain Two","type":"select","relayNodes":["DIRECT"],"targetNodes":["Target"]}
        ]
    })).unwrap();
    assert_eq!(value["proxies"][1]["dialer-proxy"], "Chain One");
    assert_eq!(
        group(&value, "Chain One")["proxies"],
        json!(["⚡ Custom Auto", "DIRECT", "Relay"])
    );
    assert_eq!(group(&value, "Chain One")["lazy"], true);
    assert_eq!(group(&value, "Chain One")["url"], "https://example.com/204");
}

#[test]
fn source_provider_mode_merges_base_and_limits_use_to_normal_groups() {
    let value=generate_config(&[],&json!({
        "dnsYaml":"proxy-providers:\n  local:\n    type: file\n    path: ./local.yaml",
        "sources":[{"id":"airport/one","type":"url","useProxyProviders":true,"content":"https://example.com/sub&token=secret"}],
        "customProxyGroups":[{"id":"filtered","name":"Filtered","emoji":"","groupType":"url-test","memberSource":"filtered-nodes"}]
    })).unwrap();
    assert_eq!(
        value["proxy-providers"]["url_airport_one"]["url"],
        "https://example.com/sub?token=secret"
    );
    assert_eq!(value["proxy-providers"]["local"]["type"], "file");
    assert_eq!(
        group(&value, "⚡ 自动选择")["use"],
        json!(["url_airport_one"])
    );
    assert!(group(&value, "Filtered").get("use").is_none());
}

#[test]
fn advanced_filters_keep_explicit_extras_and_stable_member_order() {
    let mut hk = node("香港 A");
    hk["_sourceIds"] = json!(["airport-a"]);
    let mut us = node("美国 B");
    us["_sourceIds"] = json!(["airport-b"]);
    let value=generate_config(&[hk,us],&json!({
        "enabledGroups":["select","auto"],
        "proxyGroupAdvanced":{"auto":{"sourceIds":["airport-a"],"regions":["hk"],"extraMembers":[{"kind":"node","name":"美国 B"}],"memberOrder":[{"kind":"node","name":"美国 B"}]}}
    })).unwrap();
    assert_eq!(
        group(&value, "⚡ 自动选择")["proxies"],
        json!(["美国 B", "香港 A"])
    );
    assert!(value["proxies"][0].get("_sourceIds").is_none());
}

#[test]
fn filters_use_origin_names_and_reject_unsafe_patterns_even_when_disabled() {
    let mut n = node("Renamed");
    n["_originName"] = json!("香港 A");
    let value = generate_config(
        &[n, node("USA")],
        &json!({"nodeNameFilter":{"enabled":true,"includeRegexes":["香港"],"excludeRegexes":[]}}),
    )
    .unwrap();
    assert_eq!(array(&value, "proxies").len(), 1);
    assert_eq!(value["proxies"][0]["name"], "Renamed");
    assert!(
        generate_config(
            &[],
            &json!({"nodeNameFilter":{"enabled":false,"excludeRegexes":["(a+)+$"]}})
        )
        .is_err()
    );
}

#[test]
fn shared_eager_checks_reuse_one_inline_provider_but_leave_lazy_chain_alone() {
    let value=generate_config(&[node("A")],&json!({"enabledGroups":["auto"],"sharedHealthChecks":true,
        "customProxyGroups":[{"id":"probe","name":"Probe","emoji":"","groupType":"url-test"}],
        "dialerProxyGroups":[{"id":"chain","name":"Lazy","type":"url-test","relayNodes":["A"],"targetNodes":[]}]
    })).unwrap();
    assert_eq!(
        group(&value, "Probe")["use"],
        group(&value, "⚡ 自动选择")["use"]
    );
    assert_eq!(group(&value, "Lazy")["proxies"], json!(["A"]));
    assert_eq!(
        value["proxy-providers"]["subboost_shared_1"]["payload"][0]["name"],
        "A"
    );
}

#[test]
fn full_rule_order_keeps_new_editable_rules_at_the_canonical_anchor() {
    let value = generate_config(
        &[],
        &json!({"enabledGroups":["cn","global","final"],
            "customRules":[{"id":"domain","type":"DOMAIN","value":"example.com","target":"DIRECT"}],
            "ruleOrder":["module:global:geolocation-!cn"]
        }),
    )
    .unwrap();
    let rules = strings(&value["rules"]);
    let custom = rules
        .iter()
        .position(|r| r == "DOMAIN,example.com,DIRECT")
        .unwrap();
    let global = rules
        .iter()
        .position(|r| r.starts_with("RULE-SET,geolocation-!cn,"))
        .unwrap();
    assert!(custom < global);
    assert!(rules.last().unwrap().starts_with("MATCH,"));
}
