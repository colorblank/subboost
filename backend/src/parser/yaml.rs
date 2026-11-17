use super::*;
use serde::de::{self, DeserializeSeed, EnumAccess, MapAccess, SeqAccess, VariantAccess, Visitor};
use std::{cell::Cell, fmt};

struct Budget {
    policy: &'static crate::parser_policy::ParserPolicy,
    nodes: Cell<usize>,
    text: Cell<usize>,
    merges: Cell<usize>,
}
struct Seed<'a> {
    budget: &'a Budget,
    depth: usize,
}
impl Budget {
    fn charge<E: de::Error>(&self, depth: usize, bytes: usize) -> std::result::Result<(), E> {
        self.nodes.set(self.nodes.get().saturating_add(1));
        self.text.set(self.text.get().saturating_add(bytes));
        if depth > self.policy.yaml_maximum_depth
            || self.nodes.get() > self.policy.yaml_maximum_nodes
            || self.text.get() > self.policy.yaml_maximum_text_bytes
        {
            return Err(E::custom("订阅解析受限: YAML 超过展开预算"));
        }
        Ok(())
    }
}
impl<'de> DeserializeSeed<'de> for Seed<'_> {
    type Value = Value;
    fn deserialize<D: de::Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> std::result::Result<Value, D::Error> {
        self.budget.charge::<D::Error>(self.depth, 0)?;
        deserializer.deserialize_any(self)
    }
}
impl<'de> Visitor<'de> for Seed<'_> {
    type Value = Value;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a bounded YAML value")
    }
    fn visit_unit<E: de::Error>(self) -> std::result::Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_none<E: de::Error>(self) -> std::result::Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_bool<E: de::Error>(self, v: bool) -> std::result::Result<Value, E> {
        Ok(json!(v))
    }
    fn visit_i64<E: de::Error>(self, v: i64) -> std::result::Result<Value, E> {
        Ok(json!(v))
    }
    fn visit_u64<E: de::Error>(self, v: u64) -> std::result::Result<Value, E> {
        Ok(json!(v))
    }
    fn visit_f64<E: de::Error>(self, v: f64) -> std::result::Result<Value, E> {
        serde_json::Number::from_f64(v)
            .map(Value::Number)
            .ok_or_else(|| E::custom("YAML 不支持非有限数字"))
    }
    fn visit_str<E: de::Error>(self, v: &str) -> std::result::Result<Value, E> {
        self.budget.charge::<E>(self.depth, v.len())?;
        Ok(json!(v))
    }
    fn visit_string<E: de::Error>(self, v: String) -> std::result::Result<Value, E> {
        self.budget.charge::<E>(self.depth, v.len())?;
        Ok(Value::String(v))
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut access: A) -> std::result::Result<Value, A::Error> {
        let mut out = Vec::new();
        while let Some(value) = access.next_element_seed(Seed {
            budget: self.budget,
            depth: self.depth + 1,
        })? {
            out.push(value);
        }
        Ok(Value::Array(out))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> std::result::Result<Value, A::Error> {
        let mut out = Map::new();
        let mut merge = None;
        while let Some(key) = access.next_key_seed(Seed {
            budget: self.budget,
            depth: self.depth + 1,
        })? {
            let key = match key {
                Value::String(s) => s,
                Value::Number(n) => n.to_string(),
                Value::Bool(v) => v.to_string(),
                Value::Null => "null".into(),
                _ => return Err(de::Error::custom("YAML 映射键必须是标量")),
            };
            let value = access.next_value_seed(Seed {
                budget: self.budget,
                depth: self.depth + 1,
            })?;
            if key == "<<" {
                if merge.replace(value).is_some() {
                    return Err(de::Error::custom("重复的 YAML 合并键"));
                }
            } else if out.insert(key, value).is_some() {
                return Err(de::Error::custom("重复的 YAML 映射键"));
            }
        }
        if let Some(value) = merge {
            let maps = match value {
                Value::Object(map) => vec![map],
                Value::Array(items) => items
                    .into_iter()
                    .map(|item| {
                        item.as_object()
                            .cloned()
                            .ok_or_else(|| de::Error::custom("YAML 合并需要对象"))
                    })
                    .collect::<std::result::Result<Vec<_>, A::Error>>()?,
                _ => return Err(de::Error::custom("YAML 合并需要对象")),
            };
            let mut merged = Map::new();
            for map in maps {
                self.budget
                    .merges
                    .set(self.budget.merges.get().saturating_add(map.len()));
                if self.budget.merges.get() > self.budget.policy.yaml_maximum_merge_entries {
                    return Err(de::Error::custom("订阅解析受限: YAML 合并超过预算"));
                }
                for (key, value) in map {
                    merged.entry(key).or_insert(value);
                }
            }
            merged.extend(out);
            out = merged;
        }
        Ok(Value::Object(out))
    }
    fn visit_enum<A: EnumAccess<'de>>(self, access: A) -> std::result::Result<Value, A::Error> {
        let (_tag, value) = access.variant::<String>()?;
        value.newtype_variant_seed(self)
    }
}

