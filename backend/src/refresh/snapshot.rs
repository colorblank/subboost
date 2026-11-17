use std::collections::{BTreeMap, HashMap, HashSet};

use serde_json::{Value, json};

#[derive(Debug)]
struct NodeFields {
    anchor: String,
    fields: BTreeMap<String, String>,
}

/// A comparison view, not a merged subscription: conflicting credentials never mix.
#[derive(Debug)]
pub(crate) struct SubscriptionSnapshot {
    nodes: Vec<NodeFields>,
    protocols: usize,
    fields: usize,
    errors: usize,
}

impl SubscriptionSnapshot {
    pub fn new(parsed: &Value) -> Self {
        let mut seen = HashSet::new();
        let mut nodes = Vec::new();
        let mut protocols = HashSet::new();
        for raw in parsed["nodes"].as_array().into_iter().flatten() {
            if crate::generator::info_node(raw["name"].as_str().unwrap_or_default()) {
                continue;
            }
            let Some(node) = crate::parser::sanitize_for_generation(raw) else {
                continue;
            };
            let mut fields = BTreeMap::new();
            Self::visit(&node, "", &mut fields);
            let identity = serde_json::to_string(&fields).expect("String map is serializable");
            if !seen.insert(identity) {
                continue;
            }
            protocols.insert(node["type"].as_str().unwrap_or("unknown").to_owned());
            let anchor = if node["type"] == "tailscale" {
                json!({"type":node["type"],"control-url":node["control-url"],"hostname":node["hostname"],"state-dir":node["state-dir"]})
            } else {
                let peers = node["peers"].as_array().map(|peers| {
                    peers
                        .iter()
                        .map(|peer| json!({"server":peer["server"],"port":peer["port"]}))
                        .collect::<Vec<_>>()
                });
                json!({"type":node["type"],"server":node["server"],"port":node["port"],"peers":peers,"port-range":node["port-range"]})
            };
            nodes.push(NodeFields {
                anchor: anchor.to_string(),
                fields,
            });
        }
        Self {
            fields: nodes.iter().map(|node| node.fields.len()).sum(),
            nodes,
            protocols: protocols.len(),
            errors: parsed["errors"].as_array().map_or(0, Vec::len),
        }
    }

    fn visit(value: &Value, path: &str, output: &mut BTreeMap<String, String>) {
        match value {
            Value::Null => {}
            Value::String(value) if value.is_empty() => {}
            Value::Object(fields) => {
                for (key, value) in fields {
                    if key.starts_with('_') || (path.is_empty() && key == "name") {
                        continue;
                    }
                    Self::visit(value, &format!("{path}/{key}"), output);
                }
            }
            Value::Array(values) => {
                for (index, value) in values.iter().enumerate() {
                    Self::visit(value, &format!("{path}/{index}"), output);
                }
            }
            _ => {
                output.insert(path.to_owned(), value.to_string());
            }
        }
    }

    pub fn is_usable(&self) -> bool {
        !self.nodes.is_empty()
    }

    fn covered_by(&self, other: &Self, maximum_comparisons: usize) -> bool {
        if self.nodes.len() > other.nodes.len() {
            return false;
        }
        let mut groups: HashMap<&str, Vec<&NodeFields>> = HashMap::new();
        for node in &other.nodes {
            groups.entry(&node.anchor).or_default().push(node);
        }
        for group in groups.values_mut() {
            group.sort_by_key(|node| node.fields.len());
        }
        let mut nodes: Vec<_> = self.nodes.iter().collect();
        nodes.sort_by_key(|node| std::cmp::Reverse(node.fields.len()));
        let mut comparisons = 0;
        for node in nodes {
            let Some(group) = groups.get_mut(node.anchor.as_str()) else {
                return false;
            };
            let index = group.iter().position(|other| {
                comparisons += 1;
                comparisons <= maximum_comparisons
                    && node
                        .fields
                        .iter()
                        .all(|(key, value)| other.fields.get(key) == Some(value))
            });
            let Some(index) = index.filter(|_| comparisons <= maximum_comparisons) else {
                return false;
            };
            group.remove(index);
        }
        true
    }

    pub fn prefers(&self, next: &Self, maximum_comparisons: usize) -> bool {
        match (
            self.covered_by(next, maximum_comparisons),
            next.covered_by(self, maximum_comparisons),
        ) {
            (true, false) => true,
            (true, true) => next.errors < self.errors,
            (false, true) => false,
            (false, false) => {
                (
                    next.nodes.len(),
                    next.protocols,
                    next.fields,
                    std::cmp::Reverse(next.errors),
                ) > (
                    self.nodes.len(),
                    self.protocols,
                    self.fields,
                    std::cmp::Reverse(self.errors),
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SubscriptionSnapshot;
    use serde_json::json;

    #[test]
    fn prefers_richer_snapshot_without_merging_credentials() {
        let current = json!({"nodes":[{"name":"old","type":"ss","server":"example.com","port":443,"cipher":"aes-128-gcm","password":"a"}],"errors":[]});
        let mut next = current.clone();
        next["nodes"][0]["udp"] = true.into();
        let left = SubscriptionSnapshot::new(&current);
        let right = SubscriptionSnapshot::new(&next);
        assert!(left.prefers(&right, 100));
        assert!(!right.prefers(&left, 100));
        next["nodes"][0]["password"] = "b".into();
        assert!(left.prefers(&SubscriptionSnapshot::new(&next), 100));
        assert_eq!(current["nodes"][0]["password"], "a");
    }
}
