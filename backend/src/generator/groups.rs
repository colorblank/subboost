use crate::generator::{
    array, group_type, info_node, module_name, modules, settings, strategy, strings, test_options,
    text, unique,
};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::sync::LazyLock;

const ORDER: &[&str] = &[
    "select",
    "auto",
    "ad",
    "ai",
    "gemini",
    "youtube",
    "google",
    "microsoft",
    "apple",
    "telegram",
    "twitter",
    "meta",
    "discord",
    "social-other",
    "netflix",
    "disney",
    "streaming-west",
    "streaming-asia",
    "steam",
    "gaming-pc",
    "gaming-console",
    "github",
    "cloud",
    "dev-tools",
    "storage",
    "payment",
    "crypto",
    "google-scholar",
    "education",
    "news",
    "shopping",
    "adult",
    "private",
    "cn",
    "global",
    "final",
];

pub(crate) fn regions() -> &'static [Value] {
    static REGIONS: LazyLock<Vec<Value>> = LazyLock::new(|| {
        let text = crate::config::load_file_or_embedded(
            "SUBBOOST_MIHOMO_REGIONS",
            "config/mihomo/regions.json",
            include_str!("../../../config/mihomo/regions.json"),
        );
        serde_json::from_str(&text).expect("embedded region data")
    });
    &REGIONS
}

pub(crate) fn is_valid_region(id: &str) -> bool {
    regions().iter().any(|r| text(r, "id") == id)
}

fn region_matches(name: &str, wanted: &[String]) -> bool {
    if wanted.is_empty() {
        return true;
    }
    let name = name.to_lowercase();
    let matching: Vec<&str> = regions()
        .iter()
        .filter(|r| {
            array(r, "keywords")
                .iter()
                .filter_map(Value::as_str)
                .any(|key| name.contains(&key.to_lowercase()))
        })
        .map(|r| text(r, "id"))
        .collect();
    wanted
        .iter()
        .any(|w| matching.contains(&w.as_str()) || (w == "other" && matching.is_empty()))
}

fn resolve_ref(reference: &Value, config: &Value, nodes: &[Value]) -> Option<String> {
    match text(reference, "kind") {
        "direct" => Some("DIRECT".to_owned()),
        "reject" => Some("REJECT".to_owned()),
        "node" => nodes
            .iter()
            .find(|n| text(n, "name") == text(reference, "name"))
            .map(|n| text(n, "name").to_owned()),
        "module" => modules()
            .iter()
            .find(|m| text(m, "id") == text(reference, "id"))
            .map(|m| module_name(m, config)),
        "custom" => array(config, "customProxyGroups")
            .iter()
            .find(|g| g["enabled"] != false && text(g, "id") == text(reference, "id"))
            .map(|g| text(g, "name").to_owned()),
        _ => None,
    }
}

fn members(
    defaults: Vec<String>,
    advanced: &Value,
    name: &str,
    nodes: &[Value],
    config: &Value,
) -> Vec<String> {
    let extras = unique(
        array(advanced, "extraMembers")
            .iter()
            .filter_map(|r| resolve_ref(r, config, nodes)),
    );
    let excluded: HashSet<String> = array(advanced, "excludedMembers")
        .iter()
        .filter_map(|r| resolve_ref(r, config, nodes))
        .collect();
    let sources = strings(&advanced["sourceIds"]);
    let regions = strings(&advanced["regions"]);
    let compile = |key| {
        if text(advanced, key).is_empty() {
            None
        } else {
            settings::FilterRegex::compile(text(advanced, key)).ok()
        }
    };
    let include = compile("includeRegex");
    let exclude = compile("excludeRegex");
    let mut result = unique(defaults.into_iter().chain(extras.iter().cloned()))
        .into_iter()
        .filter(|n| {
            if n == name || excluded.contains(n) {
                return false;
            }
            if extras.contains(n) {
                return true;
            }
            let Some(node) = nodes.iter().find(|node| text(node, "name") == n) else {
                return true;
            };
            (sources.is_empty()
                || strings(&node["_sourceIds"])
                    .iter()
                    .any(|id| sources.contains(id)))
                && region_matches(n, &regions)
                && include.as_ref().and_then(|r| r.matches(n).ok()) != Some(false)
                && exclude.as_ref().and_then(|r| r.matches(n).ok()) != Some(true)
        })
        .collect::<Vec<_>>();
    let ordered: Vec<String> = array(advanced, "memberOrder")
        .iter()
        .filter_map(|r| resolve_ref(r, config, nodes))
        .filter(|n| result.contains(n))
        .collect();
    result = unique(ordered.into_iter().chain(result));
    result
}