fn preserve_integer_scalars(input: &str) -> Result<String> {
    // Protect only mapping values, not numbers inside comments, strings or block scalars.
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut copied = 0;
    let mut i = 0;
    let mut quote = None;
    let mut quote_start = 0;
    let mut last_quote = None;
    let mut block_indent = None;
    while i < bytes.len() {
        if i == 0 || bytes[i - 1] == b'\n' {
            let end = input[i..].find('\n').map_or(bytes.len(), |n| i + n);
            let line = &input[i..end];
            let indent = line.bytes().take_while(|b| *b == b' ').count();
            if let Some(parent) = block_indent {
                if line.trim().is_empty() || indent > parent {
                    i = end + usize::from(end < bytes.len());
                    continue;
                }
                block_indent = None;
            }
            if line.trim_end().ends_with(['|', '>'])
                || ["|-", "|+", ">-", ">+"]
                    .iter()
                    .any(|suffix| line.trim_end().ends_with(suffix))
            {
                block_indent = Some(indent);
            }
        }
        if let Some(q) = quote {
            if q == b'"' && bytes[i] == b'\\' {
                i = (i + 2).min(bytes.len());
                continue;
            }
            if bytes[i] == q {
                if q == b'\'' && bytes.get(i + 1) == Some(&q) {
                    i += 2;
                    continue;
                }
                last_quote = Some((quote_start, i));
                quote = None;
            }
            i += 1;
            continue;
        }
        if bytes[i] == b'#' && (i == 0 || bytes[i - 1].is_ascii_whitespace()) {
            i = input[i..].find('\n').map_or(bytes.len(), |n| i + n);
            continue;
        }
        if [b'"', b'\''].contains(&bytes[i])
            && (i == 0 || bytes[i - 1].is_ascii_whitespace() || b"{[,?:".contains(&bytes[i - 1]))
        {
            quote = Some(bytes[i]);
            quote_start = i;
            i += 1;
            continue;
        }
        if bytes[i] != b':' {
            i += 1;
            continue;
        }
        let mut end = i;
        while end > 0 && matches!(bytes[end - 1], b' ' | b'\t') {
            end -= 1;
        }
        let (start, key) = if end > 0 && [b'"', b'\''].contains(&bytes[end - 1]) {
            if let Some((start, close)) = last_quote.filter(|(_, close)| *close + 1 == end) {
                (start, &input[start + 1..close])
            } else {
                i += 1;
                continue;
            }
        } else {
            let mut start = end;
            while start > 0
                && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'-')
            {
                start -= 1;
            }
            (start, &input[start..end])
        };
        let boundary = input[..start].trim_end_matches([' ', '\t']);
        if !(boundary.is_empty() || boundary.ends_with(['\n', '\r', '{', ',', '?', '-']))
            || !(STRING_FIELDS.contains(&key)
                || [
                    "short-id",
                    "hostname",
                    "auth-key",
                    "control-url",
                    "state-dir",
                    "exit-node",
                ]
                .contains(&key))
        {
            i += 1;
            continue;
        }
        let mut value = i + 1;
        while bytes.get(value).is_some_and(|b| matches!(*b, b' ' | b'\t')) {
            value += 1;
        }
        if bytes.get(value) == Some(&b'&') {
            while bytes.get(value).is_some_and(|b| !b.is_ascii_whitespace()) {
                value += 1;
            }
            while bytes.get(value).is_some_and(|b| matches!(*b, b' ' | b'\t')) {
                value += 1;
            }
        }
        let mut stop = value;
        while bytes
            .get(stop)
            .is_some_and(|b| b.is_ascii_alphanumeric() || b"+-".contains(b))
        {
            stop += 1;
        }
        let raw = &input[value..stop];
        let unsigned = raw.strip_prefix(['+', '-']).unwrap_or(raw);
        if unsigned.is_empty()
            || !bytes
                .get(stop)
                .is_none_or(|b| b.is_ascii_whitespace() || b",}]#".contains(b))
        {
            i += 1;
            continue;
        }
        let radix = if unsigned.starts_with("0x") || unsigned.starts_with("0X") {
            16
        } else if unsigned.starts_with("0o") || unsigned.starts_with("0O") {
            8
        } else if unsigned.starts_with("0b") || unsigned.starts_with("0B") {
            2
        } else {
            10
        };
        let digits = if radix == 10 {
            unsigned
        } else {
            &unsigned[2..]
        };
        if digits.is_empty() || !digits.chars().all(|c| c.is_digit(radix)) {
            i += 1;
            continue;
        }
        if radix != 10 && digits.len() > 1024 {
            return Err(limited("YAML 整数标量超过计算预算"));
        }
        let converted = if radix == 10 {
            raw.to_owned()
        } else {
            let mut decimal = vec![0u8];
            for c in digits.chars() {
                let mut carry = c.to_digit(radix).unwrap_or(0);
                for d in &mut decimal {
                    let v = u32::from(*d) * radix + carry;
                    *d = (v % 10) as u8;
                    carry = v / 10;
                }
                while carry > 0 {
                    decimal.push((carry % 10) as u8);
                    carry /= 10;
                }
            }
            format!(
                "{}{}",
                if raw.starts_with('-') { "-" } else { "" },
                decimal
                    .iter()
                    .rev()
                    .map(|v| char::from(b'0' + *v))
                    .collect::<String>()
            )
        };
        out.push_str(&input[copied..value]);
        out.push('"');
        out.push_str(&converted);
        out.push('"');
        copied = stop;
        i = stop;
        if out.len() > crate::parser_policy::policy()?.maximum_output_bytes {
            return Err(limited("YAML 标量规范化后的内容超过大小上限"));
        }
    }
    out.push_str(&input[copied..]);
    if out.len() > crate::parser_policy::policy()?.maximum_output_bytes {
        return Err(limited("YAML 标量规范化后的内容超过大小上限"));
    }
    Ok(out)
}

pub(super) fn load(content: &str) -> Result<Value> {
    let normalized = preserve_integer_scalars(content)?;
    let budget = Budget {
        policy: crate::parser_policy::policy()?,
        nodes: Cell::new(0),
        text: Cell::new(0),
        merges: Cell::new(0),
    };
    let mut docs = serde_yaml::Deserializer::from_str(&normalized);
    let value = match docs.next() {
        Some(doc) => Seed {
            budget: &budget,
            depth: 0,
        }
        .deserialize(doc)
        .map_err(|e| {
            let message = e.to_string();
            if message.contains("订阅解析受限:")
                || message.contains("recursion")
                || message.contains("repetition")
            {
                limited("YAML 超过展开预算或存在循环别名")
            } else {
                anyhow!("Clash YAML 解析失败: {message}")
            }
        })?,
        None => Value::Null,
    };
    if docs.next().is_some() {
        bail!("Clash YAML 解析失败: expected a single document");
    }
    Ok(value)
}

pub(super) fn parse(content: &str, parsed: &mut Parsed) -> Result<()> {
    let value = match load(content) {
        Ok(value) => value,
        Err(error) if !error.to_string().starts_with("订阅解析受限:") => {
            let repaired = super::legacy_yaml::normalize(content);
            if repaired.as_ref() == content {
                return Err(error);
            }
            load(&repaired)?
        }
        Err(error) => return Err(error),
    };
    let providers = value
        .get("proxy-providers")
        .and_then(Value::as_object)
        .is_some_and(|m| !m.is_empty());
    let nodes = if let Some(proxies) = value.get("proxies") {
        proxies
            .as_array()
            .ok_or_else(|| anyhow!("Clash YAML proxies 必须是数组"))?
            .clone()
    } else if let Some(items) = value.as_array() {
        items.clone()
    } else if value.is_object() {
        vec![value]
    } else {
        parsed.error(if value.is_null() {
            "空的配置文件"
        } else {
            "无法识别为 Clash YAML（缺少 proxies 或节点字段）"
        })?;
        return Ok(());
    };
    check_items(nodes.len())?;
    for node in nodes {
        let name = text(&node, "name").chars().take(100).collect::<String>();
        if let Err(error) = parsed.push(node) {
            if error.to_string().starts_with("订阅解析受限:") {
                return Err(error);
            }
            parsed.error(format!("节点 \"{name}\" 解析失败: {error}"))?;
        }
    }
    if providers {
        parsed
            .error("检测到 proxy-providers 配置，请直接导入节点内容；解析不会自动拉取远程提供者")?;
    }
    Ok(())
}