pub(super) fn typed(
    name: &str,
    typ: &str,
    proxies: Vec<String>,
    providers: &[String],
    url: &str,
    interval: u64,
    selection: &str,
    lazy: bool,
) -> Value {
    let typ = if ["url-test", "fallback", "load-balance"].contains(&typ) {
        typ
    } else {
        "select"
    };
    let mut group = json!({"name":name,"type":typ,"proxies":proxies});
    if !providers.is_empty() {
        group["use"] = json!(providers);
    }
    if typ != "select" {
        group["url"] = json!(url);
        group["interval"] = json!(interval);
        if typ == "url-test" {
            group["lazy"] = json!(lazy);
        }
        if typ == "load-balance" {
            group["strategy"] = json!(selection);
        }
    }
    group
}

pub(super) fn generate(
    nodes: &[Value],
    config: &Value,
    providers: &[String],
    dialers: &[Value],
) -> Vec<Value> {
    let enabled = strings(&config["enabledGroups"]);
    let custom = array(config, "customProxyGroups")
        .iter()
        .filter(|g| g["enabled"] != false)
        .collect::<Vec<_>>();
    let node_names = nodes
        .iter()
        .map(|n| text(n, "name").to_owned())
        .collect::<Vec<_>>();
    let filtered = node_names
        .iter()
        .filter(|n| !info_node(n))
        .cloned()
        .collect::<Vec<_>>();
    let target = |id: &str| {
        modules()
            .iter()
            .find(|m| text(m, "id") == id && enabled.iter().any(|e| e == id))
            .map(|m| module_name(m, config))
            .unwrap_or_default()
    };
    let select = target("select");
    let auto = target("auto");
    let included_custom = custom
        .iter()
        .filter(|g| g["includeInGroupMembers"] == true)
        .map(|g| text(g, "name").to_owned())
        .collect::<Vec<_>>();
    let module_base = if !select.is_empty() {
        unique(
            [
                select.clone(),
                auto.clone(),
                "DIRECT".to_owned(),
                "REJECT".to_owned(),
            ]
            .into_iter()
            .chain(included_custom.iter().cloned())
            .chain(filtered.iter().cloned()),
        )
    } else {
        unique(
            [auto.clone()]
                .into_iter()
                .chain(included_custom.iter().cloned())
                .chain(filtered.iter().cloned())
                .chain(["DIRECT".to_owned(), "REJECT".to_owned()]),
        )
    };
    let (url, interval) = test_options(config);
    let build_custom = |group: &Value| {
        let name = text(group, "name");
        let typ = text(group, "groupType");
        let advanced = &group["advanced"];
        let defaults = match typ {
            "url-test" | "fallback" | "load-balance" => filtered.clone(),
            "reject-first" => unique(
                ["REJECT".to_owned(), "DIRECT".to_owned()]
                    .into_iter()
                    .chain(filtered.iter().cloned()),
            ),
            _ => unique(
                ["DIRECT".to_owned(), "REJECT".to_owned()]
                    .into_iter()
                    .chain(filtered.iter().cloned()),
            ),
        };
        let group_url = if typ == "url-test" && settings::valid_test_url(text(advanced, "testUrl"))
        {
            text(advanced, "testUrl")
        } else {
            url
        };
        typed(
            name,
            typ,
            members(defaults, advanced, name, nodes, config),
            if group["memberSource"] == "filtered-nodes" {
                &[]
            } else {
                providers
            },
            group_url,
            interval,
            strategy(&group["strategy"]),
            false,
        )
    };
    let mut groups = custom
        .iter()
        .filter(|g| g["memberSource"] == "filtered-nodes")
        .map(|g| build_custom(g))
        .collect::<Vec<_>>();
    for id in ORDER.iter().copied().chain(
        modules()
            .iter()
            .map(|m| text(m, "id"))
            .filter(|id| !ORDER.contains(id)),
    ) {
        if id == "ai" {
            groups.extend(
                custom
                    .iter()
                    .filter(|g| g["memberSource"] != "filtered-nodes")
                    .map(|g| build_custom(g)),
            );
        }
        if !enabled.iter().any(|e| e == id) {
            continue;
        }
        let Some(module) = modules().iter().find(|m| text(m, "id") == id) else {
            continue;
        };
        let name = module_name(module, config);
        let advanced = &config["proxyGroupAdvanced"][id];
        let override_type = text(advanced, "groupType");
        let typ = if group_type(override_type) {
            override_type
        } else {
            text(module, "groupType")
        };
        let defaults = match typ {
            "url-test" | "fallback" | "load-balance" => filtered.clone(),
            "reject-first" => unique(["REJECT".to_owned(), "DIRECT".to_owned(), select.clone()]),
            "direct-first" => unique(
                ["DIRECT".to_owned(), "REJECT".to_owned()]
                    .into_iter()
                    .chain(included_custom.iter().cloned())
                    .chain([select.clone(), auto.clone()])
                    .chain(filtered.iter().cloned()),
            ),
            _ if id == "select" => unique(
                [auto.clone(), "DIRECT".to_owned(), "REJECT".to_owned()]
                    .into_iter()
                    .chain(node_names.iter().cloned()),
            ),
            _ => module_base.clone(),
        };
        let group_url = if typ == "url-test" && settings::valid_test_url(text(advanced, "testUrl"))
        {
            text(advanced, "testUrl")
        } else {
            url
        };
        groups.push(typed(
            &name,
            typ,
            members(defaults, advanced, &name, nodes, config),
            providers,
            group_url,
            interval,
            strategy(&advanced["strategy"]),
            false,
        ));
    }
    let mut chain_groups = Vec::new();
    for dialer in dialers {
        let typ = text(dialer, "type");
        let mut names = strings(&dialer["relayNodes"]);
        if typ == "direct-first" {
            names.insert(0, "DIRECT".to_owned());
        }
        if typ == "reject-first" {
            names.insert(0, "REJECT".to_owned());
        }
        let names = unique(names);
        if names.is_empty() {
            continue;
        }
        let group_url = if typ == "url-test" && settings::valid_test_url(text(dialer, "testUrl")) {
            text(dialer, "testUrl")
        } else {
            url
        };
        chain_groups.push(typed(
            text(dialer, "name"),
            typ,
            names,
            providers,
            group_url,
            interval,
            strategy(&dialer["strategy"]),
            true,
        ));
    }
    let select_name = modules()
        .iter()
        .find(|m| text(m, "id") == "select")
        .map(|m| module_name(m, config));
    let auto_name = modules()
        .iter()
        .find(|m| text(m, "id") == "auto")
        .map(|m| module_name(m, config));
    let insert = groups
        .iter()
        .enumerate()
        .filter(|(_, g)| {
            let name = text(g, "name");
            select_name.as_deref() == Some(name) || auto_name.as_deref() == Some(name)
        })
        .map(|(i, _)| i + 1)
        .max()
        .unwrap_or(0);
    groups.splice(insert..insert, chain_groups);
    let key = |group: &Value| {
        let name = text(group, "name");
        if let Some(d) = dialers.iter().rev().find(|d| text(d, "name") == name) {
            return format!("dialer:{}", text(d, "id"));
        }
        if let Some(c) = custom.iter().rev().find(|c| text(c, "name") == name) {
            return format!("custom:{}", text(c, "id"));
        }
        if let Some(m) = modules()
            .iter()
            .rev()
            .find(|m| module_name(m, config) == name)
        {
            return format!("module:{}", text(m, "id"));
        }
        format!("name:{name}")
    };
    let order = strings(&config["proxyGroupOrder"]);
    if !order.is_empty() {
        let defaults = groups.iter().map(key).collect::<Vec<_>>();
        let keys = unique(order.into_iter().chain(defaults));
        groups = keys
            .iter()
            .filter_map(|k| groups.iter().rev().find(|g| key(g) == *k).cloned())
            .collect();
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::{is_valid_region, regions};

    #[test]
    fn validates_regions_from_config() {
        assert!(!regions().is_empty());
        assert!(is_valid_region("us"));
        assert!(is_valid_region("hk"));
        assert!(is_valid_region("jp"));
        assert!(is_valid_region("other"));
        assert!(!is_valid_region("invalid_nonexistent_region"));
    }
}
